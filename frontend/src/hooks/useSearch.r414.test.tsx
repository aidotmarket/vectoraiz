import { readFileSync } from 'node:fs';
import { createElement, StrictMode, useEffect, useRef, useState } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { transpileModule, ScriptTarget, ModuleKind } from 'typescript';

type Response = { results: string[]; total: number };
type Options = { dataset_id?: string; limit?: number };
type Search = (query: string, options?: Options) => Promise<Response | undefined>;
type State = {
  results: Response | null; loading: boolean; error: string | null;
  search: Search; reset: () => void;
};
const searchApi = { search: vi.fn<Search>() };
const writes: unknown[] = [];
// Real React state, with attempted writes recorded even after unmount. A frozen
// renderHook result alone cannot prove that unmounted completions were ignored.
function useObservedState<T>(initial: T) {
  const [value, setValue] = useState(initial);
  const set: Dispatch<SetStateAction<T>> = next => {
    writes.push(next);
    setValue(next);
  };
  return [value, set] as const;
}

// Compile only the complete verbatim hook behind fake import boundaries. Never
// import useApi or its API/app/SQL dependencies. The source path override is for
// running these identical tests against a private copy of the base source.
const source = readFileSync(process.env.R414_SOURCE_PATH ?? new URL('./useApi.ts', import.meta.url), 'utf8');
const start = source.indexOf('export function useSearch() {');
const end = source.indexOf('\n}\n', start) + 2;
if (start < 0 || end < start) throw new Error('Cannot extract complete useSearch');
const hook = source.slice(start, end);
const compiled = transpileModule(hook.replace('export function', 'function'), {
  compilerOptions: { target: ScriptTarget.ES2022, module: ModuleKind.ESNext },
}).outputText;
const useSearch = new Function('useState', 'useEffect', 'useRef', 'searchApi',
  `${compiled}\nreturn useSearch;`)(useObservedState, useEffect, useRef, searchApi) as () => State;

