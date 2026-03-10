/**
 * ToolResultRenderer — Renders TOOL_RESULT data inline in chat messages.
 *
 * Handles different tool types:
 * - preview_rows / run_sql_query: styled data table
 * - list_datasets: formatted dataset list
 * - get_dataset_detail / get_dataset_statistics: detail cards
 * - search_vectors: search results list
 * - get_system_status: status card
 * - errors: error message
 *
 * PHASE: BQ-ALLAI-B6 — Frontend Tool Result Rendering
 * CREATED: 2026-02-16
 */

import { memo } from "react";
import type { ToolResultData } from "@/contexts/CoPilotContext";

interface ToolResultRendererProps {
  toolName: string;
  data: ToolResultData;
}

export default memo(function ToolResultRenderer({ toolName, data }: ToolResultRendererProps) {
  if (data.error) {
    return (
      <div className="my-2 rounded-lg border border-red-500/20 bg-red-500/5 px-3 py-2 text-xs text-red-700">
        <span className="font-medium">Error:</span> {data.error}
      </div>
    );
  }

  switch (toolName) {
    case "preview_rows":
    case "run_sql_query":
      return <DataTable data={data} />;
    case "list_datasets":
      return <DatasetList data={data} />;
    case "get_dataset_detail":
      return <DatasetDetail data={data} />;
    case "get_dataset_statistics":
      return <StatisticsTable data={data} />;
    case "search_vectors":
      return <SearchResults data={data} />;
    case "get_system_status":
      return <SystemStatus data={data} />;
    case "create_artifact":
    case "create_artifact_from_query":
      return <ArtifactCard data={data} />;
    default:
      return (
        <div className="my-2 rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          <pre className="whitespace-pre-wrap">{JSON.stringify(data, null, 2)}</pre>
        </div>
      );
  }
});

