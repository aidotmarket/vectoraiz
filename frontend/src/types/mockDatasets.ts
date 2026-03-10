/**
 * Dataset types used across the frontend.
 * Originally held mock data — now just the type definitions.
 */

export interface ColumnSchema {
  name: string;
  dataType: "string" | "number" | "date" | "boolean" | "object" | "array";
  nonNullCount: number;
  nullPercentage: number;
  sampleValues: string[];
}

export interface Dataset {
  id: string;
  name: string;
  type: "csv" | "xlsx" | "json" | "pdf" | "parquet";
  status: "ready" | "processing" | "error" | "preview_ready";
  rows: number;
  columns: number;
  size: string;
  sizeBytes: number;
  createdAt: Date;
  modifiedAt: Date;
  processingTime: number;
  marketplace?: {
    isPublished?: boolean;
    price?: number;
    views?: number;
    purchases?: number;
    earnings?: number;
  };
}
