import os
import sys
import asyncio

# Add current directory to path
sys.path.append(os.getcwd())

async def test_backend_init():
    try:
        from app.services.llm_providers.gemini import GeminiProvider
        
        print("Initializing Backend GeminiProvider with GCA manually set in env...")
        os.environ["VECTORAIZ_GOOGLE_GENAI_USE_GCA"] = "true"
        os.environ["VECTORAIZ_GEMINI_API_KEY"] = "fake-key"
        
        provider = GeminiProvider()
        print("✅ Initialized successfully!")
        
        info = provider.get_model_info()
        print(f"Model Info: {info}")
        
    except Exception as e:
        print(f"❌ Failed: {e}")
        import traceback
        traceback.print_exc()

if __name__ == "__main__":
    asyncio.run(test_backend_init())