function DataTable({ data }: { data: ToolResultData }) {
  const columns = data.columns as string[] | undefined;
  const rows = data.rows as Record<string, unknown>[] | undefined;
  if (!columns || !rows || rows.length === 0) {
    return (
      <div className="my-2 text-xs text-muted-foreground italic">No data returned.</div>
    );
  }

  return (
    <div className="my-2 overflow-x-auto rounded-lg border border-border">
      {data.query && (
        <div className="bg-gray-900 text-gray-400 px-3 py-1.5 text-[10px] font-mono border-b border-gray-800 truncate">
          {data.query as string}
        </div>
      )}
      <table className="min-w-full text-xs">
        <thead>
          <tr className="bg-muted">
            {columns.map((col) => (
              <th key={col} className="px-2 py-1.5 text-left font-medium text-muted-foreground border-b border-border whitespace-nowrap">
                {col}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, i) => (
            <tr key={i} className={i % 2 === 0 ? "" : "bg-muted/30"}>
              {columns.map((col) => (
                <td key={col} className="px-2 py-1 text-foreground/70 border-b border-border/50 whitespace-nowrap max-w-[200px] truncate">
                  {row[col] === null ? <span className="text-muted-foreground/40 italic">null</span> : String(row[col])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <div className="bg-muted/30 px-3 py-1 text-[10px] text-muted-foreground/60">
        {rows.length} row{rows.length !== 1 ? "s" : ""}
        {data.truncated ? " (truncated)" : ""}
      </div>
    </div>
  );
}

function DatasetList({ data }: { data: ToolResultData }) {
  const datasets = data.datasets as Array<Record<string, unknown>> | undefined;
  if (!datasets || datasets.length === 0) {
    return <div className="my-2 text-xs text-muted-foreground italic">No datasets found.</div>;
  }

  return (
    <div className="my-2 space-y-1">
      {datasets.map((ds) => (
        <div key={ds.id as string} className="flex items-center gap-2 rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs">
          <div className="flex-1 min-w-0">
            <div className="font-medium text-foreground/80 truncate">{ds.filename as string}</div>
            <div className="text-muted-foreground">
              {ds.rows ? `${ds.rows} rows` : ""}
              {ds.rows && ds.columns ? " · " : ""}
              {ds.columns ? `${ds.columns} cols` : ""}
              {" · "}
              <span className={
                ds.status === "ready" ? "text-green-600" :
                ds.status === "error" ? "text-red-600" :
                "text-yellow-600"
              }>
                {ds.status as string}
              </span>
            </div>
          </div>
          <div className="text-[10px] text-muted-foreground/40 font-mono">{(ds.id as string).slice(0, 8)}</div>
        </div>
      ))}
    </div>
  );
}

function DatasetDetail({ data }: { data: ToolResultData }) {
  return (
    <div className="my-2 rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs space-y-1">
      <div className="font-medium text-foreground/80">{data.filename as string}</div>
      <div className="text-muted-foreground">
        {data.rows ? `${data.rows} rows` : ""} · {data.columns ? `${data.columns} columns` : ""} · {data.status as string}
      </div>
      {data.column_names && (
        <div className="text-muted-foreground/60 text-[10px]">
          Columns: {(data.column_names as string[]).join(", ")}
        </div>
      )}
    </div>
  );
}

function StatisticsTable({ data }: { data: ToolResultData }) {
  const stats = data.statistics as Array<Record<string, unknown>> | undefined;
  if (!stats || stats.length === 0) {
    return <div className="my-2 text-xs text-muted-foreground italic">No statistics available.</div>;
  }

  const keys = Object.keys(stats[0]);

  return (
    <div className="my-2 overflow-x-auto rounded-lg border border-border">
      <table className="min-w-full text-xs">
        <thead>
          <tr className="bg-muted">
            {keys.map((key) => (
              <th key={key} className="px-2 py-1.5 text-left font-medium text-muted-foreground border-b border-border whitespace-nowrap">
                {key}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {stats.map((row, i) => (
            <tr key={i} className={i % 2 === 0 ? "" : "bg-muted/30"}>
              {keys.map((key) => (
                <td key={key} className="px-2 py-1 text-foreground/70 border-b border-border/50 whitespace-nowrap max-w-[150px] truncate">
                  {row[key] === null ? <span className="text-muted-foreground/40 italic">null</span> : String(row[key])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SearchResults({ data }: { data: ToolResultData }) {
  const results = data.results as Array<Record<string, unknown>> | undefined;
  if (!results || results.length === 0) {
    return <div className="my-2 text-xs text-muted-foreground italic">No search results found.</div>;
  }

  return (
    <div className="my-2 space-y-1">
      {results.map((r, i) => (
        <div key={i} className="rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs">
          <div className="flex items-center gap-2">
            <span className="text-primary/60 font-mono text-[10px]">{((r.score as number) * 100).toFixed(1)}%</span>
            <span className="text-muted-foreground">{r.dataset_name as string}</span>
          </div>
          {r.text_content && (
            <div className="mt-1 text-foreground/70 line-clamp-2">{r.text_content as string}</div>
          )}
        </div>
      ))}
    </div>
  );
}

function ArtifactCard({ data }: { data: ToolResultData }) {
  const filename = data.filename as string | undefined;
  const format = data.format as string | undefined;
  const sizeBytes = data.size_bytes as number | undefined;
  const description = data.description as string | undefined;
  const artifactId = data.artifact_id as string | undefined;

  const fmtSize = (b: number) => {
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / (1024 * 1024)).toFixed(1)} MB`;
  };

  const handleDownload = () => {
    if (!artifactId) return;
    const apiUrl = typeof window !== "undefined" ? localStorage.getItem("vectoraiz_api_url") || "" : "";
    const url = `${apiUrl}/api/artifacts/${artifactId}/download`;
    const apiKey = localStorage.getItem("vectoraiz_api_key");
    fetch(url, { headers: apiKey ? { "X-API-Key": apiKey } : {} })
      .then((r) => r.blob())
      .then((blob) => {
        const a = document.createElement("a");
        a.href = URL.createObjectURL(blob);
        a.download = filename || "download";
        document.body.appendChild(a);
        a.click();
        a.remove();
        URL.revokeObjectURL(a.href);
      });
  };

  return (
    <div className="my-2 rounded-lg border border-primary/20 bg-primary/5 px-3 py-2.5 text-xs">
      <div className="flex items-center gap-2">
        <span className="text-base">&#128196;</span>
        <span className="font-medium text-foreground/80">{filename || "artifact"}</span>
      </div>
      {description && (
        <div className="mt-1 text-muted-foreground text-[11px]">{description}</div>
      )}
      <div className="mt-1.5 flex items-center gap-3 text-[10px] text-muted-foreground/60">
        {sizeBytes != null && <span>{fmtSize(sizeBytes)}</span>}
        {format && <span className="uppercase">{format}</span>}
      </div>
      <div className="mt-2 flex items-center gap-2">
        <button
          onClick={handleDownload}
          className="px-2.5 py-1 rounded bg-primary/20 text-primary text-[11px] font-medium hover:bg-primary/30 transition-colors"
        >
          Download
        </button>
        <a
          href="/artifacts"
          className="px-2.5 py-1 rounded bg-muted text-muted-foreground text-[11px] font-medium hover:bg-muted/80 transition-colors"
        >
          View in Artifacts
        </a>
      </div>
    </div>
  );
}

function SystemStatus({ data }: { data: ToolResultData }) {
  return (
    <div className="my-2 rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs space-y-1">
      {Object.entries(data).map(([key, value]) => (
        <div key={key} className="flex justify-between">
          <span className="text-muted-foreground">{key}</span>
          <span className={
            value === "healthy" ? "text-green-600" :
            String(value).startsWith("error") ? "text-red-600" :
            "text-foreground/70"
          }>
            {String(value)}
          </span>
        </div>
      ))}
    </div>
  );
}
