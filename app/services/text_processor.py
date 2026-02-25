"""
Text file processor for .txt, .md, and .html formats.
Extracts text content with encoding detection and HTML tag stripping.
"""

from pathlib import Path
from typing import Dict, Any


class TextProcessor:
    """Processes plain text, Markdown, and HTML files."""

    # Encoding fallback chain: try each in order
    ENCODING_CHAIN = ['utf-8-sig', 'utf-8', 'latin-1']

    def process(self, filepath: Path) -> Dict[str, Any]:
        """
        Process a text file and return extracted content with metadata.

        Returns:
            {
                "text_content": str,
                "metadata": {
                    "char_count": int,
                    "line_count": int,
                    "processor": "text_processor",
                    "encoding": str,
                }
            }
        """
        ext = filepath.suffix.lower().strip()

        text, encoding = self._read_with_fallback(filepath)

        if ext in ('.html', '.htm'):
            text = self._extract_html_text(text)

        # .txt and .md: return raw content as-is

        return {
            "text_content": text,
            "metadata": {
                "char_count": len(text),
                "line_count": text.count('\n') + 1 if text else 0,
                "processor": "text_processor",
                "encoding": encoding,
            }
        }

    def _read_with_fallback(self, filepath: Path) -> tuple[str, str]:
        """Read file content with encoding fallback chain."""
        for encoding in self.ENCODING_CHAIN:
            try:
                text = filepath.read_text(encoding=encoding)
                return text, encoding
            except (UnicodeDecodeError, ValueError):
                continue

        # Final fallback: utf-8 with replacement characters
        text = filepath.read_text(encoding='utf-8', errors='replace')
        return text, 'utf-8 (replace)'

    def _extract_html_text(self, html: str) -> str:
        """Extract visible text from HTML, stripping script/style/nav/header/footer/aside."""
        from bs4 import BeautifulSoup

        soup = BeautifulSoup(html, 'html.parser')

        # Remove non-content tags
        for tag in soup.find_all(['script', 'style', 'nav', 'header', 'footer', 'aside']):
            tag.decompose()

        return soup.get_text(separator=' ', strip=True)
