"""
Shared helper: build multimodal user content blocks from chat attachments.

Extracted from copilot_service.py so both CoPilotService and AllieProvider
can import without circular dependency.

BQ-ALLAI-FILES (S135)
"""

import base64
from typing import Optional, Union

from app.services.chat_attachment_service import ChatAttachment


def build_user_content(
    message: str,
    attachments: Optional[list[ChatAttachment]] = None,
    supports_vision: bool = True,
) -> Union[str, list]:
    """
    Build user message content, adding attachment content blocks if present.

    Returns plain string for no-attachment case (backward compatible),
    or a list of content blocks for multimodal messages.

    When supports_vision=False, image attachments produce a text note instead
    of base64 image blocks (M6 provider capability check).
    """
    if not attachments:
        return message

    content_blocks: list[dict] = []
    for att in attachments:
        if att.type == "image":
            if supports_vision:
                with open(att.file_path, "rb") as f:
                    b64 = base64.b64encode(f.read()).decode()
                content_blocks.append({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": att.mime_type,
                        "data": b64,
                    },
                })
            else:
                content_blocks.append({
                    "type": "text",
                    "text": (
                        f"[Note: Your configured model doesn't support images. "
                        f"Image '{att.filename}' was attached but cannot be processed.]"
                    ),
                })
        elif att.type in ("document", "data") and att.extracted_text:
            content_blocks.append({
                "type": "text",
                "text": f"[Attached: {att.filename}]\n{att.extracted_text}",
            })
    # User message as final text block
    content_blocks.append({"type": "text", "text": message})
    return content_blocks