function deferred() {
  let resolve!: (value: Response) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<Response>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const response = (name: string): Response => ({ results: [name], total: 1 });
function begin(state: { current: State }, query: string, pending = deferred()) {
  searchApi.search.mockReturnValueOnce(pending.promise);
  let completion!: ReturnType<Search>;
  act(() => { completion = state.current.search(query); });
  return { ...pending, completion };
}
async function finish(request: ReturnType<typeof begin>, outcome: 'success' | 'error', name = 'A') {
  await act(async () => {
    if (outcome === 'success') request.resolve(response(name));
    else request.reject(new Error(`${name} failed`));
    await request.completion;
  });
}
function expectState(state: State, results: Response | null, loading = false, error: string | null = null) {
  expect({ results: state.results, loading: state.loading, error: state.error }).toEqual({ results, loading, error });
}

beforeEach(() => {
  searchApi.search.mockReset();
  writes.length = 0;
  vi.stubGlobal('fetch', vi.fn(() => { throw new Error('Unexpected network'); }));
  vi.stubGlobal('XMLHttpRequest', vi.fn(() => { throw new Error('Unexpected network'); }));
});
afterEach(() => {
  cleanup();
  expect(fetch).not.toHaveBeenCalled();
  expect(XMLHttpRequest).not.toHaveBeenCalled();
  vi.unstubAllGlobals();
});

it.each(['success', 'error'] as const)('old %s/finally cannot end a newer pending search', async outcome => {
  const { result } = renderHook(useSearch);
  const a = begin(result, 'A');
  const b = begin(result, 'B');
  const before = writes.length;
  await finish(a, outcome);
  expect(writes).toHaveLength(before);
  expectState(result.current, null, true);
  await finish(b, 'success', 'B');
  expectState(result.current, response('B'));
  expect(searchApi.search).toHaveBeenCalledTimes(2);
});

it.each([
  ['success', 'success'], ['error', 'success'],
  ['success', 'error'], ['error', 'error'],
] as const)('old %s/finally cannot change a newer completed %s', async (oldOutcome, newOutcome) => {
  const { result } = renderHook(useSearch);
  const a = begin(result, 'A');
  const b = begin(result, 'B');
  await finish(b, newOutcome, 'B');
  const before = writes.length;
  await finish(a, oldOutcome);
  expect(writes).toHaveLength(before);
  expectState(result.current, newOutcome === 'success' ? response('B') : null,
    false, newOutcome === 'error' ? 'B failed' : null);
});

it.each(['success', 'error'] as const)('reset clears state and retires late %s/finally', async outcome => {
  const { result } = renderHook(useSearch);
  await finish(begin(result, 'seed'), 'success', 'seed');
  const a = begin(result, 'A');
  act(() => result.current.reset());
  expectState(result.current, null);
  const before = writes.length;
  await finish(a, outcome);
  expect(writes).toHaveLength(before);
  expectState(result.current, null);
  await finish(begin(result, 'B'), 'success', 'B');
  expectState(result.current, response('B'));
});

it.each(['success', 'error'] as const)('blank input makes no request and retires late %s', async outcome => {
  const { result } = renderHook(useSearch);
  await finish(begin(result, 'seed'), 'success', 'seed');
  const a = begin(result, 'A');
  await act(async () => { expect(await result.current.search(' \t\n')).toBeUndefined(); });
  expect(searchApi.search).toHaveBeenCalledTimes(2);
  expectState(result.current, null);
  const before = writes.length;
  await finish(a, outcome);
  expect(writes).toHaveLength(before);
  expectState(result.current, null);
});

it('blank and reset clear an owning error without making another request', async () => {
  const { result } = renderHook(useSearch);
  expectState(result.current, null);
  expect(searchApi.search).not.toHaveBeenCalled();
  await finish(begin(result, 'A'), 'error');
  expectState(result.current, null, false, 'A failed');
  await act(async () => { await result.current.search(''); });
  expectState(result.current, null);
  await finish(begin(result, 'B'), 'error', 'B');
  act(() => result.current.reset());
  expectState(result.current, null);
  expect(searchApi.search).toHaveBeenCalledTimes(2);
});

it.each(['success', 'error'] as const)('unmount retires pending %s without attempted state writes', async outcome => {
  const { result, unmount } = renderHook(useSearch);
  const a = begin(result, 'A');
  unmount();
  const before = writes.length;
  await finish(a, outcome);
  expect(writes).toHaveLength(before);
  expect(searchApi.search).toHaveBeenCalledTimes(1);
});

it.each(['success', 'error'] as const)('StrictMode replay does not revive retired %s', async outcome => {
  const first = deferred();
  const second = deferred();
  searchApi.search.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const completions: ReturnType<Search>[] = [];
  const { result } = renderHook(() => {
    const state = useSearch();
    // Intentionally exercise mount-effect replay with the initial search closure.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    useEffect(() => { completions.push(state.search('effect query')); }, []);
    return state;
  }, { wrapper: ({ children }) => createElement(StrictMode, null, children) });
  // Two deliberate effect setups, with cleanup between, and no hook auto-retry.
  expect(searchApi.search).toHaveBeenCalledTimes(2);
  expect(completions).toHaveLength(2);
  const before = writes.length;
  await finish({ ...first, completion: completions[0] }, outcome);
  expect(writes).toHaveLength(before);
  expectState(result.current, null, true);
  await finish({ ...second, completion: completions[1] }, 'success', 'live');
  expectState(result.current, response('live'));
  expect(searchApi.search).toHaveBeenCalledTimes(2);
});

it('preserves exact query/options, success identity, and one call per intention', async () => {
  const { result } = renderHook(useSearch);
  const pending = deferred();
  const options = { dataset_id: 'synthetic-dataset', limit: 7 };
  const query = '  synthetic query  ';
  searchApi.search.mockReturnValueOnce(pending.promise);
  let completion!: ReturnType<Search>;
  act(() => { completion = result.current.search(query, options); });
  expectState(result.current, null, true);
  expect(searchApi.search).toHaveBeenCalledExactlyOnceWith(query, options);
  expect(searchApi.search.mock.calls[0][1]).toBe(options);
  const value = response('ordinary');
  await act(async () => { pending.resolve(value); expect(await completion).toBe(value); });
  expect(result.current.results).toBe(value);
  expectState(result.current, value);
  expect(searchApi.search).toHaveBeenCalledTimes(1);
});

it.each([new Error('visible failure'), 'non-Error failure'])('preserves owning failure and does not retry: %s', async failure => {
  const { result } = renderHook(useSearch);
  const pending = begin(result, 'query');
  await act(async () => { pending.reject(failure); expect(await pending.completion).toBeUndefined(); });
  expectState(result.current, null, false, failure instanceof Error ? failure.message : 'Search failed');
  expect(searchApi.search).toHaveBeenCalledExactlyOnceWith('query', undefined);
  act(() => result.current.reset());
  expectState(result.current, null);
  expect(searchApi.search).toHaveBeenCalledTimes(1);
});
