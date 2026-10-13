"""Real ai.market providers and mounted portal auth; all wire data is synthetic.

No full application lifespan, embedding model, or external service is needed.
"""

import base64
import builtins
import copy
import json
import socket
import sys
import types
from datetime import datetime, timedelta, timezone
from pathlib import Path
from types import SimpleNamespace

import httpx
import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient

from app.config import settings
from app.middleware import portal_auth
from app.models import portal
from app.routers import portal_allai, website_chat
from app.schemas.portal import PortalConfig, PortalSession, PortalTier
from app.services import allai_agentic_provider as agentic, allie_provider as allie
from app.services.allai_tool_result import ToolResult


DISABLED = "Allie requires an ai.market connection. Switch to connected mode to use Allie."


@pytest.fixture
def boundary(tmp_path, monkeypatch, request):
    """Owned retained install token, real config reader, fake HTTP transport.

    Count file opens and client construction in addition to wire requests;
    socket network is forbidden even in connected positive controls.
    """
    monkeypatch.setattr(settings, "serial_data_dir", str(tmp_path))
    monkeypatch.setattr(settings, "mode", "connected")
    monkeypatch.setattr(settings, "internal_api_key", "synthetic-env-token")
    monkeypatch.setattr(settings, "serial", "synthetic-env-serial")
    monkeypatch.setattr(settings, "ai_market_url", "https://synthetic.invalid")
    monkeypatch.delenv("VECTORAIZ_ALLIE_PROVIDER", raising=False)
    allie.reset_provider()
    allie.write_allie_config("synthetic-retained-serial", "synthetic-retained-token", "https://synthetic.invalid")
    image = tmp_path / "synthetic.png"
    image.write_bytes(b"synthetic-image-bytes")
    attachment = SimpleNamespace(type="image", file_path=image, mime_type="image/png", filename=image.name)
    document = SimpleNamespace(type="document", extracted_text="synthetic attached document", filename="synthetic.txt")
    counts = {"config_reads": 0, "attachment_opens": 0, "clients": 0, "transport": 0}
    requests = []
    responses = []
    real_open = builtins.open
    real_client = httpx.AsyncClient

    def tracked_open(path, *args, **kwargs):
        if isinstance(path, (str, Path)):
            if Path(path) == tmp_path / allie.ALLIE_CONFIG_FILENAME:
                counts["config_reads"] += 1
            if Path(path) == image:
                counts["attachment_opens"] += 1
        return real_open(path, *args, **kwargs)

    def wire(request):
        counts["transport"] += 1
        requests.append({"url": str(request.url), "headers": dict(request.headers), "body": json.loads(request.content)})
        if request.url.path.endswith("/agentic"):
            content = responses.pop(0) if responses else [{"type": "text", "text": "synthetic reply"}]
            return httpx.Response(200, json={"content": content, "model": "synthetic-model", "usage": {"input_tokens": 10, "output_tokens": 5, "cost_cents": 2}})
        return httpx.Response(200, text='event: start\ndata: {"model":"synthetic-model"}\n\nevent: delta\ndata: {"text":"synthetic reply"}\n\nevent: done\ndata: {"usage":{"input_tokens":10,"output_tokens":5},"cost_cents":2}\n\n')

    def client(**kwargs):
        counts["clients"] += 1
        return real_client(transport=httpx.MockTransport(wire), **kwargs)

    def no_network(*args, **kwargs):
        raise AssertionError("Actual socket network forbidden")

    monkeypatch.setattr(builtins, "open", tracked_open)
    monkeypatch.setattr(httpx, "AsyncClient", client)
    monkeypatch.setattr(socket.socket, "connect", no_network)
    monkeypatch.setattr(socket.socket, "connect_ex", no_network)
    monkeypatch.setattr(socket, "create_connection", no_network)
    yield SimpleNamespace(counts=counts, requests=requests, responses=responses, attachments=[attachment, document])
    request.node.user_properties.append(("synthetic_boundary_counts", json.dumps(counts, sort_keys=True)))
    request.node.user_properties.append(("synthetic_wire_requests", json.dumps(requests, sort_keys=True)))
    allie.reset_provider()


def assert_no_outbound(boundary):
    assert boundary.counts == {"config_reads": 0, "attachment_opens": 0, "clients": 0, "transport": 0}, boundary.counts


