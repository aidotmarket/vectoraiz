import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Current { pack_id: string; generation: string; review_hash: string }
interface Entry { location: string; kind: string; value: string | null; warning: boolean }
interface Review { metadata_inventory: Entry[]; projection_inventory?: Entry[]; warnings: string[]; exclusions: string[]; accepted_review_hash: string | null }
interface Source { source: string; artifact: string; format: string; supported: boolean; refusal?: string | null; projection_summary?: {mandatory_drops: Record<string, number>; hidden_locations: string[]; included_pdf_segments?: {invisible: number; off_page: number}; warning?: string} }
interface Sources { pack_id: string; sources: Source[]; protected_columns: string[] }
interface Built { current: Current; review: Review; source_mapping?: Record<string, string> }
const PAGE = 50;
const failure = (cause: unknown) => typeof cause === "object" && cause !== null && "reason" in cause ? String(cause.reason) : String(cause);
function editable(entry: Entry, protectedColumns: string[] | undefined) {
  const key = entry.location;
  return entry.value !== null && ((key.startsWith("projection/") && entry.kind.endsWith(": included")) || (entry.kind === "text" && (key === "README.md" || key === "examples/query.py" || key.startsWith("originals-0001.zip!document-")))
    || ((entry.kind === "Office text" || entry.kind === "Office typed value") && key.endsWith("/text") && !key.includes("}numLit[") && !key.includes("}numCache["))
    || (entry.kind === "Office attribute" && ["name", "displayName", "descr", "title", "tooltip", "errorTitle", "error", "promptTitle", "prompt", "text"].some(attribute => key.endsWith(`/@{}${attribute}`)))
    || key === "examples/query.json/query"
    || ((key.startsWith("documents.parquet/column/") || key.startsWith("chunks.parquet/column/")) && key.endsWith("/name") && protectedColumns !== undefined && !protectedColumns.includes(entry.value))
    || (key.startsWith("documents.parquet/id/") && key.endsWith("/column/2"))
    || (key.startsWith("chunks.parquet/id/") && key.endsWith("/column/4")));
}

function ProjectionSummary({summary}: {summary: NonNullable<Source["projection_summary"]>}) {
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const locations = summary.hidden_locations.filter(location => location.toLocaleLowerCase().includes(search.toLocaleLowerCase()));
  return <details><summary>Private extraction counts and hidden locations ({summary.hidden_locations.length} hidden nodes)</summary>
    <p>{Object.entries(summary.mandatory_drops).map(([category, count]) => `${category}: ${count}`).join("; ")}</p>
    {summary.included_pdf_segments && <p>Included PDF segments: invisible {summary.included_pdf_segments.invisible}; off-page {summary.included_pdf_segments.off_page}. {summary.warning}</p>}
    <label>Find hidden locations<input type="search" value={search} onChange={event => {setSearch(event.target.value); setPage(0);}} /></label>
    <ul>{locations.slice(page * PAGE, (page + 1) * PAGE).map(location => <li key={location}>{location}: hidden, omitted by default</li>)}</ul>
    <button disabled={page === 0} onClick={() => setPage(value => value - 1)}>Previous hidden locations</button>
    <button disabled={(page + 1) * PAGE >= locations.length} onClick={() => setPage(value => value + 1)}>More hidden locations</button>
  </details>;
}

