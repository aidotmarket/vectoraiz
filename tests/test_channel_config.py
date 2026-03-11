"""
Tests for channel config parsing (BQ-VZ-CHANNEL, Condition C1/C5).

6 test cases: marketplace, direct, unset, invalid, case-insensitive, whitespace.
"""

import logging
import os
from unittest.mock import patch

import pytest

from app.core.channel_config import ChannelType, parse_channel


def test_channel_marketplace():
    """VECTORAIZ_CHANNEL=marketplace → ChannelType.marketplace"""
    with patch.dict(os.environ, {"VECTORAIZ_CHANNEL": "marketplace"}):
        assert parse_channel() == ChannelType.marketplace


def test_channel_direct():
    """VECTORAIZ_CHANNEL=direct → ChannelType.direct"""
    with patch.dict(os.environ, {"VECTORAIZ_CHANNEL": "direct"}):
        assert parse_channel() == ChannelType.direct


def test_channel_unset():
    """VECTORAIZ_CHANNEL unset → ChannelType.direct (default)"""
    with patch.dict(os.environ, {}, clear=True):
        # Ensure VECTORAIZ_CHANNEL is not set
        os.environ.pop("VECTORAIZ_CHANNEL", None)
        assert parse_channel() == ChannelType.direct


def test_channel_invalid(caplog):
    """VECTORAIZ_CHANNEL=foobar → ChannelType.direct + warning logged"""
    with patch.dict(os.environ, {"VECTORAIZ_CHANNEL": "foobar"}):
        with caplog.at_level(logging.WARNING):
            result = parse_channel()
        assert result == ChannelType.direct
        assert "Invalid VECTORAIZ_CHANNEL" in caplog.text


def test_channel_case_insensitive():
    """VECTORAIZ_CHANNEL=MARKETPLACE → ChannelType.marketplace (CH-C1)"""
    with patch.dict(os.environ, {"VECTORAIZ_CHANNEL": "MARKETPLACE"}):
        assert parse_channel() == ChannelType.marketplace


def test_channel_whitespace():
    """VECTORAIZ_CHANNEL=' marketplace ' → trimmed → marketplace"""
    with patch.dict(os.environ, {"VECTORAIZ_CHANNEL": " marketplace "}):
        assert parse_channel() == ChannelType.marketplace
