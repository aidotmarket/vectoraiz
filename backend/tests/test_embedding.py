import pytest
from app.services.embedding_service import EmbeddingService, VECTOR_SIZE


@pytest.fixture
def embedding_service():
    """Create embedding service instance."""
    return EmbeddingService()


def test_embed_single_text(embedding_service):
    """Test single text embedding."""
    text = "This is a test sentence for embedding."
    vector = embedding_service.embed_text(text)
    
    assert isinstance(vector, list)
    assert len(vector) == VECTOR_SIZE  # 384 dimensions
    assert all(isinstance(x, float) for x in vector)


def test_embed_multiple_texts(embedding_service):
    """Test batch text embedding."""
    texts = [
        "First test sentence.",
        "Second test sentence.",
        "Third test sentence.",
    ]
    vectors = embedding_service.embed_texts(texts)
    
    assert len(vectors) == 3
    assert all(len(v) == VECTOR_SIZE for v in vectors)


def test_embed_empty_list(embedding_service):
    """Test embedding empty list returns empty."""
    vectors = embedding_service.embed_texts([])
    assert vectors == []


def test_similar_texts_have_similar_embeddings(embedding_service):
    """Test that semantically similar texts have similar embeddings."""
    import numpy as np
    
    text1 = "The quick brown fox jumps over the lazy dog."
    text2 = "A fast brown fox leaps over a sleepy dog."
    text3 = "Python is a programming language."
    
    vec1 = np.array(embedding_service.embed_text(text1))
    vec2 = np.array(embedding_service.embed_text(text2))
    vec3 = np.array(embedding_service.embed_text(text3))
    
    # Cosine similarity
    sim_1_2 = np.dot(vec1, vec2) / (np.linalg.norm(vec1) * np.linalg.norm(vec2))
    sim_1_3 = np.dot(vec1, vec3) / (np.linalg.norm(vec1) * np.linalg.norm(vec3))
    
    # Similar texts should have higher similarity
    assert sim_1_2 > sim_1_3


def test_model_info(embedding_service):
    """Test model info retrieval."""
    # Preload model
    embedding_service.preload()
    
    info = embedding_service.get_model_info()
    assert info["model_name"] == "all-MiniLM-L6-v2"
    assert info["vector_size"] == 384
    assert info["loaded"] == True
