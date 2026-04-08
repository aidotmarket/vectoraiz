"""
Tests for channel prompt templates (BQ-VZ-CHANNEL, Condition C4/C5).

Ensures all ChannelType members have templates and templates are static
(no f-string interpolation of channel value).
"""

from app.core.channel_config import ChannelType
from app.prompts.channel_prompts import (
    CHANNEL_GREETINGS,
    CHANNEL_SYSTEM_CONTEXTS,
    get_greeting,
    get_system_context,
)


def test_all_channel_types_have_greetings():
    """Every ChannelType enum member has a greeting template."""
    for member in ChannelType:
        assert member in CHANNEL_GREETINGS, f"Missing greeting for {member}"
        assert len(CHANNEL_GREETINGS[member]) > 0


def test_all_channel_types_have_system_contexts():
    """Every ChannelType enum member has a system context template."""
    for member in ChannelType:
        assert member in CHANNEL_SYSTEM_CONTEXTS, f"Missing system context for {member}"
        assert len(CHANNEL_SYSTEM_CONTEXTS[member]) > 0


def test_greeting_templates_are_static():
    """Templates contain no {channel} or f-string interpolation markers (C4)."""
    for member in ChannelType:
        text = CHANNEL_GREETINGS[member]
        assert "{channel}" not in text, f"Greeting for {member} contains {{channel}}"
        assert "{CHANNEL}" not in text, f"Greeting for {member} contains {{CHANNEL}}"


def test_system_context_templates_are_static():
    """System context templates contain no {channel} interpolation markers (C4)."""
    for member in ChannelType:
        text = CHANNEL_SYSTEM_CONTEXTS[member]
        assert "{channel}" not in text, f"System context for {member} contains {{channel}}"
        assert "{CHANNEL}" not in text, f"System context for {member} contains {{CHANNEL}}"


def test_get_greeting_returns_string():
    """get_greeting() returns a non-empty string for each channel."""
    for member in ChannelType:
        result = get_greeting(member)
        assert isinstance(result, str)
        assert len(result) > 0


def test_get_system_context_returns_string():
    """get_system_context() returns a non-empty string for each channel."""
    for member in ChannelType:
        result = get_system_context(member)
        assert isinstance(result, str)
        assert len(result) > 0


def test_aim_data_prompt_keys_exist():
    """aim-data channel is wired into both prompt dictionaries."""
    assert ChannelType.aim_data in CHANNEL_GREETINGS
    assert ChannelType.aim_data in CHANNEL_SYSTEM_CONTEXTS
