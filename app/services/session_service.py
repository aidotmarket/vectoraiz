"""
Session Service
===============

CRUD operations for chat sessions and messages.
Manages local SQLite storage for conversation history.

Phase: 3.W.2
Created: 2026-01-25
"""

from typing import List, Optional, Dict, Any
from uuid import UUID
from datetime import datetime, timezone

from sqlmodel import Session as DBSession, select, func, desc

from app.models.state import Session, Message, MessageRole


class SessionService:
    """
    Service for managing Chat Sessions and Messages.
    
    All operations are local to the customer's vectorAIz instance.
    Data never leaves to ai.market.
    """

    def __init__(self, db: DBSession):
        """Initialize with database session."""
        self.db = db

    # =========================================================================
    # Session CRUD
    # =========================================================================

    def create_session(
        self, 
        title: Optional[str] = None, 
        dataset_id: Optional[str] = None
    ) -> Session:
        """
        Create a new chat session.
        
        Args:
            title: Optional session title (auto-generated if not provided)
            dataset_id: Optional dataset to scope this conversation to
        
        Returns:
            The created Session object
        """
        session = Session(
            title=title,
            dataset_id=dataset_id,
            total_message_count=0,
            archived=False
        )
        self.db.add(session)
        self.db.commit()
        self.db.refresh(session)
        return session

    def get_session(self, session_id: UUID) -> Optional[Session]:
        """
        Get a session by ID.
        
        Args:
            session_id: UUID of the session
            
        Returns:
            Session if found, None otherwise
        """
        return self.db.get(Session, session_id)

    def list_sessions(
        self, 
        include_archived: bool = False, 
        limit: int = 50, 
        offset: int = 0
    ) -> List[Session]:
        """
        List sessions ordered by most recently updated.
        
        Args:
            include_archived: Include archived sessions
            limit: Max sessions to return
            offset: Pagination offset
            
        Returns:
            List of Session objects
        """
        statement = select(Session)
        
        if not include_archived:
            statement = statement.where(Session.archived == False)  # noqa: E712
            
        statement = statement.order_by(desc(Session.updated_at))
        statement = statement.offset(offset).limit(limit)
        
        return list(self.db.exec(statement).all())

    def update_session(
        self, 
        session_id: UUID, 
        title: Optional[str] = None, 
        archived: Optional[bool] = None
    ) -> Optional[Session]:
        """
        Update session metadata.
        
        Args:
            session_id: UUID of the session
            title: New title (if provided)
            archived: New archived status (if provided)
            
        Returns:
            Updated Session if found, None otherwise
        """
        session = self.get_session(session_id)
        if not session:
            return None
            
        if title is not None:
            session.title = title
        if archived is not None:
            session.archived = archived
            
        session.updated_at = datetime.now(timezone.utc)
        
        self.db.add(session)
        self.db.commit()
        self.db.refresh(session)
        return session

    def delete_session(self, session_id: UUID, hard_delete: bool = False) -> bool:
        """
        Delete a session.
        
        Args:
            session_id: UUID of the session
            hard_delete: If True, permanently remove. If False, soft delete (archived=True)
            
        Returns:
            True if session was found and deleted, False otherwise
        """
        session = self.get_session(session_id)
        if not session:
            return False
            
        if hard_delete:
            # Cascade will handle messages
            self.db.delete(session)
        else:
            session.archived = True
            session.updated_at = datetime.now(timezone.utc)
            self.db.add(session)
            
        self.db.commit()
        return True

    # =========================================================================
    # Message Operations
    # =========================================================================

    def add_message(
        self, 
        session_id: UUID, 
        role: MessageRole, 
        content: str, 
        token_count: Optional[int] = None, 
        metadata: Optional[Dict[str, Any]] = None
    ) -> Message:
        """
        Add a message to a session.
        
        Side effects:
        - Increments session.message_count
        - Updates session.updated_at
        - Auto-generates session title from first user message if not set
        
        Args:
            session_id: UUID of the session
            role: Message role (user, assistant, system)
            content: Message content
            token_count: Token count for context window tracking
            metadata: Additional metadata (citations, sources, etc.)
            
        Returns:
            The created Message object
            
        Raises:
            ValueError: If session not found
        """
        session = self.get_session(session_id)
        if not session:
            raise ValueError(f"Session {session_id} not found")

        # Create message
        message = Message(
            session_id=session_id,
            role=role,
            content=content,
            token_count=token_count,
            metadata_=metadata or {},
        )
        
        # Update session stats
        session.total_message_count += 1
        session.updated_at = datetime.now(timezone.utc)
        
        # Auto-title generation from first user message
        if not session.title and role == MessageRole.USER:
            # Take first line, max 50 chars
            first_line = content.strip().split('\n')[0]
            if len(first_line) > 50:
                session.title = first_line[:50] + "..."
            else:
                session.title = first_line

        self.db.add(message)
        self.db.add(session)
        self.db.commit()
        self.db.refresh(message)
        return message

    def get_messages(
        self, 
        session_id: UUID, 
        limit: int = 100,
        offset: int = 0
    ) -> List[Message]:
        """
        Get messages for a session, ordered chronologically.
        
        Args:
            session_id: UUID of the session
            limit: Max messages to return
            offset: Pagination offset
            
        Returns:
            List of Message objects
        """
        statement = (
            select(Message)
            .where(Message.session_id == session_id)
            .order_by(Message.created_at)
            .offset(offset)
            .limit(limit)
        )
        return list(self.db.exec(statement).all())

    def count_tokens(self, session_id: UUID) -> int:
        """
        Calculate total tokens used in a session.
        
        Args:
            session_id: UUID of the session
            
        Returns:
            Sum of all message token counts (0 if none)
        """
        statement = (
            select(func.sum(Message.token_count))
            .where(Message.session_id == session_id)
        )
        result = self.db.exec(statement).first()
        return result if result else 0

    def get_recent_context(
        self, 
        session_id: UUID, 
        max_tokens: int = 4000
    ) -> List[Message]:
        """
        Get recent messages that fit within a token budget.
        Used for context window management.
        
        Args:
            session_id: UUID of the session
            max_tokens: Maximum tokens to include
            
        Returns:
            List of recent messages within token budget
        """
        # Get all messages, newest first
        statement = (
            select(Message)
            .where(Message.session_id == session_id)
            .order_by(desc(Message.created_at))
        )
        messages = list(self.db.exec(statement).all())
        
        # Build context from most recent, respecting token limit
        context = []
        total_tokens = 0
        
        for msg in messages:
            msg_tokens = msg.token_count or 0
            if total_tokens + msg_tokens <= max_tokens:
                context.append(msg)
                total_tokens += msg_tokens
            else:
                break
        
        # Reverse to chronological order
        return list(reversed(context))


# =============================================================================
# Dependency Helper
# =============================================================================

def get_session_service(db: DBSession) -> SessionService:
    """FastAPI dependency to get SessionService instance."""
    return SessionService(db)