export function ListingReview() {
  const [sources, setSources] = useState<Sources>();
  const [sourceSearch, setSourceSearch] = useState("");
  const [sourcePage, setSourcePage] = useState(0);
  const [excluded, setExcluded] = useState<string[]>([]);
  const [includeHidden, setIncludeHidden] = useState(false);
  const [privateProjection, setPrivateProjection] = useState(false);
  const [terms, setTerms] = useState("");
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [built, setBuilt] = useState<Built>();
  const [dirty, setDirty] = useState(true);
  const [confirmed, setConfirmed] = useState(false);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [warningPage, setWarningPage] = useState(0);
  const [editing, setEditing] = useState<{entry: Entry; index: number; value: string}>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const heading = useRef<HTMLHeadingElement>(null);
  const alive = useRef(true);
  const mutation = useRef(0);
  const invalidation = useRef<Promise<unknown>>(Promise.resolve());
  useLayoutEffect(() => { if (built) heading.current?.focus(); }, [built?.current.generation]);
  async function load() {
    setBusy(true); setError("");
    try { const result = await invoke<Sources>("listing_sources"); if (alive.current) setSources(result); }
    catch (cause) { if (alive.current) setError(failure(cause)); }
    finally { if (alive.current) setBusy(false); }
  }
  useEffect(() => { alive.current = true; void load(); return () => { alive.current = false; }; }, []);

  function changed() {
    mutation.current++; setDirty(true); setConfirmed(false); setMessage("Changes need a new preview. Previous acceptance is invalidated.");
    if (built && !dirty) {
      invalidation.current = invoke("listing_invalidate", {current: built.current}).catch(cause => {
        if (alive.current) setError(failure(cause));
      });
    }
  }
  async function rebuild() {
    if (!sources) return;
    const version = mutation.current;
    setBusy(true); setError(""); setConfirmed(false); setMessage("");
    try {
      await invalidation.current;
      const result = await invoke<Built>("listing_rebuild", {packId: sources.pack_id, changes: {edits, exclusions: excluded, identity_terms: terms.split(/\r?\n/).map(term => term.trim()).filter(Boolean), origin: built?.current ?? null, include_hidden: includeHidden}});
      if (!alive.current || version !== mutation.current) return;
      setBuilt(result); setDirty(false); setPage(0); setWarningPage(0); setSearch(""); setMessage("Preview rebuilt from delivered bytes. Review the full inventory before accepting.");
    } catch (cause) { if (alive.current) {setDirty(true); setError(failure(cause));} }
    finally { if (alive.current) setBusy(false); }
  }
  async function accept() {
    if (!built || dirty || !confirmed) return;
    const version = mutation.current;
    setBusy(true); setError("");
    try {
      const review = await invoke<Review>("listing_review", {current: built.current, accept: true});
      if (alive.current && version === mutation.current) { setBuilt({...built, review}); setMessage("This exact generation is accepted for transfer. Destination checks still apply."); }
    } catch (cause) { if (alive.current) { setDirty(true); setConfirmed(false); setError(failure(cause)); } }
    finally { if (alive.current) setBusy(false); }
  }
  const needle = search.toLocaleLowerCase();
  const entries = (privateProjection ? built?.review.projection_inventory : built?.review.metadata_inventory)?.filter(entry => `${entry.location} ${entry.kind} ${entry.value ?? "Unknown / absent"}`.toLocaleLowerCase().includes(needle)) ?? [];
  const warnings = built?.review.warnings.filter(warning => warning.toLocaleLowerCase().includes(needle)) ?? [];
  const sourceEntries = sources?.sources.filter(source => `${source.source} ${source.format} ${built?.source_mapping?.[source.source] ?? source.artifact} ${source.refusal ?? ""}`.toLocaleLowerCase().includes(sourceSearch.toLocaleLowerCase())) ?? [];
  const unsupported = sources?.sources.some(source => !source.supported && !excluded.includes(source.source));
  const allExcluded = sources?.sources.every(source => excluded.includes(source.source));
  return <section className="card stack" aria-labelledby="listing-review-title" aria-busy={busy}>
    <h2 id="listing-review-title">What your buyer receives</h2>
    <p>The seller attests: chunks, embeddings and metadata; no source files are delivered.</p>
    <p>This private review shows the actual delivered files and metadata. Source documents stay on this computer. Names inside the content can identify their source.</p>
    {error && <p role="alert">{error}</p>}
    {!sources && <button disabled={busy} onClick={() => void load()}>Retry loading documents</button>}
    {sources && <>
      <fieldset disabled={busy}><legend>Documents to include</legend>
        <p>Exclude unsupported source documents. These source names and their neutral-name mapping stay private.</p>
        <label>Find source documents<input type="search" value={sourceSearch} onChange={event => {setSourceSearch(event.target.value); setSourcePage(0);}} /></label>
        <p>{sourceEntries.length} source documents</p>
        {sourceEntries.slice(sourcePage * PAGE, (sourcePage + 1) * PAGE).map(source => <div key={source.source}><label>
          <input type="checkbox" checked={!excluded.includes(source.source)} onChange={event => {changed(); setExcluded(old => event.target.checked ? old.filter(value => value !== source.source) : [...old, source.source]);}} />
          Include {source.source} → {built?.source_mapping?.[source.source] ?? source.artifact} ({source.supported ? "supported" : `unsupported: ${source.refusal ?? "exclude or re-ingest"}`})
        </label><span> Source format: {source.format.toUpperCase()}</span>{source.projection_summary && <ProjectionSummary summary={source.projection_summary} />}</div>)}
        <button disabled={sourcePage === 0} onClick={() => setSourcePage(value => value - 1)}>Previous documents</button>
        <button disabled={(sourcePage + 1) * PAGE >= sourceEntries.length} onClick={() => setSourcePage(value => value + 1)}>More documents</button>
        <label><input type="checkbox" checked={includeHidden} onChange={event => {changed(); setIncludeHidden(event.target.checked);}} />Include supported hidden content (omitted by default). Rebuild and review its private locations before accepting.</label>
        <label>Private identity terms, one per line<textarea value={terms} onChange={event => {changed(); setTerms(event.target.value);}} /></label>
      </fieldset>
      {unsupported && <p>Unsupported source documents block rebuilding until you exclude them.</p>}
      <button disabled={busy || unsupported || allExcluded || !!editing} onClick={() => void rebuild()}>{busy ? "Working…" : "Rebuild purchaser preview"}</button>
    </>}
    {built && <>
      <h3 ref={heading} tabIndex={-1}>Delivered inventory</h3>
      <p>Generation {built.current.generation}. {built.review.metadata_inventory.length} complete values and locations; {built.review.warnings.length} warnings. Unknown measurements are shown explicitly.</p>
      {dirty && <p>Changes pending: rebuild before accepting or transferring.</p>}
      <label><input type="checkbox" checked={privateProjection} onChange={event => {setPrivateProjection(event.target.checked); setPage(0);}} />Show private projection nodes and omissions</label>
      {privateProjection && <p>Private source projection: inclusion status and mandatory omissions. These records are separate from the delivered inventory.</p>}
      <label>Search delivered inventory<input type="search" value={search} onChange={event => {setSearch(event.target.value); setPage(0); setWarningPage(0);}} /></label>
      <ul aria-label="Delivered warnings">{warnings.slice(warningPage * PAGE, (warningPage + 1) * PAGE).map((warning, index) => <li key={index}>Warning: {warning}</li>)}</ul>
      <button disabled={warningPage === 0} onClick={() => setWarningPage(value => value - 1)}>Previous warnings</button>
      <button disabled={(warningPage + 1) * PAGE >= warnings.length} onClick={() => setWarningPage(value => value + 1)}>More warnings</button>
      <p aria-live="polite">{entries.length} matching entries; {warnings.length} matching warnings. Page {page + 1} of {Math.max(1, Math.ceil(entries.length / PAGE))}.</p>
      <ol start={page * PAGE + 1}>
        {entries.slice(page * PAGE, (page + 1) * PAGE).map((entry, index) => <li key={`${entry.location}:${entry.kind}:${index}`}>
          <strong>{entry.location}</strong> <span>({entry.kind})</span>
          {entry.warning && <p>Warning: identity or confidential-looking content at this location. You may accept it or edit/exclude and rebuild.</p>}
          <pre tabIndex={0} style={{whiteSpace: "pre-wrap", overflowWrap: "anywhere"}}>{entry.value ?? "Unknown / absent"}</pre>
          {privateProjection && entry.location.startsWith("projection/") && entry.value !== null && <label><input type="checkbox" checked={!excluded.includes(entry.location)} disabled={busy} onChange={event => {changed(); setExcluded(old => event.target.checked ? old.filter(v => v !== entry.location) : [...old, entry.location]);}} />Retain this projection node (hidden content also requires inclusion above)</label>}
          {editable(entry, sources?.protected_columns) && <button id={`listing-edit-${index}`} disabled={busy} onClick={() => setEditing({entry, index, value: edits[entry.location] ?? entry.value ?? ""})}>Edit {entry.location}</button>}
        </li>)}
      </ol>
      <button disabled={page === 0} onClick={() => setPage(value => value - 1)}>Previous inventory page</button>
      <button disabled={(page + 1) * PAGE >= entries.length} onClick={() => setPage(value => value + 1)}>Next inventory page</button>
      {editing && <fieldset disabled={busy}><legend>Edit reviewed value at {editing.entry.location}</legend>
        <label>Replacement value<textarea autoFocus value={editing.value} onChange={event => setEditing({...editing, value: event.target.value})} /></label>
        <button onClick={() => {changed(); setEdits(old => ({...old, [editing.entry.location]: editing.value})); const index = editing.index; setEditing(undefined); document.getElementById(`listing-edit-${index}`)?.focus();}}>Apply change and require rebuild</button>
        <button onClick={() => {const index = editing.index; setEditing(undefined); document.getElementById(`listing-edit-${index}`)?.focus();}}>Cancel edit</button>
      </fieldset>}
      <label><input type="checkbox" checked={confirmed} disabled={busy || dirty || !!editing} onChange={event => setConfirmed(event.target.checked)} />I reviewed the complete purchaser inventory and accept its warnings and unknown values.</label>
      <button disabled={busy || dirty || !confirmed || !!editing || !!built.review.accepted_review_hash} onClick={() => void accept()}>Accept this exact generation</button>
      {built.review.accepted_review_hash && !dirty && <p>Accepted review: {built.review.accepted_review_hash}</p>}
    </>}
    <p role="status">{message}</p>
  </section>;
}
