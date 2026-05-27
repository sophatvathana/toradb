export type Health = {
  status: "ok" | "degraded";
  db_path: string;
  db_exists: boolean;
  tables: string[];
  indexing_tables: string[];
  toradb_importable: boolean;
  hint: string | null;
};

export type IndexStatus = {
  table: string;
  state: "building" | "ready" | "failed";
  phase?: string | null;
  segments_done?: number;
  segments_total?: number;
  message?: string | null;
  updated_unix_secs?: number | null;
};

export type TableInfo = {
  name: string;
  rows: number;
  describe: string | null;
};

export type SearchHit = {
  id: number;
  score: number;
  text: string | null;
  metadata: Record<string, string>;
};

export type SearchResponse = {
  table: string;
  query: string;
  strategy: string | null;
  hits: SearchHit[];
  explain: string | null;
  latency_ms: number;
  open_ms?: number;
  search_ms?: number;
  fetch_ms?: number;
  total_ms?: number;
};

export type SqlResult =
  | { kind: "frame"; columns: string[]; rows: Record<string, unknown>[]; latency_ms: number }
  | { kind: "message"; text: string; latency_ms: number };

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    headers: { "Content-Type": "application/json", ...init?.headers },
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    const detailObj =
      typeof body.detail === "object" && body.detail !== null ? body.detail : null;
    const detail =
      typeof body.detail === "string"
        ? body.detail
        : detailObj?.message ??
          detailObj?.hint ??
          detailObj?.error ??
          res.statusText;
    const err = new Error(detail || `HTTP ${res.status}`);
    if (res.status === 503 && detailObj?.error === "index_building") {
      (err as Error & { code: string }).code = "index_building";
    }
    throw err;
  }
  return res.json() as Promise<T>;
}

export const api = {
  health: () => json<Health>("/api/health"),
  indexStatus: (table: string) =>
    json<IndexStatus>(`/api/index-status?table=${encodeURIComponent(table)}`),
  tables: () => json<TableInfo[]>("/api/tables"),
  sampleQueries: (table: string) =>
    json<{ search: string[]; sql: string[] }>(`/api/tables/${encodeURIComponent(table)}/sample-queries`),
  search: (body: {
    table: string;
    query: string;
    top_k: number;
    strategy: string | null;
    explain: boolean;
    graph_expand: boolean;
  }) =>
    json<SearchResponse>("/api/search", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  sql: (query: string) =>
    json<SqlResult>("/api/sql", {
      method: "POST",
      body: JSON.stringify({ query }),
    }),
  cliHint: (table: string, query: string) =>
    json<{ query: string; sql: string }>(
      `/api/cli-hint?table=${encodeURIComponent(table)}&query=${encodeURIComponent(query)}`,
    ),
};
