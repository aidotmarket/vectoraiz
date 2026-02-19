"""
Embedding service using sentence-transformers.
Loads model once at startup for efficient inference.
"""

from typing import Optional, List, Dict, Any
import time

from app.config import settings


# Model configuration
MODEL_NAME = "all-MiniLM-L6-v2"
VECTOR_SIZE = 384
DEFAULT_BATCH_SIZE = 32


class EmbeddingService:
    """
    Generates text embeddings using sentence-transformers.
    Model is loaded once and reused for all requests.
    """
    
    def __init__(self):
        self._model = None
        self._model_name = MODEL_NAME
        self._load_time: Optional[float] = None
    
    @property
    def model(self):
        """Lazy load the embedding model."""
        if self._model is None:
            self._load_model()
        return self._model
    
    def _load_model(self):
        """Load the sentence-transformers model."""
        from sentence_transformers import SentenceTransformer
        
        start_time = time.time()
        print(f"Loading embedding model: {self._model_name}...")
        
        self._model = SentenceTransformer(self._model_name)
        
        self._load_time = time.time() - start_time
        print(f"Model loaded in {self._load_time:.2f}s")
    
    def embed_text(self, text: str) -> List[float]:
        """
        Generate embedding for a single text string.
        
        Args:
            text: Input text to embed
            
        Returns:
            384-dimensional embedding vector
        """
        embedding = self.model.encode(text, convert_to_numpy=True)
        return embedding.tolist()
    
    def embed_texts(
        self, 
        texts: List[str], 
        batch_size: int = DEFAULT_BATCH_SIZE,
        show_progress: bool = False
    ) -> List[List[float]]:
        """
        Generate embeddings for multiple texts in batches.
        
        Args:
            texts: List of input texts
            batch_size: Number of texts to process at once (default: 32)
            show_progress: Show progress bar (for large batches)
            
        Returns:
            List of 384-dimensional embedding vectors
        """
        if not texts:
            return []
        
        embeddings = self.model.encode(
            texts,
            batch_size=batch_size,
            convert_to_numpy=True,
            show_progress_bar=show_progress,
        )
        
        return embeddings.tolist()
    
    def get_model_info(self) -> Dict[str, Any]:
        """Get information about the loaded model."""
        return {
            "model_name": self._model_name,
            "vector_size": VECTOR_SIZE,
            "loaded": self._model is not None,
            "load_time_seconds": self._load_time,
        }
    
    def preload(self):
        """Explicitly preload the model (call at startup)."""
        _ = self.model
        return self.get_model_info()


# Singleton instance
_embedding_service: Optional[EmbeddingService] = None


def get_embedding_service() -> EmbeddingService:
    """Get the singleton embedding service instance."""
    global _embedding_service
    if _embedding_service is None:
        _embedding_service = EmbeddingService()
    return _embedding_service
