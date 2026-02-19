"""
Pydantic Schemas for Quality Attestation Reports (BQ-061)
"""
from __future__ import annotations

from pydantic import BaseModel, Field
from typing import List, Optional


class ColumnMetrics(BaseModel):
    column_name: str
    null_ratio: float = Field(..., description="Ratio of null values in this column.")
    type_consistency: float = Field(1.0, description="Ratio of values matching the inferred type.")


class DataProfileSummary(BaseModel):
    row_count: int = 0
    column_count: int = 0
    size_bytes: int = 0
    file_format: str = ""
    columns: List[ColumnMetrics] = Field(default_factory=list)


class PIIRiskAssessment(BaseModel):
    overall_risk: str = Field("none", description="none, low, medium, high")
    pii_entities_found: List[str] = Field(default_factory=list)
    total_pii_findings: int = 0


class ComplianceStatus(BaseModel):
    compliance_score: int = Field(100, description="0-100, 100 = fully compliant")
    status: str = Field("not_checked", description="low_risk, medium_risk, high_risk, not_checked")
    flagged_regulations: List[str] = Field(default_factory=list)


class QualityScores(BaseModel):
    completeness: float = Field(..., description="0.0-1.0 based on null ratio.")
    consistency: float = Field(..., description="0.0-1.0 based on type uniformity.")
    freshness: float = Field(0.0, description="0.0-1.0 based on date column recency.")


class QualityAttestation(BaseModel):
    data_hash: str = Field(..., description="SHA-256 hash of the processed data file.")
    attestation_hash: str = Field("", description="SHA-256 of the report JSON for integrity verification.")
    row_count: int = Field(..., description="Total number of rows.")
    column_count: int = Field(..., description="Total number of columns.")
    completeness_score: float = Field(..., description="Overall data completeness score (0.0 to 1.0).")
    type_consistency_score: float = Field(..., description="Overall type consistency score (0.0 to 1.0).")
    freshness_score: float = Field(0.0, description="Date recency score (0.0 to 1.0).")
    null_ratio_per_column: List[ColumnMetrics] = Field(..., description="Null value ratios for each column.")
    quality_grade: str = Field(..., description="Overall quality grade (A, B, C, D, F).")
    generated_at: str = Field(..., description="ISO 8601 timestamp of when the attestation was generated.")

    # BQ-061 AC2: Report sections
    data_profile: Optional[DataProfileSummary] = None
    pii_risk: Optional[PIIRiskAssessment] = None
    compliance: Optional[ComplianceStatus] = None
    quality_scores: Optional[QualityScores] = None