@pytest.mark.asyncio
@pytest.mark.parametrize("enabled", [True, False])
async def test_standalone_stream_cached_connected_provider(boundary, monkeypatch, enabled):
    provider = allie.AiMarketAllieProvider(api_key="synthetic-explicit", serial="synthetic-serial")
    monkeypatch.setattr(settings, "mode", "standalone")
    monkeypatch.setattr(settings, "allai_enabled", enabled)
    caught = None
    try:
        chunks = [c async for c in provider.stream("synthetic message", context="synthetic context", attachments=boundary.attachments, chat_history=[{"role": "assistant", "content": "synthetic history"}])]
    except allie.AllieDisabledError as exc:
        caught = exc
    assert_no_outbound(boundary)
    assert str(caught) == DISABLED


def test_standalone_constructor_precedes_credential_lookup(boundary, monkeypatch):
    monkeypatch.setattr(settings, "mode", "standalone")
    # A missing key must still produce the existing disabled exception first.
    monkeypatch.setattr(settings, "internal_api_key", None)
    with pytest.raises(allie.AllieDisabledError, match="ai.market connection"):
        allie.AiMarketAllieProvider()
    assert_no_outbound(boundary)


@pytest.mark.asyncio
@pytest.mark.parametrize("entry", ["proxy", "llm", "loop"])
@pytest.mark.parametrize("enabled", [True, False])
async def test_standalone_agentic_entries(boundary, monkeypatch, entry, enabled):
    monkeypatch.setattr(settings, "mode", "standalone")
    monkeypatch.setattr(settings, "allai_enabled", enabled)
    provider = agentic.AgenticAllieProvider()
    messages = [{"role": "user", "content": [{"type": "text", "text": "synthetic attachment and tool summary"}]}]
    caught = None
    async def noop(*args):
        pass
    try:
        if entry == "loop":
            await provider.run_agentic_loop(messages, "synthetic context", [], None, noop, noop)
        else:
            await getattr(provider, "_call_" + entry)(messages, "synthetic context", [])
    except allie.AllieDisabledError as exc:
        caught = exc
    assert_no_outbound(boundary)
    assert str(caught) == DISABLED


@pytest.mark.asyncio
@pytest.mark.parametrize("selection", ["", "mock"])
async def test_standalone_mock_retrieval_provider_skips_retained_config(boundary, monkeypatch, selection):
    monkeypatch.setattr(settings, "mode", "standalone")
    monkeypatch.setenv("VECTORAIZ_ALLIE_PROVIDER", selection)
    provider = allie.get_allie_provider()
    assert_no_outbound(boundary)
    assert isinstance(provider, allie.MockAllieProvider)
    chunks = [c async for c in provider.stream("hello", context="local retrieval context")]
    assert any(c.done and c.usage.provider == "mock" for c in chunks)
    assert_no_outbound(boundary)


@pytest.mark.asyncio
async def test_connected_stream_preserves_payload_headers_usage(boundary):
    provider = allie.get_allie_provider()
    chunks = [c async for c in provider.stream("synthetic message", context="synthetic context", attachments=boundary.attachments, chat_history=[{"role": "assistant", "content": "synthetic history"}])]
    assert "".join(c.text for c in chunks) == "synthetic reply"
    assert chunks[-1].done
    assert chunks[-1].usage == allie.AllieUsage(10, 5, 2, "aimarket", "synthetic-model")
    request, = boundary.requests
    assert request["url"] == "https://synthetic.invalid/api/v1/allie/chat"
    assert request["headers"]["x-api-key"] == "synthetic-retained-token"
    assert request["headers"]["x-serial"] == "synthetic-retained-serial"
    assert request["headers"]["accept"] == "text/event-stream"
    assert request["body"]["messages"] == [
        {"role": "user", "content": "synthetic context"},
        {"role": "assistant", "content": "synthetic history"},
        {"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": base64.b64encode(b"synthetic-image-bytes").decode()}},
            {"type": "text", "text": "[Attached: synthetic.txt]\nsynthetic attached document"},
            {"type": "text", "text": "synthetic message"},
        ]},
    ]
    assert boundary.counts == {"config_reads": 1, "attachment_opens": 1, "clients": 1, "transport": 1}


