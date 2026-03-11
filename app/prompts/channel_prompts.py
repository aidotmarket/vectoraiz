"""
Channel Prompts — Static templates keyed by ChannelType (BQ-VZ-CHANNEL)
=======================================================================

Condition C4: Fixed local prompt templates keyed by enum.
No raw env interpolation — channel value is ONLY used as a dict key,
never embedded in prompt text.
"""

from app.core.channel_config import ChannelType

# Static templates — no string interpolation of channel value (C4)
CHANNEL_GREETINGS: dict[ChannelType, str] = {
    ChannelType.direct: (
        "Hi! I'm your data copilot. I can help you process, explore, "
        "and query your data. What would you like to work on?"
    ),
    ChannelType.marketplace: (
        "Hi! I'm your marketplace copilot. I'll help you get your data "
        "listed on ai.market — from upload to publishing. Ready to start?"
    ),
}

CHANNEL_SYSTEM_CONTEXTS: dict[ChannelType, str] = {
    ChannelType.direct: (
        "The user is using vectorAIz primarily as a data processing tool. "
        "Focus on helping with data ingestion, vectorization, chunking strategies, "
        "and RAG queries. Mention marketplace publishing as a secondary option "
        "when relevant."
    ),
    ChannelType.marketplace: (
        "The user downloaded vectorAIz from ai.market and is primarily interested "
        "in listing their data for sale. Focus on helping with listing creation, "
        "metadata enrichment, pricing strategy, compliance checks, and publishing. "
        "Data processing features support the goal of creating high-quality listings."
    ),
}


def get_greeting(channel: ChannelType) -> str:
    return CHANNEL_GREETINGS[channel]


def get_system_context(channel: ChannelType) -> str:
    return CHANNEL_SYSTEM_CONTEXTS[channel]
