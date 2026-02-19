from typing import List, Dict, Any
from app.services.duckdb_service import get_duckdb_service

class QaaSAdvisorTools:
    """
    Tools for the allAI Advisor to help providers set up QaaS.
    """
    
    async def analyze_dataset_risks(self, dataset_id: str) -> Dict[str, Any]:
        """
        Analyze a dataset for QaaS risks (PII, value density).
        """
        duck = get_duckdb_service()
        metadata = duck.get_enhanced_metadata(dataset_id)
        
        # Logic to identify PII columns based on semantic_type
        pii_columns = [
            col['name'] for col in metadata['column_profiles'] 
            if col['semantic_type'] in ('email', 'phone', 'id')
        ]
        
        return {
            "dataset_id": dataset_id,
            "row_count": metadata['row_count'],
            "risk_score": len(pii_columns) * 10,  # Simple heuristic
            "pii_columns": pii_columns,
            "recommendation": "Exclude PII columns from Basic tiers." if pii_columns else "Low risk."
        }

    async def suggest_tiers(self, dataset_id: str) -> List[Dict[str, Any]]:
        """
        Suggest pricing tiers based on dataset value.
        """
        # In reality, this would use an LLM to generate suggestions based on metadata
        return [
            {
                "name": "Basic",
                "price_cents": 50,
                "description": "Aggregated stats only",
                "constraints": {"allow_aggregation": True, "max_rows": 0}
            },
            {
                "name": "Standard",
                "price_cents": 500,
                "description": "Up to 10 rows, no PII",
                "constraints": {"max_rows": 10, "exclude_columns": ["email", "phone"]}
            }
        ]