@pytest.mark.asyncio
async def test_connected_agentic_loop_preserves_tool_summary_and_usage(boundary):
    boundary.responses.extend([
        [{"type": "tool_use", "id": "synthetic-tool", "name": "synthetic_read", "input": {}}],
        [{"type": "text", "text": "synthetic reply"}],
    ])
    class Executor:
        async def execute(self, name, args):
            assert name == "synthetic_read" and args == {}
            return ToolResult(frontend_data={"private": "synthetic raw rows"}, llm_summary="synthetic safe summary")
    chunks, beats = [], []
    async def chunk(text):
        chunks.append(text)
    async def beat():
        beats.append(True)
    messages = [{"role": "user", "content": [{"type": "text", "text": "synthetic attached content"}]}]
    original = copy.deepcopy(messages)
    text, usage = await agentic.AgenticAllieProvider().run_agentic_loop(messages, "synthetic system", [{"name": "synthetic_read"}], Executor(), chunk, beat)
    assert text == "synthetic reply" and chunks == [text] and len(beats) == 3
    assert usage == allie.AllieUsage(20, 10, 4, "anthropic", "synthetic-model")
    first, second = boundary.requests
    assert first["body"]["messages"] == original
    assert second["body"]["messages"][-1]["content"] == [{"type": "tool_result", "tool_use_id": "synthetic-tool", "content": "synthetic safe summary"}]
    assert "synthetic raw rows" not in json.dumps(second)
    for request in boundary.requests:
        assert request["url"] == "https://synthetic.invalid/api/v1/allie/chat/agentic"
        assert request["headers"]["x-api-key"] == "synthetic-retained-token"
        assert request["headers"]["x-serial"] == "synthetic-retained-serial"
        assert request["body"]["system"] == "synthetic system"
        assert request["body"]["tools"] == [{"name": "synthetic_read"}]
        assert request["body"]["max_tokens"] == 4096


@pytest.fixture
def mounted_portal(tmp_path, monkeypatch, boundary):
    """Exact router prefix and actual auth dependency, owned disk config and key."""
    monkeypatch.setattr(portal, "_PORTAL_CONFIG_PATH", tmp_path / "portal_config.json")
    monkeypatch.setattr(portal_auth, "_PORTAL_JWT_SECRET_PATH", tmp_path / "portal.key")
    monkeypatch.setattr(portal_auth, "_portal_jwt_secret", None)
    (tmp_path / "portal.key").write_text("synthetic-portal-signing-key-000000000000000000000")
    portal.reset_portal_config_cache()
    portal_allai.clear_chat_rate_limits()
    app = FastAPI()
    app.include_router(portal_allai.router, prefix="/api/portal/allai", tags=["portal-allai"])
    app.include_router(website_chat.router, prefix="/api/website-chat")
    with TestClient(app) as client:
        yield client
    portal.reset_portal_config_cache()
    portal_allai.clear_chat_rate_limits()
    website_chat._rate_log.clear()
    website_chat._conversations.clear()


def portal_headers(tier):
    now = datetime.now(timezone.utc)
    config = PortalConfig(enabled=True, tier=tier, active_sessions={"synthetic-session": (now + timedelta(hours=1)).isoformat()})
    portal.save_portal_config(config)
    portal.reset_portal_config_cache()  # prove actual disk load
    session = PortalSession(session_id="synthetic-session", tier=tier, ip_address="127.0.0.1", created_at=now, expires_at=now + timedelta(hours=1), portal_session_version=config.portal_session_version)
    return {"Authorization": "Bearer " + portal_auth.create_portal_jwt(session)} if tier != PortalTier.open else {}


@pytest.mark.parametrize("tier", list(PortalTier))
@pytest.mark.parametrize("enabled", [True, False])
def test_standalone_mounted_portal_disabled_contract(boundary, mounted_portal, monkeypatch, tier, enabled, request):
    headers = portal_headers(tier)
    monkeypatch.setattr(settings, "mode", "standalone")
    monkeypatch.setattr(settings, "allai_enabled", enabled)
    constructions = []
    original = agentic.AgenticAllieProvider
    def construct():
        constructions.append(True)
        return original()
    monkeypatch.setattr(agentic, "AgenticAllieProvider", construct)
    response = mounted_portal.post("/api/portal/allai/chat", headers=headers, json={"messages": [{"role": "user", "content": "synthetic private request"}]})
    request.node.user_properties.append(("portal_sse", response.text))
    assert_no_outbound(boundary)
    assert constructions == []
    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/event-stream")
    events = [json.loads(line[6:]) for line in response.text.splitlines() if line.startswith("data: ")]
    assert events == [{"type": "error", "text": DISABLED}]


