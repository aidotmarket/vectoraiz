"""
Search service that orchestrates semantic search across datasets.
"""

from typing import Optional, List, Dict, Any
from datetime import datetime

from app.services.embedding_service import get_embedding_service, EmbeddingService
from app.services.qdrant_service import get_qdrant_service, QdrantService
from app.services.processing_service import get_processing_service, ProcessingService


class SearchService:
    """
    Handles semantic search queries across indexed datasets.
    """
    
    def __init__(self):
        self.embedding_service: EmbeddingService = get_embedding_service()
        self.qdrant_service: QdrantService = get_qdrant_service()
        self.processing_service: ProcessingService = get_processing_service()
    
    def search(
        self,
        query: str,
        dataset_id: Optional[str] = None,
        limit: int = 10,
        min_score: Optional[float] = None,
    ) -> Dict[str, Any]:
        """
        Perform semantic search.
        
        Args:
            query: Natural language search query
            dataset_id: Search within specific dataset (None = search all)
            limit: Maximum results to return
            min_score: Minimum relevance score threshold (0-1)
            
        Returns:
            Search results with metadata
        """
        start_time = datetime.utcnow()
        
        if not query.strip():
            return {
                "query": query,
                "results": [],
                "total": 0,
                "message": "Empty query"
            }
        
        # Generate query embedding
        query_vector = self.embedding_service.embed_text(query)
        
        # Determine which collections to search
        if dataset_id:
            collections = [f"dataset_{dataset_id}"]
        else:
            collections = self._get_searchable_collections()
        
        if not collections:
            return {
                "query": query,
                "results": [],
                "total": 0,
                "message": "No indexed datasets available"
            }
        
        # Search across collections
        all_results = []
        
        for collection_name in collections:
            try:
                results = self.qdrant_service.search(
                    collection_name=collection_name,
                    query_vector=query_vector,
                    limit=limit,
                    score_threshold=min_score,
                )
                
                # Extract dataset_id from collection name
                ds_id = collection_name.replace("dataset_", "")
                
                # Get dataset info
                dataset_info = self._get_dataset_info(ds_id)
                
                for result in results:
                    all_results.append({
                        "dataset_id": ds_id,
                        "dataset_name": dataset_info.get("filename", ds_id),
                        "score": round(result["score"], 4),
                        "row_index": result["payload"].get("row_index"),
                        "text_content": result["payload"].get("text_content"),
                        "row_data": result["payload"].get("row_data", {}),
                    })
            except Exception as e:
                # Skip collections that fail (may have been deleted)
                print(f"Warning: Search failed for {collection_name}: {e}")
                continue
        
        # Sort by score descending and limit
        all_results.sort(key=lambda x: x["score"], reverse=True)
        all_results = all_results[:limit]
        
        end_time = datetime.utcnow()
        duration_ms = (end_time - start_time).total_seconds() * 1000
        
        return {
            "query": query,
            "results": all_results,
            "total": len(all_results),
            "datasets_searched": len(collections),
            "duration_ms": round(duration_ms, 2),
        }
    
    def search_dataset(
        self,
        dataset_id: str,
        query: str,
        limit: int = 10,
        min_score: Optional[float] = None,
    ) -> Dict[str, Any]:
        """
        Search within a specific dataset.
        
        Convenience method that ensures the dataset exists before searching.
        """
        # Verify dataset exists
        record = self.processing_service.get_dataset(dataset_id)
        if not record:
            raise ValueError(f"Dataset '{dataset_id}' not found")
        
        # Check if indexed
        collection_name = f"dataset_{dataset_id}"
        if not self.qdrant_service.collection_exists(collection_name):
            raise ValueError(f"Dataset '{dataset_id}' is not indexed for search")
        
        return self.search(
            query=query,
            dataset_id=dataset_id,
            limit=limit,
            min_score=min_score,
        )
    
    def _get_searchable_collections(self) -> List[str]:
        """Get all dataset collections available for search."""
        collections = self.qdrant_service.list_collections()
        return [
            c["name"] for c in collections 
            if c["name"].startswith("dataset_") and (c.get("vectors_count", 0) > 0 or c.get("points_count", 0) > 0)
        ]
    
    def _get_dataset_info(self, dataset_id: str) -> Dict[str, Any]:
        """Get basic dataset info for search results."""
        record = self.processing_service.get_dataset(dataset_id)
        if record:
            return {
                "filename": record.original_filename,
                "file_type": record.file_type,
            }
        return {}
    
    def get_search_stats(self) -> Dict[str, Any]:
        """Get statistics about searchable datasets."""
        collections = self._get_searchable_collections()
        
        total_vectors = 0
        datasets = []
        
        for collection_name in collections:
            try:
                info = self.qdrant_service.get_collection_info(collection_name)
                dataset_id = collection_name.replace("dataset_", "")
                dataset_info = self._get_dataset_info(dataset_id)
                
                datasets.append({
                    "dataset_id": dataset_id,
                    "filename": dataset_info.get("filename", dataset_id),
                    "vectors_count": info.get("vectors_count", 0),
                })
                total_vectors += info.get("vectors_count", 0)
            except Exception:
                continue
        
        return {
            "total_datasets": len(datasets),
            "total_vectors": total_vectors,
            "datasets": datasets,
        }


# Singleton instance
_search_service: Optional[SearchService] = None


def get_search_service() -> SearchService:
    """Get the singleton search service instance."""
    global _search_service
    if _search_service is None:
        _search_service = SearchService()
    return _search_service
