"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { FormEvent, Suspense, useEffect, useMemo, useState } from "react";
import { type ColumnDef } from "@tanstack/react-table";
import { LoaderCircle, Search } from "lucide-react";

import { DataTable } from "@/components/data-table";
import { ExplainPanel } from "@/components/explain-panel";
import { QueryMetricsCard } from "@/components/query-metrics-card";
import { TableSearchInput } from "@/components/table-search-input";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { buildBm25SearchSql } from "@/lib/search";
import {
  runTableSearch,
  SEARCH_STRATEGIES,
  type ProvenanceRecord,
  type QueryMetricsResponse,
  type SearchHit,
} from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

type ResultRow = {
  id: number;
  score: number;
  text: string;
  metadata: string;
};

/** Tables above this use sparse-only + skip passage fetch by default. */
const LARGE_TABLE_ROWS = 500_000;

function SearchPageInner() {
  const searchParams = useSearchParams();
  const tables = usePlatformStore((s) => s.tables);
  const setSql = usePlatformStore((s) => s.setSql);
  const setSelectedTable = usePlatformStore((s) => s.setSelectedTable);

  const [table, setTable] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [topK, setTopK] = useState(10);
  const [offset, setOffset] = useState(0);
  const [strategy, setStrategy] = useState("sparse");
  const [fetchText, setFetchText] = useState(true);
  const [graphExpand, setGraphExpand] = useState(false);
  const [explain, setExplain] = useState(false);
  const [queryVectorJson, setQueryVectorJson] = useState("");
  const [resultFilter, setResultFilter] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [activeStrategy, setActiveStrategy] = useState<string | null>(null);
  const [explainText, setExplainText] = useState<string | null>(null);
  const [provenance, setProvenance] = useState<ProvenanceRecord | null>(null);
  const [metrics, setMetrics] = useState<QueryMetricsResponse | null>(null);
  const [latencyMs, setLatencyMs] = useState<number | null>(null);
  const [searchMs, setSearchMs] = useState<number | null>(null);
  const [fetchMs, setFetchMs] = useState<number | null>(null);

  const selectedTableInfo = tables.find((t) => t.name === table);
  const vectorDim = selectedTableInfo?.vector_dim ?? 0;
  const isLargeTable = (selectedTableInfo?.rows ?? 0) >= LARGE_TABLE_ROWS;

  useEffect(() => {
    if (!selectedTableInfo) return;
    if (selectedTableInfo.rows >= LARGE_TABLE_ROWS) {
      setStrategy("sparse");
    }
  }, [selectedTableInfo?.name, selectedTableInfo?.rows]);

  useEffect(() => {
    const fromUrl = searchParams.get("table");
    if (fromUrl) {
      setTable(fromUrl);
      setSelectedTable(fromUrl);
      return;
    }
    if (!table && tables[0]) {
      setTable(tables[0].name);
    }
  }, [searchParams, tables, table, setSelectedTable]);

  async function runSearch(ev?: FormEvent) {
    ev?.preventDefault();
    if (!table.trim() || !searchQuery.trim()) return;

    setLoading(true);
    setError("");
    try {
      let queryVector: number[] | undefined;
      if (queryVectorJson.trim()) {
        const parsed = JSON.parse(queryVectorJson) as unknown;
        if (!Array.isArray(parsed)) throw new Error("query_vector must be a JSON array");
        queryVector = parsed.map((n) => Number(n));
        if (vectorDim > 0 && queryVector.length !== vectorDim) {
          throw new Error(`query_vector must have ${vectorDim} dimensions`);
        }
      }

      const data = await runTableSearch({
        table,
        query: searchQuery,
        top_k: topK,
        offset,
        strategy: strategy || null,
        explain,
        graph_expand: graphExpand,
        query_vector: queryVector,
        fetch_text: fetchText,
      });
      setHits(data.hits);
      setActiveStrategy(data.strategy);
      setExplainText(data.explain);
      setProvenance(data.provenance ?? null);
      setMetrics(data.metrics);
      setLatencyMs(data.latency_ms);
      setSearchMs(data.search_ms);
      setFetchMs(data.fetch_ms);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setHits([]);
      setExplainText(null);
      setProvenance(null);
      setMetrics(null);
      setLatencyMs(null);
      setSearchMs(null);
      setFetchMs(null);
    } finally {
      setLoading(false);
    }
  }

  const rows = useMemo<ResultRow[]>(
    () =>
      hits.map((h) => ({
        id: h.id,
        score: h.score,
        text:
          h.text ??
          (fetchText ? "(text not found for this id)" : "(enable Load passage text)"),
        metadata: Object.keys(h.metadata).length
          ? JSON.stringify(h.metadata)
          : "",
      })),
    [hits, fetchText],
  );

  const filteredRows = useMemo(() => {
    const q = resultFilter.trim().toLowerCase();
    if (!q) return rows;
    return rows.filter((row) =>
      [row.id, row.score, row.text, row.metadata].some((v) =>
        String(v).toLowerCase().includes(q),
      ),
    );
  }, [rows, resultFilter]);

  const columns = useMemo<ColumnDef<ResultRow>[]>(
    () => [
      { accessorKey: "id", header: "id" },
      {
        accessorKey: "score",
        header: "score",
        cell: ({ row }) => Number(row.getValue("score")).toFixed(4),
      },
      {
        accessorKey: "text",
        header: "text",
        cell: ({ row }) => (
          <span className="block max-w-lg truncate text-xs">{String(row.getValue("text"))}</span>
        ),
      },
      {
        accessorKey: "metadata",
        header: "metadata",
        cell: ({ row }) => (
          <span className="block max-w-xs truncate font-mono text-xs text-muted-foreground">
            {String(row.getValue("metadata") || "—")}
          </span>
        ),
      },
    ],
    [],
  );

  const equivalentSql = table && searchQuery.trim()
    ? buildBm25SearchSql({ table, query: searchQuery, limit: topK })
    : "";

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Document search</CardTitle>
          <CardDescription>
            Native <code className="text-xs">Table.search</code> retrieval · POST /api/search
          </CardDescription>
        </CardHeader>
        <CardContent>
          {isLargeTable && (
            <p className="mb-3 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
              Large corpus ({selectedTableInfo?.rows.toLocaleString()} rows): defaults to{" "}
              <strong>sparse</strong> BM25 over segment indexes. Keep &quot;Load passage text&quot;
              checked to see snippets (adds fetch time). Uncheck for id/score-only benchmarks, or
              raise segment workers via{" "}
              <code className="text-[11px]">ALTER TABLE … SET SEGMENT_WORKERS = 8</code>.
            </p>
          )}
          <form className="space-y-4" onSubmit={(e) => void runSearch(e)}>
            <div className="flex flex-wrap gap-3">
              <label className="text-sm">
                <span className="text-muted-foreground">Table</span>
                <select
                  className="mt-1 flex h-9 w-full min-w-[160px] rounded-md border border-input bg-background px-3 text-sm"
                  value={table}
                  onChange={(e) => {
                    setTable(e.target.value);
                    setSelectedTable(e.target.value);
                  }}
                >
                  {tables.length === 0 ? (
                    <option value="">No tables</option>
                  ) : (
                    tables.map((t) => (
                      <option key={t.name} value={t.name}>
                        {t.name} ({t.rows} rows)
                      </option>
                    ))
                  )}
                </select>
              </label>
              <label className="text-sm">
                <span className="text-muted-foreground">Strategy</span>
                <select
                  className="mt-1 flex h-9 min-w-[160px] rounded-md border border-input bg-background px-3 text-sm"
                  value={strategy}
                  onChange={(e) => setStrategy(e.target.value)}
                >
                  {SEARCH_STRATEGIES.map((s) => (
                    <option key={s.value || "auto"} value={s.value}>
                      {s.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="text-sm">
                <span className="text-muted-foreground">Top K</span>
                <Input
                  className="mt-1 w-20"
                  type="number"
                  min={1}
                  max={100}
                  value={topK}
                  onChange={(e) => setTopK(Number(e.target.value) || 10)}
                />
              </label>
              <label className="text-sm">
                <span className="text-muted-foreground">Offset</span>
                <Input
                  className="mt-1 w-20"
                  type="number"
                  min={0}
                  value={offset}
                  onChange={(e) => setOffset(Number(e.target.value) || 0)}
                />
              </label>
            </div>

            <label className="block text-sm">
              <span className="text-muted-foreground">Search query</span>
              <Input
                className="mt-1"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="e.g. database engine"
                autoFocus
              />
            </label>

            {vectorDim > 0 && (
              <label className="block text-sm">
                <span className="text-muted-foreground">
                  Query vector (optional JSON, dim {vectorDim}; omit for lexical proxy)
                </span>
                <Input
                  className="mt-1 font-mono text-xs"
                  value={queryVectorJson}
                  onChange={(e) => setQueryVectorJson(e.target.value)}
                  placeholder={`[${Array(Math.min(vectorDim, 4)).fill("0").join(", ")}…]`}
                />
              </label>
            )}

            <div className="flex flex-wrap items-center gap-4 text-sm">
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={fetchText}
                  onChange={(e) => setFetchText(e.target.checked)}
                />
                <span className="text-muted-foreground">Load passage text</span>
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={graphExpand}
                  onChange={(e) => setGraphExpand(e.target.checked)}
                />
                <span className="text-muted-foreground">Graph expand</span>
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={explain}
                  onChange={(e) => setExplain(e.target.checked)}
                />
                <span className="text-muted-foreground">Explain plan</span>
              </label>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <Button type="submit" disabled={loading || !table || !searchQuery.trim()}>
                {loading ? (
                  <LoaderCircle className="size-4 animate-spin" />
                ) : (
                  <Search className="size-4" />
                )}
                Search
              </Button>
              {equivalentSql && (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setSql(equivalentSql);
                    setSelectedTable(table);
                  }}
                  asChild
                >
                  <Link href="/query">Equivalent SQL</Link>
                </Button>
              )}
              {selectedTableInfo && (
                <Badge variant="outline">
                  {selectedTableInfo.rows} rows · {selectedTableInfo.state}
                  {vectorDim > 0 ? ` · dim ${vectorDim}` : ""}
                </Badge>
              )}
              {activeStrategy && (
                <Badge variant="secondary">strategy: {activeStrategy}</Badge>
              )}
              {searchMs != null && (
                <Badge variant="outline">search {Math.round(searchMs)} ms</Badge>
              )}
              {fetchMs != null && fetchText && (
                <Badge variant="outline">fetch {Math.round(fetchMs)} ms</Badge>
              )}
              {latencyMs != null && (
                <Badge variant="secondary">total {Math.round(latencyMs)} ms</Badge>
              )}
              {metrics != null && metrics.segments_scanned > 0 && (
                <Badge variant="outline">
                  {metrics.segments_scanned} segs · {metrics.segment_workers} workers
                </Badge>
              )}
            </div>
          </form>
        </CardContent>
      </Card>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/20 p-2 text-sm text-destructive-foreground">
          {error}
        </div>
      )}

      <ExplainPanel text={explainText} provenance={provenance} />
      <QueryMetricsCard metrics={metrics} />

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <div>
            <CardTitle>Results</CardTitle>
            <CardDescription>
              {filteredRows.length} hit{filteredRows.length === 1 ? "" : "s"}
            </CardDescription>
          </div>
        </CardHeader>
        <CardContent className="space-y-3">
          <TableSearchInput
            value={resultFilter}
            onChange={setResultFilter}
            placeholder="Filter results…"
          />
          <DataTable
            columns={columns}
            data={filteredRows}
            emptyMessage={loading ? "Searching…" : "Run a search to see results"}
            pageSize={25}
          />
        </CardContent>
      </Card>
    </div>
  );
}

export default function SearchPage() {
  return (
    <Suspense
      fallback={
        <div className="text-sm text-muted-foreground">Loading search…</div>
      }
    >
      <SearchPageInner />
    </Suspense>
  );
}