@pytest.mark.parametrize("case,expected", [("disabled", 404), ("missing_auth", 401), ("bad_auth", 401), ("revoked", 401), ("malformed", 422)])
def test_standalone_portal_auth_and_validation_precede_chat(boundary, mounted_portal, monkeypatch, case, expected):
    headers = portal_headers(PortalTier.code if case in {"missing_auth", "bad_auth", "revoked"} else PortalTier.open)
    if case == "disabled":
        portal.save_portal_config(PortalConfig(enabled=False))
    elif case == "missing_auth":
        headers = {}
    elif case == "bad_auth":
        headers = {"Authorization": "Bearer synthetic-invalid"}
    elif case == "revoked":
        config = portal.get_portal_config()
        config.active_sessions.clear()
        portal.save_portal_config(config)
    monkeypatch.setattr(settings, "mode", "standalone")
    body = {"messages": []} if case == "malformed" else {"messages": [{"role": "user", "content": "synthetic"}]}
    response = mounted_portal.post("/api/portal/allai/chat", json=body, headers=headers)
    assert response.status_code == expected
    assert_no_outbound(boundary)


@pytest.mark.parametrize("tier", list(PortalTier))
def test_connected_mounted_portal_real_loop(boundary, mounted_portal, tier):
    headers = portal_headers(tier)
    # Real portal executor denies mutation and feeds its safe summary back.
    boundary.responses.extend([[{"type": "tool_use", "id": "synthetic-blocked", "name": "delete_dataset", "input": {}}], [{"type": "text", "text": "synthetic reply"}]])
    response = mounted_portal.post("/api/portal/allai/chat", headers=headers, json={"messages": [{"role": "user", "content": "synthetic request"}]})
    assert response.status_code == 200
    events = [json.loads(line[6:]) for line in response.text.splitlines() if line.startswith("data: ")]
    assert events == [{"type": "chunk", "text": "synthetic reply"}, {"type": "done"}]
    assert boundary.counts["transport"] == 2
    assert boundary.requests[0]["body"]["system"] == portal_allai.PORTAL_SYSTEM_PROMPT
    assert {t["name"] for t in boundary.requests[0]["body"]["tools"]} <= portal_allai.PORTAL_ALLOWED_TOOLS
    assert "not available" in boundary.requests[1]["body"]["messages"][-1]["content"][0]["content"].lower()


def test_website_cached_real_provider_cannot_send_in_standalone(boundary, mounted_portal, monkeypatch):
    allie.get_allie_provider()  # connected provider cached before mode change
    boundary.counts["config_reads"] = 0
    monkeypatch.setattr(settings, "mode", "standalone")
    response = mounted_portal.post("/api/website-chat", json={"message": "synthetic website context"})
    assert_no_outbound(boundary)
    assert response.status_code == 503


@pytest.fixture
def rag_with_local_retrieval(boundary, monkeypatch, tmp_path):
    """Actual RAG, prompt, citation, context and session code; synthetic retrieval.

    Replace only the heavyweight search import so no embeddings/model downloads
    or Qdrant are needed. Providers and their guards are never replaced.
    """
    search = types.ModuleType("app.services.search_service")
    class LocalSearch:
        def search(self, *args, **kwargs):
            return {"results": [{"dataset_id": "synthetic-dataset", "dataset_name": "synthetic.csv", "row_index": 1, "text_content": "synthetic retrieved context", "score": 0.9}]}
    search.SearchService = LocalSearch
    search.get_search_service = LocalSearch
    monkeypatch.setitem(sys.modules, search.__name__, search)
    from app.services.rag_service import RAGService
    from sqlmodel import SQLModel, Session, create_engine
    engine = create_engine("sqlite:///" + str(tmp_path / "synthetic-rag.db"))
    SQLModel.metadata.create_all(engine)
    with Session(engine) as db:
        svc = RAGService(db)
        session = svc.session_service.create_session(title="synthetic session")
        yield svc, session.id
    engine.dispose()


