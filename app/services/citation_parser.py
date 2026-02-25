"""
Citation Parser
===============

Parses LLM responses to extract and validate citations.
Maps citation markers [N] back to source chunks.

Phase: 3.V.4
Created: 2026-01-25
"""

import re
import logging
from typing import List, Set, Optional

from app.models.rag import ParsedRAGResponse, Citation, SourceChunk

logger = logging.getLogger(__name__)


class CitationParser:
    """
    Parses LLM responses to extract and validate citations.
    Expected format: [N] where N is the source index.
    """

    # Regex to find [1], [2], [3], etc.
    CITATION_PATTERN = re.compile(r'\[(\d+)\]')
    
    # Pattern to find multiple consecutive citations like [1][2][3]
    MULTI_CITATION_PATTERN = re.compile(r'(\[\d+\])+')

    def parse(
        self, 
        text: str, 
        source_chunks: List[SourceChunk]
    ) -> ParsedRAGResponse:
        """
        Parse the text for citations and map them to the provided source chunks.
        
        Args:
            text: The raw response string from the LLM
            source_chunks: The list of chunks provided in the prompt context
            
        Returns:
            ParsedRAGResponse with validated citations
        """
        citations: List[Citation] = []
        unique_indices: Set[int] = set()
        invalid_citations: Set[int] = set()
        
        # Create a lookup map for efficiency
        source_map = {chunk.index: chunk for chunk in source_chunks}
        max_index = max(source_map.keys()) if source_map else 0

        # Find all citation matches
        matches = self.CITATION_PATTERN.finditer(text)
        
        for match in matches:
            try:
                index = int(match.group(1))
                
                is_valid = index in source_map
                source = source_map.get(index)
                
                citation = Citation(
                    source_index=index,
                    is_valid=is_valid,
                    source=source
                )
                
                citations.append(citation)
                
                if is_valid:
                    unique_indices.add(index)
                else:
                    invalid_citations.add(index)
                    
            except ValueError:
                logger.warning(f"Failed to parse citation: {match.group(0)}")
                continue

        # Log warnings for invalid citations
        if invalid_citations:
            logger.warning(
                f"Found {len(invalid_citations)} invalid citations: {sorted(invalid_citations)}. "
                f"Valid range: 1-{max_index}"
            )

        # Collect unique valid sources (sorted by index)
        unique_sources = [source_map[i] for i in sorted(unique_indices)]

        return ParsedRAGResponse(
            answer=text,
            citations=citations,
            unique_sources_cited=unique_sources,
            chunks_retrieved=len(source_chunks)
        )

    def extract_citation_indices(self, text: str) -> List[int]:
        """
        Extract just the citation indices from text.
        
        Args:
            text: Text containing citations
            
        Returns:
            List of citation indices in order of appearance
        """
        indices = []
        for match in self.CITATION_PATTERN.finditer(text):
            try:
                indices.append(int(match.group(1)))
            except ValueError:
                continue
        return indices

    def has_citations(self, text: str) -> bool:
        """Check if text contains any citations."""
        return bool(self.CITATION_PATTERN.search(text))

    def count_citations(self, text: str) -> int:
        """Count total number of citations in text."""
        return len(self.CITATION_PATTERN.findall(text))

    def strip_citations(self, text: str) -> str:
        """Remove all citation markers from text."""
        return self.CITATION_PATTERN.sub('', text).strip()

    def validate_coverage(
        self, 
        text: str, 
        source_chunks: List[SourceChunk],
        min_coverage: float = 0.5
    ) -> dict:
        """
        Validate that the response adequately cites its sources.
        
        Args:
            text: The response text
            source_chunks: Available source chunks
            min_coverage: Minimum fraction of sources that should be cited
            
        Returns:
            Validation result dict
        """
        if not source_chunks:
            return {
                "valid": True,
                "coverage": 0.0,
                "cited_count": 0,
                "total_sources": 0,
                "uncited_sources": []
            }
        
        cited_indices = set(self.extract_citation_indices(text))
        source_indices = {chunk.index for chunk in source_chunks}
        
        cited_sources = cited_indices & source_indices
        uncited_sources = source_indices - cited_indices
        
        coverage = len(cited_sources) / len(source_indices) if source_indices else 0.0
        
        return {
            "valid": coverage >= min_coverage,
            "coverage": round(coverage, 2),
            "cited_count": len(cited_sources),
            "total_sources": len(source_indices),
            "uncited_sources": sorted(uncited_sources)
        }


# Singleton instance
_citation_parser: Optional[CitationParser] = None


def get_citation_parser() -> CitationParser:
    """Get the singleton citation parser instance."""
    global _citation_parser
    if _citation_parser is None:
        _citation_parser = CitationParser()
    return _citation_parser
