"""Tests for BQ-106: Text format support (.txt, .md, .html)."""

import pytest
import tempfile
from pathlib import Path
import io

from app.services.text_processor import TextProcessor
from app.routers.datasets import SUPPORTED_EXTENSIONS, get_file_extension
from app.services.processing_service import TEXT_TYPES
from app.main import app
from fastapi.testclient import TestClient

client = TestClient(app)


@pytest.fixture
def processor():
    return TextProcessor()


# --- TextProcessor unit tests ---


def test_txt_upload(processor):
    """Clean UTF-8 .txt file → success."""
    with tempfile.NamedTemporaryFile(suffix='.txt', mode='w', encoding='utf-8', delete=False) as f:
        f.write("Hello, world!\nSecond line.")
        f.flush()
        result = processor.process(Path(f.name))

    assert result["text_content"] == "Hello, world!\nSecond line."
    assert result["metadata"]["char_count"] == 26
    assert result["metadata"]["line_count"] == 2
    assert result["metadata"]["processor"] == "text_processor"
    assert "utf-8" in result["metadata"]["encoding"]


def test_txt_encoding_fallback(processor):
    """Latin-1 encoded file → success via fallback."""
    with tempfile.NamedTemporaryFile(suffix='.txt', delete=False) as f:
        # Write latin-1 encoded text with non-UTF-8 byte
        f.write("café résumé naïve".encode('latin-1'))
        f.flush()
        result = processor.process(Path(f.name))

    assert "caf" in result["text_content"]
    assert result["metadata"]["char_count"] > 0


def test_md_upload(processor):
    """Markdown file → raw content preserved (no stripping)."""
    md_content = "# Heading\n\n**bold** and *italic*\n\n- list item\n- another"
    with tempfile.NamedTemporaryFile(suffix='.md', mode='w', encoding='utf-8', delete=False) as f:
        f.write(md_content)
        f.flush()
        result = processor.process(Path(f.name))

    # Raw markdown must be preserved exactly
    assert result["text_content"] == md_content
    assert "# Heading" in result["text_content"]
    assert "**bold**" in result["text_content"]
    assert result["metadata"]["processor"] == "text_processor"


def test_html_upload(processor):
    """HTML with script/style → text only."""
    html = """<html><head><style>body{color:red}</style></head>
    <body><p>Hello world</p><script>alert('x')</script></body></html>"""
    with tempfile.NamedTemporaryFile(suffix='.html', mode='w', encoding='utf-8', delete=False) as f:
        f.write(html)
        f.flush()
        result = processor.process(Path(f.name))

    assert "Hello world" in result["text_content"]
    assert "alert" not in result["text_content"]
    assert "color:red" not in result["text_content"]


def test_html_strip_scripts(processor):
    """Scripts, styles, nav, header, footer, aside all removed."""
    html = """<html><body>
    <header>Site Header</header>
    <nav>Navigation</nav>
    <main><p>Main content here</p></main>
    <aside>Sidebar</aside>
    <footer>Footer text</footer>
    <script>var x = 1;</script>
    <style>.foo { display: none; }</style>
    </body></html>"""
    with tempfile.NamedTemporaryFile(suffix='.html', mode='w', encoding='utf-8', delete=False) as f:
        f.write(html)
        f.flush()
        result = processor.process(Path(f.name))

    assert "Main content here" in result["text_content"]
    assert "Site Header" not in result["text_content"]
    assert "Navigation" not in result["text_content"]
    assert "Sidebar" not in result["text_content"]
    assert "Footer text" not in result["text_content"]
    assert "var x" not in result["text_content"]
    assert "display: none" not in result["text_content"]


def test_empty_text_file(processor):
    """Empty .txt file → success, char_count=0."""
    with tempfile.NamedTemporaryFile(suffix='.txt', mode='w', encoding='utf-8', delete=False) as f:
        f.write("")
        f.flush()
        result = processor.process(Path(f.name))

    assert result["text_content"] == ""
    assert result["metadata"]["char_count"] == 0
    assert result["metadata"]["line_count"] == 0


def test_bom_file(processor):
    """BOM-prefixed file → BOM stripped (utf-8-sig handles it)."""
    with tempfile.NamedTemporaryFile(suffix='.txt', delete=False) as f:
        # Write UTF-8 BOM + content
        f.write(b'\xef\xbb\xbfHello BOM')
        f.flush()
        result = processor.process(Path(f.name))

    # utf-8-sig strips the BOM
    assert result["text_content"] == "Hello BOM"
    assert result["metadata"]["encoding"] == "utf-8-sig"


# --- Extension normalization tests ---


def test_extension_normalization():
    """.TXT, .Md → accepted (case-insensitive)."""
    assert get_file_extension("README.TXT") == ".txt"
    assert get_file_extension("notes.Md") == ".md"
    assert get_file_extension("page.HTML") == ".html"

    assert ".txt" in SUPPORTED_EXTENSIONS
    assert ".md" in SUPPORTED_EXTENSIONS
    assert ".html" in SUPPORTED_EXTENSIONS


def test_text_types_in_processing_service():
    """TEXT_TYPES constant contains txt, md, html, htm."""
    assert TEXT_TYPES == {'txt', 'md', 'html', 'htm'}


# --- Upload integration tests ---


def test_upload_txt_accepted():
    """Upload .txt via API → accepted (202)."""
    files = {"file": ("hello.txt", io.BytesIO(b"Hello world"), "text/plain")}
    response = client.post("/api/datasets/upload", files=files)
    assert response.status_code == 202
    assert response.json()["filename"] == "hello.txt"


def test_upload_md_accepted():
    """Upload .md via API → accepted (202)."""
    files = {"file": ("readme.md", io.BytesIO(b"# Title\nContent"), "text/markdown")}
    response = client.post("/api/datasets/upload", files=files)
    assert response.status_code == 202
    assert response.json()["filename"] == "readme.md"


def test_upload_html_accepted():
    """Upload .html via API → accepted (202)."""
    files = {"file": ("page.html", io.BytesIO(b"<html><body>Hi</body></html>"), "text/html")}
    response = client.post("/api/datasets/upload", files=files)
    assert response.status_code == 202
    assert response.json()["filename"] == "page.html"
