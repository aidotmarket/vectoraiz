import { useState, useEffect, useCallback } from 'react';
import { 
  datasetsApi, 
  searchApi, 
  sqlApi, 
  healthApi,
  type DatasetListResponse,
  type ApiDataset,
  type SearchResponse, 
  type SQLResponse,
  type SQLTablesResponse,
  type DatasetSampleResponse,
  type DatasetProfileResponse,
  type DatasetStatisticsResponse,
  type SearchStatsResponse,
  type UploadResponse,
} from '@/lib/api';

// Generic async state hook
function useAsyncState<T>(asyncFn: () => Promise<T>, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refetch = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await asyncFn();
      setData(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setLoading(false);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(() => {
    refetch();
  }, [refetch]);

  return { data, loading, error, refetch };
}

// Dataset hooks
export function useDatasets() {
  return useAsyncState<DatasetListResponse>(() => datasetsApi.list(), []);
}

export function useDataset(id: string) {
  return useAsyncState<ApiDataset>(() => datasetsApi.get(id), [id]);
}

export function useDatasetSample(id: string, limit = 10) {
  return useAsyncState<DatasetSampleResponse>(() => datasetsApi.getSample(id, limit), [id, limit]);
}

export function useDatasetProfile(id: string) {
  return useAsyncState<DatasetProfileResponse>(() => datasetsApi.getProfile(id), [id]);
}

export function useDatasetStatistics(id: string) {
  return useAsyncState<DatasetStatisticsResponse>(() => datasetsApi.getStatistics(id), [id]);
}

// Upload hook with progress
export function useUpload() {
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<UploadResponse | null>(null);

  const upload = async (file: File, options?: { allowDuplicate?: boolean }) => {
    setUploading(true);
    setError(null);
    setResult(null);
    
    try {
      const res = await datasetsApi.upload(file, options);
      setResult(res);
      return res;
    } catch (e) {
      const msg = e instanceof Error ? e.message : 'Upload failed';
      setError(msg);
      throw e;
    } finally {
      setUploading(false);
    }
  };

  const reset = () => {
    setUploading(false);
    setError(null);
    setResult(null);
  };

  return { upload, uploading, error, result, reset };
}

// Search hook
export function useSearch() {
  const [results, setResults] = useState<SearchResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const search = async (query: string, options?: { dataset_id?: string; limit?: number }) => {
    if (!query.trim()) {
      setResults(null);
      return;
    }
    
    setLoading(true);
    setError(null);
    
    try {
      const res = await searchApi.search(query, options);
      setResults(res);
      return res;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Search failed');
    } finally {
      setLoading(false);
    }
  };

  const reset = () => {
    setResults(null);
    setError(null);
    setLoading(false);
  };

  return { search, results, loading, error, reset };
}

// SQL hook
export function useSQLQuery() {
  const [results, setResults] = useState<SQLResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const execute = async (sql: string, options?: { dataset_id?: string; limit?: number }) => {
    setLoading(true);
    setError(null);
    
    try {
      const res = await sqlApi.query(sql, options);
      setResults(res);
      return res;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Query failed');
    } finally {
      setLoading(false);
    }
  };

  const validate = async (sql: string) => {
    try {
      return await sqlApi.validate(sql);
    } catch (e) {
      return { valid: false, error: e instanceof Error ? e.message : 'Validation failed', query: sql };
    }
  };

  const reset = () => {
    setResults(null);
    setError(null);
    setLoading(false);
  };

  return { execute, validate, results, loading, error, reset };
}

// SQL Tables hook
export function useSQLTables() {
  return useAsyncState<SQLTablesResponse>(() => sqlApi.tables(), []);
}

// Search stats hook
export function useSearchStats() {
  return useAsyncState<SearchStatsResponse>(() => searchApi.stats(), []);
}

// Polling hook for dataset status
export function useDatasetStatus(id: string, pollInterval = 2000) {
  const [status, setStatus] = useState<string>('unknown');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!id) return;

    let active = true;
    
    const poll = async () => {
      try {
        const res = await datasetsApi.getStatus(id);
        if (active) {
          setStatus(res.status);
          if (res.error) setError(res.error);
          
          // Stop polling when done
          if (res.status === 'ready' || res.status === 'error') {
            return;
          }
        }
      } catch (e) {
        if (active) setError(e instanceof Error ? e.message : 'Status check failed');
        return;
      }
      
      // Continue polling
      if (active) {
        setTimeout(poll, pollInterval);
      }
    };

    poll();

    return () => { active = false; };
  }, [id, pollInterval]);

  return { status, error };
}

// Backend connection status hook
export function useBackendConnection(pollInterval = 30000) {
  const [status, setStatus] = useState<'checking' | 'connected' | 'disconnected'>('checking');
  const [lastChecked, setLastChecked] = useState<Date | null>(null);

  const checkConnection = useCallback(async () => {
    setStatus('checking');
    try {
      await healthApi.check();
      setStatus('connected');
      setLastChecked(new Date());
      return true;
    } catch {
      setStatus('disconnected');
      setLastChecked(new Date());
      return false;
    }
  }, []);

  useEffect(() => {
    checkConnection();
    
    const interval = setInterval(checkConnection, pollInterval);
    return () => clearInterval(interval);
  }, [checkConnection, pollInterval]);

  return { status, lastChecked, checkConnection };
}

// Delete dataset hook
export function useDeleteDataset() {
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const deleteDataset = async (id: string) => {
    setDeleting(true);
    setError(null);
    
    try {
      await datasetsApi.delete(id);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Delete failed');
      return false;
    } finally {
      setDeleting(false);
    }
  };

  return { deleteDataset, deleting, error };
}
