"use client";

import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { FormEvent, Suspense, useEffect, useMemo, useState } from "react";
import { type ColumnDef } from "@tanstack/react-table";
import {
  ChevronRight,
  LoaderCircle,
  Plus,
  Search,
  SlidersHorizontal,
  Star,
  X,
} from "lucide-react";

import { DataTable } from "@/components/data-table";
import { ExplainPanel } from "@/components/explain-panel";
import { HighlightedSnippet } from "@/components/highlighted-snippet";
import { QueryMetricsCard } from "@/components/query-metrics-card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { buildBm25SearchSql } from "@/lib/search";
import {
  runTableSearch,
  SEARCH_STRATEGIES,
  type FacetGroup,
  type ProvenanceRecord,
  type QueryMetricsResponse,
  type SearchHit,
  type TableSearchRequest,
} from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

type ResultRow = {
  id: number;
  score: number;
  text: string;
  metadata: string;
};

type BoostRow = { field: string; factor: string };

/** Tables above this use sparse-only + skip passage fetch by default. */
const LARGE_TABLE_ROWS = 500_000;

function SearchPageInner() {
  const searchParams = useSearchParams();
  const tables = usePlatformStore((s) => s.tables);
  const setSql = usePlatformStore((s) => s.setSql);
  const setSelectedTable = usePlatformStore((s) => s.setSelectedTable);
  const savedSearches = usePlatformStore((s) => s.savedSearches);
  const addSavedSearch = usePlatformStore((s) => s.addSavedSearch);
  const removeSavedSearch = usePlatformStore((s) => s.removeSavedSearch);

  const [table, setTable] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [topK, setTopK] = useState(10);
  const [offset, setOffset] = useState(0);
  const [strategy, setStrategy] = useState("sparse");
  const [fetchText, setFetchText] = useState(true);
  const [graphExpand, setGraphExpand] = useState(false);
  const [explain, setExplain] = useState(false);
  const [queryVectorJson, setQueryVectorJson] = useState("");

  // Ranking knobs / parity controls.
  const [showTuning, setShowTuning] = useState(false);
  const [highlight, setHighlight] = useState(true);
  const [snippetLen, setSnippetLen] = useState(160);
  const [facetsInput, setFacetsInput] = useState("");
  const [k1, setK1] = useState("");
  const [b, setB] = useState("");
  const [boosts, setBoosts] = useState<BoostRow[]>([]);
  const [decayField, setDecayField] = useState("");
  const [decayHalfLife, setDecayHalfLife] = useState("");
  const [sparseJson, setSparseJson] = useState("");

  const [viewMode, setViewMode] = useState<"cards" | "table">("cards");
  const [resultFilter, setResultFilter] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [facets, setFacets] = useState<FacetGroup[]>([]);
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

  function buildRequest(): TableSearchRequest {
    let queryVector: number[] | undefined;
    if (queryVectorJson.trim()) {
      const parsed = JSON.parse(queryVectorJson) as unknown;
      if (!Array.isArray(parsed)) throw new Error("query_vector must be a JSON array");
      queryVector = parsed.map((n) => Number(n));
      if (vectorDim > 0 && queryVector.length !== vectorDim) {
        throw new Error(`query_vector must have ${vectorDim} dimensions`);
      }
    }

    let sparse: Record<string, number> | undefined;
    if (sparseJson.trim()) {
      const parsed = JSON.parse(sparseJson) as unknown;
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        throw new Error("sparse must be a JSON object of {token: weight}");
      }
      sparse = Object.fromEntries(
        Object.entries(parsed as Record<string, unknown>).map(([k, v]) => [k, Number(v)]),
      );
    }

    const facetFields = facetsInput
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);

    const boostMap: Record<string, number> = {};
    for (const row of boosts) {
      const f = row.field.trim();
      const factor = Number(row.factor);
      if (f && Number.isFinite(factor)) boostMap[f] = factor;
    }

    const decay: [string, number] | undefined =
      decayField.trim() && Number.isFinite(Number(decayHalfLife))
        ? [decayField.trim(), Number(decayHalfLife)]
        : undefined;

    return {
      table,
      query: searchQuery,
      top_k: topK,
      offset,
      strategy: strategy || null,
      explain,
      graph_expand: graphExpand,
      query_vector: queryVector,
      fetch_text: fetchText,
      highlight,
      snippet_len: highlight ? snippetLen : undefined,
      facets: facetFields.length ? facetFields : undefined,
      bm25_k1: k1.trim() ? Number(k1) : undefined,
      bm25_b: b.trim() ? Number(b) : undefined,
      boosts: Object.keys(boostMap).length ? boostMap : undefined,
      decay,
      sparse,
    };
  }

  /** Restore the full builder state from a saved request (for preset replay). */
  function applyRequest(req: TableSearchRequest) {
    setTable(req.table);
    setSelectedTable(req.table);
    setSearchQuery(req.query);
    setTopK(req.top_k ?? 10);
    setOffset(req.offset ?? 0);
    setStrategy(req.strategy ?? "sparse");
    setExplain(req.explain ?? false);
    setGraphExpand(req.graph_expand ?? false);
    setFetchText(req.fetch_text ?? true);
    setHighlight(req.highlight ?? false);
    setSnippetLen(req.snippet_len ?? 160);
    setFacetsInput((req.facets ?? []).join(", "));
    setK1(req.bm25_k1 != null ? String(req.bm25_k1) : "");
    setB(req.bm25_b != null ? String(req.bm25_b) : "");
    setBoosts(
      Object.entries(req.boosts ?? {}).map(([field, factor]) => ({
        field,
        factor: String(factor),
      })),
    );
    setDecayField(req.decay ? req.decay[0] : "");
    setDecayHalfLife(req.decay ? String(req.decay[1]) : "");
    setQueryVectorJson(req.query_vector ? JSON.stringify(req.query_vector) : "");
    setSparseJson(req.sparse ? JSON.stringify(req.sparse) : "");
    // Surface advanced knobs if the preset uses any.
    if (
      req.bm25_k1 != null ||
      req.bm25_b != null ||
      req.decay ||
      (req.boosts && Object.keys(req.boosts).length) ||
      (req.facets && req.facets.length) ||
      req.sparse
    ) {
      setShowTuning(true);
    }
  }

  async function runWith(req: TableSearchRequest) {
    if (!req.table.trim() || !req.query.trim()) return;
    setLoading(true);
    setError("");
    try {
      const data = await runTableSearch(req);
      setHits(data.hits);
      setFacets(data.facets ?? []);
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
      setFacets([]);
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

  async function runSearch(ev?: FormEvent) {
    ev?.preventDefault();
    if (!table.trim() || !searchQuery.trim()) return;
    let req: TableSearchRequest;
    try {
      req = buildRequest();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      return;
    }
    await runWith(req);
  }

  function runSavedSearch(req: TableSearchRequest) {
    applyRequest(req);
    void runWith(req);
  }

  const rows = useMemo<ResultRow[]>(
    () =>
      hits.map((h) => ({
        id: h.id,
        score: h.score,
        text:
          h.text ??
          (fetchText ? "(text not found for this id)" : "(enable Load passage text)"),
        metadata: Object.keys(h.metadata).length ? JSON.stringify(h.metadata) : "",
      })),
    [hits, fetchText],
  );

  const filteredHits = useMemo(() => {
    const q = resultFilter.trim().toLowerCase();
    if (!q) return hits;
    return hits.filter((h) =>
      [h.id, h.score, h.text ?? "", JSON.stringify(h.metadata)].some((v) =>
        String(v).toLowerCase().includes(q),
      ),
    );
  }, [hits, resultFilter]);

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

  const equivalentSql =
    table && searchQuery.trim()
      ? buildBm25SearchSql({ table, query: searchQuery, limit: topK })
      : "";

  function applyFacet(value: string) {
    setSearchQuery((q) => (q.includes(value) ? q : `${q} ${value}`.trim()));
  }

  function saveCurrentSearch() {
    if (!table.trim() || !searchQuery.trim()) return;
    let req: TableSearchRequest;
    try {
      req = buildRequest();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      return;
    }
    const name = window.prompt("Name this saved search:", searchQuery.slice(0, 40));
    if (!name?.trim()) return;
    addSavedSearch(name.trim(), req);
  }

  return (
    <div className="space-y-4">
      {/* Terminal-style query surface */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="font-mono text-sm tracking-tight text-muted-foreground">
            Table.search · <span className="text-foreground">POST /api/search</span>
          </CardTitle>
        </CardHeader>
        <CardContent>
          {isLargeTable && (
            <p className="mb-3 rounded-md border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
              Large corpus ({selectedTableInfo?.rows.toLocaleString()} rows): defaults to{" "}
              <strong>sparse</strong> BM25 over segment indexes. Keep highlight on to see
              snippets (adds fetch time).
            </p>
          )}
          <form className="space-y-3" onSubmit={(e) => void runSearch(e)}>
            <div className="flex items-center gap-2 rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus-within:ring-1 focus-within:ring-ring">
              <span className="select-none text-primary">▸</span>
              <input
                className="flex-1 bg-transparent outline-none placeholder:text-muted-foreground"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="search query…"
                autoFocus
              />
              <Button
                type="submit"
                size="sm"
                disabled={loading || !table || !searchQuery.trim()}
              >
                {loading ? (
                  <LoaderCircle className="size-4 animate-spin" />
                ) : (
                  <Search className="size-4" />
                )}
                Search
              </Button>
            </div>

            <div className="flex flex-wrap items-center gap-2 text-xs">
              <select
                className="h-8 rounded-md border border-input bg-background px-2 font-mono"
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
                      {t.name} ({t.rows})
                    </option>
                  ))
                )}
              </select>
              <select
                className="h-8 rounded-md border border-input bg-background px-2 font-mono"
                value={strategy}
                onChange={(e) => setStrategy(e.target.value)}
              >
                {SEARCH_STRATEGIES.map((s) => (
                  <option key={s.value || "auto"} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
              <label className="flex items-center gap-1 text-muted-foreground">
                k
                <Input
                  className="h-8 w-16"
                  type="number"
                  min={1}
                  max={100}
                  value={topK}
                  onChange={(e) => setTopK(Number(e.target.value) || 10)}
                />
              </label>
              <label className="flex items-center gap-1 text-muted-foreground">
                offset
                <Input
                  className="h-8 w-16"
                  type="number"
                  min={0}
                  value={offset}
                  onChange={(e) => setOffset(Number(e.target.value) || 0)}
                />
              </label>
              <label className="flex items-center gap-2 text-muted-foreground">
                <Switch checked={highlight} onCheckedChange={setHighlight} />
                highlight
              </label>
              <label className="flex items-center gap-2 text-muted-foreground">
                <Switch checked={explain} onCheckedChange={setExplain} />
                explain
              </label>
              <button
                type="button"
                onClick={() => setShowTuning((v) => !v)}
                className="ml-auto flex items-center gap-1 rounded-md border border-border px-2 py-1.5 text-muted-foreground hover:text-foreground"
              >
                <SlidersHorizontal className="size-3.5" />
                Tuning
                <ChevronRight
                  className={`size-3.5 transition-transform ${showTuning ? "rotate-90" : ""}`}
                />
              </button>
            </div>

            {showTuning && (
              <div className="grid gap-4 rounded-md border border-border bg-muted/20 p-3 text-xs sm:grid-cols-2">
                <div className="space-y-2">
                  <div className="font-medium text-muted-foreground">BM25 params</div>
                  <div className="flex gap-2">
                    <label className="flex items-center gap-1">
                      k1
                      <Input
                        className="h-8 w-20"
                        placeholder="1.2"
                        value={k1}
                        onChange={(e) => setK1(e.target.value)}
                      />
                    </label>
                    <label className="flex items-center gap-1">
                      b
                      <Input
                        className="h-8 w-20"
                        placeholder="0.75"
                        value={b}
                        onChange={(e) => setB(e.target.value)}
                      />
                    </label>
                  </div>

                  <div className="pt-1 font-medium text-muted-foreground">Temporal decay</div>
                  <div className="flex gap-2">
                    <Input
                      className="h-8 flex-1"
                      placeholder="field (e.g. published)"
                      value={decayField}
                      onChange={(e) => setDecayField(e.target.value)}
                    />
                    <Input
                      className="h-8 w-28"
                      placeholder="half-life days"
                      value={decayHalfLife}
                      onChange={(e) => setDecayHalfLife(e.target.value)}
                    />
                  </div>

                  {highlight && (
                    <>
                      <div className="pt-1 font-medium text-muted-foreground">Snippet length</div>
                      <Input
                        className="h-8 w-28"
                        type="number"
                        min={16}
                        value={snippetLen}
                        onChange={(e) => setSnippetLen(Number(e.target.value) || 160)}
                      />
                    </>
                  )}
                </div>

                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="font-medium text-muted-foreground">Field boosts</span>
                    <button
                      type="button"
                      className="flex items-center gap-1 text-primary hover:underline"
                      onClick={() => setBoosts((b) => [...b, { field: "", factor: "2.0" }])}
                    >
                      <Plus className="size-3" /> add
                    </button>
                  </div>
                  {boosts.length === 0 && (
                    <p className="text-muted-foreground">No boosts — add a field → factor.</p>
                  )}
                  {boosts.map((row, i) => (
                    <div key={i} className="flex gap-2">
                      <Input
                        className="h-8 flex-1"
                        placeholder="field"
                        value={row.field}
                        onChange={(e) =>
                          setBoosts((b) =>
                            b.map((r, j) => (j === i ? { ...r, field: e.target.value } : r)),
                          )
                        }
                      />
                      <Input
                        className="h-8 w-20"
                        placeholder="2.0"
                        value={row.factor}
                        onChange={(e) =>
                          setBoosts((b) =>
                            b.map((r, j) => (j === i ? { ...r, factor: e.target.value } : r)),
                          )
                        }
                      />
                      <button
                        type="button"
                        className="text-muted-foreground hover:text-destructive-foreground"
                        onClick={() => setBoosts((b) => b.filter((_, j) => j !== i))}
                      >
                        <X className="size-4" />
                      </button>
                    </div>
                  ))}

                  <div className="pt-1 font-medium text-muted-foreground">Facets</div>
                  <Input
                    className="h-8"
                    placeholder="comma-separated fields (e.g. category, author)"
                    value={facetsInput}
                    onChange={(e) => setFacetsInput(e.target.value)}
                  />

                  {(strategy === "splade" || strategy === "seismic") && (
                    <>
                      <div className="pt-1 font-medium text-muted-foreground">
                        Sparse weights (JSON)
                      </div>
                      <Input
                        className="h-8 font-mono"
                        placeholder={`{"tesla": 2.1, "motor": 1.4}`}
                        value={sparseJson}
                        onChange={(e) => setSparseJson(e.target.value)}
                      />
                    </>
                  )}

                  {vectorDim > 0 && (
                    <>
                      <div className="pt-1 font-medium text-muted-foreground">
                        Query vector (JSON, dim {vectorDim})
                      </div>
                      <Input
                        className="h-8 font-mono"
                        value={queryVectorJson}
                        onChange={(e) => setQueryVectorJson(e.target.value)}
                        placeholder={`[${Array(Math.min(vectorDim, 4)).fill("0").join(", ")}…]`}
                      />
                    </>
                  )}

                  <label className="flex items-center gap-2 pt-1 text-muted-foreground">
                    <Switch checked={fetchText} onCheckedChange={setFetchText} />
                    load passage text
                  </label>
                  <label className="flex items-center gap-2 text-muted-foreground">
                    <Switch checked={graphExpand} onCheckedChange={setGraphExpand} />
                    graph expand
                  </label>
                </div>
              </div>
            )}

            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={saveCurrentSearch}
                disabled={!table || !searchQuery.trim()}
              >
                <Star className="size-3.5" />
                Save preset
              </Button>
              {equivalentSql && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
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
              {activeStrategy && <Badge variant="secondary">strategy: {activeStrategy}</Badge>}
              {searchMs != null && (
                <Badge variant="outline">search {Math.round(searchMs)} ms</Badge>
              )}
              {fetchMs != null && fetchText && (
                <Badge variant="outline">fetch {Math.round(fetchMs)} ms</Badge>
              )}
              {latencyMs != null && (
                <Badge variant="secondary">total {Math.round(latencyMs)} ms</Badge>
              )}
            </div>
          </form>

          {savedSearches.length > 0 && (
            <div className="mt-3 flex flex-wrap items-center gap-1.5 border-t border-border pt-3">
              <span className="text-xs text-muted-foreground">Saved:</span>
              {savedSearches.map((s) => (
                <span
                  key={s.id}
                  className="inline-flex items-center gap-1 rounded-full border border-border bg-card pl-2.5 text-xs"
                >
                  <button
                    type="button"
                    title={`${s.request.table} · ${s.request.query}`}
                    onClick={() => void runSavedSearch(s.request)}
                    className="py-1 hover:text-primary"
                  >
                    {s.name}
                  </button>
                  <button
                    type="button"
                    aria-label={`Delete ${s.name}`}
                    onClick={() => removeSavedSearch(s.id)}
                    className="px-1.5 py-1 text-muted-foreground hover:text-destructive"
                  >
                    <X className="size-3" />
                  </button>
                </span>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/20 p-2 text-sm text-destructive-foreground">
          {error}
        </div>
      )}

      <ExplainPanel text={explainText} provenance={provenance} />
      <QueryMetricsCard metrics={metrics} />

      {/* Results + facet sidebar */}
      <div className="grid gap-4 lg:grid-cols-[1fr_280px]">
        <Card>
          <CardHeader className="flex-row items-center justify-between gap-3 space-y-0">
            <CardTitle className="text-base">
              Results{" "}
              <span className="text-sm font-normal text-muted-foreground">
                · {filteredHits.length} hit{filteredHits.length === 1 ? "" : "s"}
              </span>
            </CardTitle>
            <div className="flex items-center gap-1 rounded-md border border-border p-0.5 text-xs">
              <button
                type="button"
                onClick={() => setViewMode("cards")}
                className={`rounded px-2 py-1 ${viewMode === "cards" ? "bg-muted text-foreground" : "text-muted-foreground"}`}
              >
                Cards
              </button>
              <button
                type="button"
                onClick={() => setViewMode("table")}
                className={`rounded px-2 py-1 ${viewMode === "table" ? "bg-muted text-foreground" : "text-muted-foreground"}`}
              >
                Table
              </button>
            </div>
          </CardHeader>
          <CardContent className="space-y-3">
            <Input
              value={resultFilter}
              onChange={(e) => setResultFilter(e.target.value)}
              placeholder="Filter results…"
              className="h-8"
            />
            {viewMode === "table" ? (
              <DataTable
                columns={columns}
                data={filteredRows}
                emptyMessage={loading ? "Searching…" : "Run a search to see results"}
                pageSize={25}
              />
            ) : filteredHits.length === 0 ? (
              <div className="rounded-md border border-dashed border-border p-8 text-center text-sm text-muted-foreground">
                {loading ? "Searching…" : "Run a search to see results"}
              </div>
            ) : (
              <div className="space-y-2">
                {filteredHits.map((h) => (
                  <ResultCard key={h.id} hit={h} fetchText={fetchText} />
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        {facets.length > 0 && (
          <div className="space-y-3">
            {facets.map((group) => (
              <Card key={group.field}>
                <CardHeader className="pb-2">
                  <CardTitle className="font-mono text-xs uppercase tracking-wide text-muted-foreground">
                    {group.field}
                  </CardTitle>
                </CardHeader>
                <CardContent className="space-y-1">
                  {group.values.map((v) => (
                    <button
                      key={v.value}
                      type="button"
                      onClick={() => applyFacet(v.value)}
                      className="flex w-full items-center justify-between rounded px-2 py-1 text-left text-xs hover:bg-muted"
                    >
                      <span className="truncate">{v.value || "—"}</span>
                      <span className="ml-2 font-mono text-muted-foreground">{v.count}</span>
                    </button>
                  ))}
                </CardContent>
              </Card>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function ResultCard({ hit, fetchText }: { hit: SearchHit; fetchText: boolean }) {
  const meta = Object.entries(hit.metadata);
  const body =
    hit.snippet ??
    hit.text ??
    (fetchText ? "(text not found for this id)" : "(enable Load passage text)");
  return (
    <div className="rounded-md border border-border bg-card/60 p-3">
      <div className="mb-1 flex items-center gap-2 font-mono text-xs text-muted-foreground">
        <span className="text-foreground">#{hit.id}</span>
        <span>·</span>
        <span>{hit.score.toFixed(4)}</span>
      </div>
      {hit.snippet ? (
        <HighlightedSnippet snippet={body} className="text-sm leading-relaxed" />
      ) : (
        <p className="text-sm leading-relaxed">{body}</p>
      )}
      {meta.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {meta.map(([k, v]) => (
            <Badge key={k} variant="outline" className="font-mono text-[11px]">
              {k}={v}
            </Badge>
          ))}
        </div>
      )}
    </div>
  );
}

export default function SearchPage() {
  return (
    <Suspense fallback={<div className="text-sm text-muted-foreground">Loading search…</div>}>
      <SearchPageInner />
    </Suspense>
  );
}
