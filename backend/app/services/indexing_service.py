"""
Indexing service that combines embedding and vector storage.
Handles automatic indexing of datasets for semantic search.
"""

from pathlib import Path
from typing import Optional, List, Dict, Any
from datetime import datetime, date
from uuid import UUID

from app.config import settings
from app.services.embedding_service import get_embedding_service, EmbeddingService
from app.services.qdrant_service import get_qdrant_service, QdrantService
from app.services.duckdb_service import get_duckdb_service, DuckDBService


# Indexing configuration
DEFAULT_ROW_LIMIT = 10000  # Max rows to index per dataset
DEFAULT_BATCH_SIZE = 32


class IndexingService:
    """
    Orchestrates dataset indexing: extracts text, generates embeddings, stores in Qdrant.
    """
    
    def __init__(self):
        self.embedding_service: EmbeddingService = get_embedding_service()
        self.qdrant_service: QdrantService = get_qdrant_service()
        self.duckdb_service: DuckDBService = get_duckdb_service()
    
    def index_dataset(
        self,
        dataset_id: str,
        filepath: Path,
        row_limit: int = DEFAULT_ROW_LIMIT,
        text_columns: Optional[List[str]] = None,
        recreate_collection: bool = False,
    ) -> Dict[str, Any]:
        """
        Index a dataset for semantic search.
        
        Args:
            dataset_id: Unique identifier for the dataset (used as collection name)
            filepath: Path to the Parquet file
            row_limit: Maximum rows to index (default: 10000)
            text_columns: Specific columns to use for text (auto-detect if None)
            recreate_collection: Delete existing collection first
            
        Returns:
            Indexing result with statistics
        """
        start_time = datetime.utcnow()
        
        # Create or get collection
        collection_name = f"dataset_{dataset_id}"
        self.qdrant_service.create_collection(
            collection_name, 
            recreate_if_exists=recreate_collection
        )
        
        # Get dataset metadata to identify text columns
        metadata = self.duckdb_service.get_file_metadata(filepath)
        
        # Auto-detect text columns if not specified
        if text_columns is None:
            text_columns = self._detect_text_columns(filepath)
        
        if not text_columns:
            return {
                "dataset_id": dataset_id,
                "status": "skipped",
                "reason": "No text columns found for indexing",
                "collection": collection_name,
            }
        
        # Extract rows for indexing
        rows = self._extract_rows(filepath, row_limit)
        
        if not rows:
            return {
                "dataset_id": dataset_id,
                "status": "skipped",
                "reason": "No rows to index",
                "collection": collection_name,
            }
        
        # Generate text representations and embeddings
        texts = []
        payloads = []
        
        filename = filepath.name

        for i, row in enumerate(rows):
            # Combine text columns into single string
            text_parts = []
            for col in text_columns:
                if col in row and row[col] is not None:
                    text_parts.append(f"{col}: {row[col]}")

            if text_parts:
                text = " | ".join(text_parts)
                texts.append(text)
                payloads.append({
                    "dataset_id": dataset_id,
                    "filename": filename,
                    "row_index": i,
                    "row_id": f"{dataset_id}:{i}",  # stable row identifier
                    "text_content": text,
                    "row_data": row,  # Store full row for retrieval
                })
        
        if not texts:
            return {
                "dataset_id": dataset_id,
                "status": "skipped",
                "reason": "No text content to index",
                "collection": collection_name,
            }
        
        # Generate embeddings in batches
        embeddings = self.embedding_service.embed_texts(
            texts, 
            batch_size=DEFAULT_BATCH_SIZE,
            show_progress=len(texts) > 100
        )
        
        # Store in Qdrant
        result = self.qdrant_service.upsert_vectors(
            collection_name=collection_name,
            vectors=embeddings,
            payloads=payloads,
        )
        
        end_time = datetime.utcnow()
        duration = (end_time - start_time).total_seconds()
        
        return {
            "dataset_id": dataset_id,
            "status": "completed",
            "collection": collection_name,
            "rows_indexed": result["upserted"],
            "text_columns_used": text_columns,
            "duration_seconds": round(duration, 2),
            "rows_per_second": round(result["upserted"] / duration, 1) if duration > 0 else 0,
        }
    
    def _detect_text_columns(self, filepath: Path) -> List[str]:
        """
        Auto-detect columns suitable for text search.
        Prefers text/varchar columns with reasonable content.
        """
        profiles = self.duckdb_service.get_column_profile(filepath)
        
        text_columns = []
        for profile in profiles:
            # Include text-like columns
            if profile["semantic_type"] in ["text", "email", "url"]:
                # Skip columns that are mostly unique (likely IDs)
                if profile["uniqueness_ratio"] < 0.95:
                    text_columns.append(profile["name"])
            
            # Include columns with "name", "description", "title" in name
            col_lower = profile["name"].lower()
            if any(x in col_lower for x in ["name", "description", "title", "content", "text", "comment", "note"]):
                if profile["name"] not in text_columns:
                    text_columns.append(profile["name"])
        
        return text_columns
    
    def _extract_rows(self, filepath: Path, limit: int) -> List[Dict[str, Any]]:
        """Extract rows from dataset for indexing."""
        file_type = self.duckdb_service.detect_file_type(filepath)
        read_func = self.duckdb_service.get_read_function(file_type, str(filepath))
        
        query = f"SELECT * FROM {read_func} LIMIT {limit}"
        result = self.duckdb_service.connection.execute(query).fetchall()
        
        # Get column names
        schema = self.duckdb_service.connection.execute(f"DESCRIBE SELECT * FROM {read_func}").fetchall()
        column_names = [row[0] for row in schema]
        
        # Convert to list of dicts
        rows = []
        for row in result:
            row_dict = {}
            for col_name, value in zip(column_names, row):
                row_dict[col_name] = self._serialize_value(value)
            rows.append(row_dict)

        return rows

    @staticmethod
    def _serialize_value(value: Any) -> Any:
        """Convert a single value to a JSON-serializable form with type preservation."""
        if value is None:
            return None
        if isinstance(value, (int, float, bool)):
            return value
        if isinstance(value, datetime):
            return {"__type__": "datetime", "value": value.isoformat()}
        if isinstance(value, date):
            return {"__type__": "date", "value": value.isoformat()}
        if isinstance(value, UUID):
            return {"__type__": "uuid", "value": str(value)}
        return str(value)
    
    def delete_dataset_index(self, dataset_id: str) -> bool:
        """Delete the vector index for a dataset."""
        collection_name = f"dataset_{dataset_id}"
        return self.qdrant_service.delete_collection(collection_name)
    
    def get_index_status(self, dataset_id: str) -> Dict[str, Any]:
        """Get indexing status for a dataset."""
        collection_name = f"dataset_{dataset_id}"
        
        if not self.qdrant_service.collection_exists(collection_name):
            return {
                "dataset_id": dataset_id,
                "indexed": False,
                "collection": None,
            }
        
        info = self.qdrant_service.get_collection_info(collection_name)
        return {
            "dataset_id": dataset_id,
            "indexed": True,
            "collection": collection_name,
            "vectors_count": info["vectors_count"],
            "status": info["status"],
        }


# Singleton instance
_indexing_service: Optional[IndexingService] = None


def get_indexing_service() -> IndexingService:
    """Get the singleton indexing service instance."""
    global _indexing_service
    if _indexing_service is None:
        _indexing_service = IndexingService()
    return _indexing_service
