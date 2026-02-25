"""
NudgeManager — Server-Authoritative Proactive Nudge Gate
=========================================================

The LLM never decides to send proactive messages.
Only server-side events on the allowlist can trigger nudges.
Rate limits enforced per-session per-trigger-type.

Triggers: error_event, upload_complete, processing_complete,
          missing_config, pii_detected, long_running_op, destructive_action

Analytics: nudge_shown, nudge_dismissed, nudge_acted, nudge_permanent_dismiss

CREATED: BQ-128 Phase 3 (2026-02-14)
SPEC: BQ-128 §Task 3.1
"""

import asyncio
import logging
import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------

@dataclass
class TriggerConfig:
    """Configuration for a single trigger type."""
    max_per_event: int = 1           # Max nudges per triggering event
    max_per_session: int = 0         # Max nudges per session (0=unlimited)
    max_per_dataset: int = 0         # Max nudges per dataset (0=unlimited)
    max_per_operation: int = 0       # Max nudges per operation (0=unlimited)
    cooldown_s: int = 0              # Cooldown in seconds after last nudge
    dismissable: bool = True         # Can user dismiss this nudge?
    required: bool = False           # Cannot be dismissed (destructive_action)
    message_template: str = ""       # Default message template


@dataclass
class NudgeMessage:
    """A nudge message to be sent to the client."""
    nudge_id: str
    trigger: str
    message: str
    dismissable: bool = True
    metadata: Dict[str, Any] = field(default_factory=dict)


@dataclass
class NudgeAnalytics:
    """Analytics event for a nudge interaction."""
    nudge_id: str
    trigger: str
    action: str  # nudge_shown, nudge_dismissed, nudge_acted, nudge_permanent_dismiss
    session_id: str
    user_id: str
    timestamp: float = field(default_factory=time.time)


# ---------------------------------------------------------------------------
# Nudge message templates
# ---------------------------------------------------------------------------

NUDGE_TEMPLATES: Dict[str, str] = {
    "error_event": "Something went wrong during processing. I can help troubleshoot — want me to take a look?",
    "upload_complete": "Your data has been uploaded successfully! Would you like me to summarize what's in this dataset?",
    "processing_complete": "Processing is complete. Your data is ready to explore — ask me anything about it.",
    "missing_config": "It looks like some configuration is missing. Want me to walk you through the setup?",
    "pii_detected": "I noticed potential personal information (PII) in this dataset. Want me to help you review and handle it?",
    "long_running_op": "This operation is taking a while. I'll keep an eye on it and let you know when it's done.",
    "destructive_action": "This action will permanently modify your data. Please confirm you want to proceed.",
    "first_search": "Nice — your first search worked! How's the experience so far?",
}

# Trigger-appropriate icons (for frontend reference)
NUDGE_ICONS: Dict[str, str] = {
    "error_event": "AlertTriangle",
    "upload_complete": "CheckCircle",
    "processing_complete": "CheckCircle",
    "missing_config": "Settings",
    "pii_detected": "Shield",
    "long_running_op": "Clock",
    "destructive_action": "AlertOctagon",
    "first_search": "MessageCircle",
}


# ---------------------------------------------------------------------------
# NudgeManager
# ---------------------------------------------------------------------------