@pytest.mark.asyncio
@pytest.mark.parametrize("entry", ["query", "query_stream", "chat", "chat_stream", "summary"])
async def test_rag_context_cached_real_provider_denied(boundary, rag_with_local_retrieval, monkeypatch, entry):
    svc, session_id = rag_with_local_retrieval
    svc.context_manager._allie_provider = svc._allie
    boundary.counts["config_reads"] = 0
    monkeypatch.setattr(settings, "mode", "standalone")
    if entry == "summary":
        from app.models.state import Message, MessageRole
        summary = await svc.context_manager.summarize_messages_async([Message(session_id=session_id, role=MessageRole.USER, content="synthetic private history")])
        assert "truncated" in summary  # existing safe fallback contract
    else:
        # query/chat wrap provider failures; streaming methods propagate.
        caught = None
        try:
            method = getattr(svc, entry)
            args = {"question": "synthetic question", "template": "rag_qa"}
            if entry.startswith("chat"):
                args["session_id"] = session_id
            if entry.endswith("stream"):
                output = [c async for c in method(**args)]
            else:
                output = await method(**args)
        except (allie.AllieDisabledError, RuntimeError) as exc:
            caught = exc
        assert caught is not None and "ai.market connection" in str(caught)
    assert_no_outbound(boundary)


@pytest.mark.asyncio
async def test_local_rag_mock_generation_remains_available(boundary, rag_with_local_retrieval, monkeypatch):
    svc, _ = rag_with_local_retrieval
    allie.reset_provider()
    monkeypatch.setattr(settings, "mode", "standalone")
    boundary.counts["config_reads"] = 0  # connected fixture initialization above
    svc._allie = allie.get_allie_provider()
    result = await svc.query("synthetic question")
    assert result.chunks_retrieved == 1
    assert result.model_used == "MockAllieProvider"
    assert result.answer
    assert_no_outbound(boundary)


@pytest.mark.asyncio
async def test_mode_change_between_agentic_tool_iterations_stops_summary(boundary, monkeypatch):
    boundary.responses.append([{"type": "tool_use", "id": "synthetic-tool", "name": "synthetic_read", "input": {}}])
    class Executor:
        async def execute(self, name, args):
            monkeypatch.setattr(settings, "mode", "standalone")
            return ToolResult(frontend_data={}, llm_summary="synthetic private tool summary")
    async def noop(*args):
        pass
    with pytest.raises(allie.AllieDisabledError, match="ai.market connection"):
        await agentic.AgenticAllieProvider().run_agentic_loop([{"role": "user", "content": "synthetic"}], "synthetic system", [], Executor(), noop, noop)
    assert boundary.counts == {"config_reads": 1, "attachment_opens": 0, "clients": 1, "transport": 1}
    assert "synthetic private tool summary" not in json.dumps(boundary.requests)


def test_standalone_explicit_real_factory_denied(boundary, monkeypatch):
    monkeypatch.setattr(settings, "mode", "standalone")
    monkeypatch.setenv("VECTORAIZ_ALLIE_PROVIDER", "aimarket")
    with pytest.raises(allie.AllieDisabledError, match="ai.market connection"):
        allie.get_allie_provider()
    assert_no_outbound(boundary)


def test_standalone_portal_preserves_rate_limit(boundary, mounted_portal, monkeypatch):
    portal_headers(PortalTier.open)
    monkeypatch.setattr(settings, "mode", "standalone")
    body = {"messages": [{"role": "user", "content": "synthetic"}]}
    for _ in range(portal_allai.PORTAL_CHAT_RATE_LIMIT):
        response = mounted_portal.post("/api/portal/allai/chat", json=body)
        assert response.status_code == 200
        assert json.loads(response.text.splitlines()[0][6:]) == {"type": "error", "text": DISABLED}
    assert mounted_portal.post("/api/portal/allai/chat", json=body).status_code == 429
    assert_no_outbound(boundary)


@pytest.mark.asyncio
@pytest.mark.parametrize("entry", ["streaming", "agentic"])
async def test_existing_copilot_guard_precedes_attachment_processing(boundary, monkeypatch, entry):
    from app.services.copilot_service import CoPilotService
    monkeypatch.setattr(settings, "mode", "standalone")
    async def noop(*args):
        pass
    args = {"user": SimpleNamespace(user_id="synthetic-user"), "message": "synthetic", "session_id": "synthetic-session", "message_id": "synthetic-message", "send_chunk": noop, "attachments": boundary.attachments}
    if entry == "agentic":
        args.update(send_ws=noop, send_heartbeat=noop)
    with pytest.raises(allie.AllieDisabledError, match="ai.market connection"):
        await getattr(CoPilotService(), "process_message_" + entry)(**args)
    assert_no_outbound(boundary)