class NudgeManager:
    """
    Server-authoritative proactive nudge gate.

    The LLM never decides to send proactive messages.
    Only server-side events on the allowlist can trigger nudges.
    Rate limits enforced per-session per-trigger-type.
    """

    TRIGGER_ALLOWLIST: Dict[str, TriggerConfig] = {
        "error_event": TriggerConfig(
            max_per_event=1, cooldown_s=0,
            message_template=NUDGE_TEMPLATES["error_event"],
        ),
        "upload_complete": TriggerConfig(
            max_per_event=1, cooldown_s=0,
            message_template=NUDGE_TEMPLATES["upload_complete"],
        ),
        "processing_complete": TriggerConfig(
            max_per_event=1, cooldown_s=0,
            message_template=NUDGE_TEMPLATES["processing_complete"],
        ),
        "missing_config": TriggerConfig(
            max_per_session=1, cooldown_s=0, dismissable=True,
            message_template=NUDGE_TEMPLATES["missing_config"],
        ),
        "pii_detected": TriggerConfig(
            max_per_dataset=1, cooldown_s=0,
            message_template=NUDGE_TEMPLATES["pii_detected"],
        ),
        "long_running_op": TriggerConfig(
            max_per_operation=1, cooldown_s=30,
            message_template=NUDGE_TEMPLATES["long_running_op"],
        ),
        "destructive_action": TriggerConfig(
            max_per_event=1, cooldown_s=0, dismissable=False, required=True,
            message_template=NUDGE_TEMPLATES["destructive_action"],
        ),
        "first_search": TriggerConfig(
            max_per_session=1, cooldown_s=30, dismissable=True,
            message_template=NUDGE_TEMPLATES["first_search"],
        ),
    }

    # Max entries before evicting stale dataset/operation counts
    _MAX_COUNT_ENTRIES = 1000
    # Entries older than this (seconds) are considered stale
    _COUNT_TTL_S = 3600  # 1 hour

    def __init__(self) -> None:
        # Rate limit state: {session_id: {trigger: [timestamp, ...]}}
        self._session_nudge_history: Dict[str, Dict[str, List[float]]] = {}
        # Session-level counters: {session_id: {trigger: count}}
        self._session_counts: Dict[str, Dict[str, int]] = {}
        # Dataset-level counters: {dataset_id: {trigger: count}}
        self._dataset_counts: Dict[str, Dict[str, int]] = {}
        # Dataset-level timestamps: {dataset_id: last_updated_timestamp}
        self._dataset_counts_ts: Dict[str, float] = {}
        # Operation-level counters: {operation_id: {trigger: count}}
        self._operation_counts: Dict[str, Dict[str, int]] = {}
        # Operation-level timestamps: {operation_id: last_updated_timestamp}
        self._operation_counts_ts: Dict[str, float] = {}
        # Session-level dismissals: {session_id: set(trigger)}
        self._session_dismissals: Dict[str, set] = {}
        # Permanent dismissals (loaded from DB): {user_id: set(trigger)}
        self._permanent_dismissals: Dict[str, set] = {}
        # Quiet mode: {session_id: bool}
        self._quiet_mode: Dict[str, bool] = {}
        # Analytics buffer (flushed periodically or on persist)
        self._analytics: List[NudgeAnalytics] = []
        # Fix 2: Track issued nudges per session: {session_id: {nudge_id: trigger}}
        self._issued_nudges: Dict[str, Dict[str, str]] = {}
        # Fix 4: asyncio.Lock for maybe_nudge concurrency safety
        self._lock: Optional[asyncio.Lock] = None
        # Fix 5: Counter for periodic pruning
        self._nudge_check_count: int = 0

    @property
    def _async_lock(self) -> asyncio.Lock:
        """Lazily create lock in the current event loop."""
        if self._lock is None:
            self._lock = asyncio.Lock()
        return self._lock

    async def maybe_nudge(
        self,
        trigger: str,
        context: Dict[str, Any],
        session_id: str,
        user_id: str = "",
    ) -> Optional[NudgeMessage]:
        """
        Check if a nudge should fire. Returns None if suppressed.

        Respects: allowlist, rate limits, quiet mode, user dismissals.
        Uses asyncio.Lock to prevent concurrent double-firing.

        Args:
            trigger: The trigger name (must be in TRIGGER_ALLOWLIST)
            context: Event context (dataset_id, operation_id, error_message, etc.)
            session_id: WebSocket session ID
            user_id: User ID for permanent dismissal checks

        Returns:
            NudgeMessage if nudge should fire, None if suppressed.
        """
        async with self._async_lock:
            return self._maybe_nudge_unlocked(trigger, context, session_id, user_id)

    def _maybe_nudge_unlocked(
        self,
        trigger: str,
        context: Dict[str, Any],
        session_id: str,
        user_id: str = "",
    ) -> Optional[NudgeMessage]:
        """Inner nudge logic — caller must hold _async_lock."""
        # Periodic pruning of stale counts (every 100 checks)
        self._nudge_check_count += 1
        if self._nudge_check_count % 100 == 0:
            self._prune_stale_counts()

        # 1. Check allowlist
        config = self.TRIGGER_ALLOWLIST.get(trigger)
        if config is None:
            logger.warning("Nudge trigger '%s' not in allowlist — blocked", trigger)
            return None

        # 2. Check quiet mode
        if self._quiet_mode.get(session_id, False):
            logger.debug("Nudge suppressed by quiet mode: trigger=%s session=%s", trigger, session_id)
            return None

        # 3. Check permanent dismissals (user-level, from DB)
        if user_id and trigger in self._permanent_dismissals.get(user_id, set()):
            logger.debug("Nudge permanently dismissed: trigger=%s user=%s", trigger, user_id)
            return None

        # 4. Check session-level dismissals
        if trigger in self._session_dismissals.get(session_id, set()):
            logger.debug("Nudge session-dismissed: trigger=%s session=%s", trigger, session_id)
            return None

        # 5. Check rate limits
        now = time.time()

        # Per-session limit
        if config.max_per_session > 0:
            count = self._session_counts.get(session_id, {}).get(trigger, 0)
            if count >= config.max_per_session:
                logger.debug("Nudge rate-limited (session): trigger=%s count=%d", trigger, count)
                return None

        # Per-dataset limit
        dataset_id = context.get("dataset_id")
        if config.max_per_dataset > 0 and dataset_id:
            count = self._dataset_counts.get(dataset_id, {}).get(trigger, 0)
            if count >= config.max_per_dataset:
                logger.debug("Nudge rate-limited (dataset): trigger=%s dataset=%s", trigger, dataset_id)
                return None

        # Per-operation limit
        operation_id = context.get("operation_id")
        if config.max_per_operation > 0 and operation_id:
            count = self._operation_counts.get(operation_id, {}).get(trigger, 0)
            if count >= config.max_per_operation:
                logger.debug("Nudge rate-limited (operation): trigger=%s op=%s", trigger, operation_id)
                return None

        # Cooldown check
        if config.cooldown_s > 0:
            history = self._session_nudge_history.get(session_id, {}).get(trigger, [])
            if history and (now - history[-1]) < config.cooldown_s:
                logger.debug(
                    "Nudge rate-limited (cooldown): trigger=%s remaining=%ds",
                    trigger, config.cooldown_s - (now - history[-1]),
                )
                return None

        # Per-event limit (checked against recent history within a small window)
        if config.max_per_event > 0:
            # Check within 1-second event window
            history = self._session_nudge_history.get(session_id, {}).get(trigger, [])
            recent = [t for t in history if (now - t) < 1.0]
            if len(recent) >= config.max_per_event:
                return None

        # --- All checks passed: build nudge ---
        nudge_id = f"ndg_{uuid.uuid4().hex[:12]}"
        message = context.get("message") or config.message_template

        # Record in rate limit state
        self._session_nudge_history.setdefault(session_id, {}).setdefault(trigger, []).append(now)
        self._session_counts.setdefault(session_id, {})[trigger] = (
            self._session_counts.get(session_id, {}).get(trigger, 0) + 1
        )
        if dataset_id:
            self._dataset_counts.setdefault(dataset_id, {})[trigger] = (
                self._dataset_counts.get(dataset_id, {}).get(trigger, 0) + 1
            )
            self._dataset_counts_ts[dataset_id] = now
        if operation_id:
            self._operation_counts.setdefault(operation_id, {})[trigger] = (
                self._operation_counts.get(operation_id, {}).get(trigger, 0) + 1
            )
            self._operation_counts_ts[operation_id] = now

        # Track issued nudge for this session (Fix 2)
        self._issued_nudges.setdefault(session_id, {})[nudge_id] = trigger

        # Track analytics: nudge_shown
        self._analytics.append(NudgeAnalytics(
            nudge_id=nudge_id,
            trigger=trigger,
            action="nudge_shown",
            session_id=session_id,
            user_id=user_id,
        ))

        nudge = NudgeMessage(
            nudge_id=nudge_id,
            trigger=trigger,
            message=message,
            dismissable=config.dismissable and not config.required,
            metadata={
                "icon": NUDGE_ICONS.get(trigger, "Info"),
                "dataset_id": dataset_id,
                "operation_id": operation_id,
            },
        )

        logger.info(
            "Nudge fired: id=%s trigger=%s session=%s user=%s",
            nudge_id, trigger, session_id, user_id,
        )
        return nudge

    def was_nudge_issued(self, session_id: str, nudge_id: str) -> bool:
        """Check if a nudge_id was actually issued to the given session."""
        return nudge_id in self._issued_nudges.get(session_id, {})

    def record_dismissal(
        self,
        session_id: str,
        trigger: str,
        permanent: bool = False,
        user_id: str = "",
        nudge_id: str = "",
    ) -> None:
        """
        Record user dismissal of a nudge.

        Args:
            session_id: WebSocket session ID
            trigger: The trigger type being dismissed
            permanent: If True, "Don't show again" (persisted to DB)
            user_id: User ID (required for permanent dismissals)
            nudge_id: The nudge ID being dismissed
        """
        # Defensive: only accept known triggers
        if trigger not in self.TRIGGER_ALLOWLIST:
            logger.warning("record_dismissal called with unknown trigger: %s", trigger)
            return

        # Session-level dismissal (always)
        self._session_dismissals.setdefault(session_id, set()).add(trigger)

        # Permanent dismissal
        if permanent and user_id:
            self._permanent_dismissals.setdefault(user_id, set()).add(trigger)
            # Persist to DB (handled by caller via _persist_permanent_dismissal)

        # Analytics
        action = "nudge_permanent_dismiss" if permanent else "nudge_dismissed"
        self._analytics.append(NudgeAnalytics(
            nudge_id=nudge_id or f"unknown_{trigger}",
            trigger=trigger,
            action=action,
            session_id=session_id,
            user_id=user_id,
        ))

        logger.info(
            "Nudge dismissed: trigger=%s permanent=%s session=%s user=%s",
            trigger, permanent, session_id, user_id,
        )

    def record_acted(
        self,
        session_id: str,
        trigger: str,
        user_id: str = "",
        nudge_id: str = "",
    ) -> None:
        """Record that user took action on a nudge."""
        self._analytics.append(NudgeAnalytics(
            nudge_id=nudge_id or f"unknown_{trigger}",
            trigger=trigger,
            action="nudge_acted",
            session_id=session_id,
            user_id=user_id,
        ))

    def set_quiet_mode(self, session_id: str, quiet: bool) -> None:
        """Enable/disable quiet mode for a session."""
        self._quiet_mode[session_id] = quiet

    def load_permanent_dismissals(self, user_id: str, triggers: List[str]) -> None:
        """Load permanent dismissals from DB for a user. Called on session init."""
        self._permanent_dismissals[user_id] = set(triggers)

    def _prune_stale_counts(self) -> None:
        """Remove stale entries from _dataset_counts and _operation_counts (Fix 5)."""
        now = time.time()
        cutoff = now - self._COUNT_TTL_S

        # Prune stale dataset counts
        stale_ds = [k for k, ts in self._dataset_counts_ts.items() if ts < cutoff]
        for k in stale_ds:
            self._dataset_counts.pop(k, None)
            self._dataset_counts_ts.pop(k, None)

        # Prune stale operation counts
        stale_ops = [k for k, ts in self._operation_counts_ts.items() if ts < cutoff]
        for k in stale_ops:
            self._operation_counts.pop(k, None)
            self._operation_counts_ts.pop(k, None)

        # Hard cap: if still over limit, evict oldest
        if len(self._dataset_counts) > self._MAX_COUNT_ENTRIES:
            sorted_ds = sorted(self._dataset_counts_ts.items(), key=lambda x: x[1])
            to_evict = len(self._dataset_counts) - self._MAX_COUNT_ENTRIES
            for k, _ in sorted_ds[:to_evict]:
                self._dataset_counts.pop(k, None)
                self._dataset_counts_ts.pop(k, None)

        if len(self._operation_counts) > self._MAX_COUNT_ENTRIES:
            sorted_ops = sorted(self._operation_counts_ts.items(), key=lambda x: x[1])
            to_evict = len(self._operation_counts) - self._MAX_COUNT_ENTRIES
            for k, _ in sorted_ops[:to_evict]:
                self._operation_counts.pop(k, None)
                self._operation_counts_ts.pop(k, None)

        if stale_ds or stale_ops:
            logger.debug(
                "Pruned stale counts: datasets=%d operations=%d",
                len(stale_ds), len(stale_ops),
            )

    def cleanup_session(self, session_id: str) -> None:
        """Clean up state for a disconnected session."""
        self._session_nudge_history.pop(session_id, None)
        self._session_counts.pop(session_id, None)
        self._session_dismissals.pop(session_id, None)
        self._quiet_mode.pop(session_id, None)
        self._issued_nudges.pop(session_id, None)

    def get_analytics(self) -> List[NudgeAnalytics]:
        """Get and clear the analytics buffer."""
        analytics = self._analytics.copy()
        self._analytics.clear()
        return analytics

    def to_ws_message(self, nudge: NudgeMessage) -> dict:
        """Convert a NudgeMessage to a WebSocket message dict."""
        return {
            "type": "NUDGE",
            "trigger": nudge.trigger,
            "message": nudge.message,
            "nudge_id": nudge.nudge_id,
            "dismissable": nudge.dismissable,
            "icon": nudge.metadata.get("icon", "Info"),
        }


# Module-level singleton
nudge_manager = NudgeManager()
