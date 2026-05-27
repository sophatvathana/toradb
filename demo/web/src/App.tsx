import { useCallback, useEffect, useState } from "react";
import {
  api,
  type Health,
  type IndexStatus,
  type SearchResponse,
  type SqlResult,
  type TableInfo,
} from "./api";

type Tab = "search" | "sql" | "catalog";

const STRATEGIES = [
  { value: "", label: "Default (hybrid when vectors exist)" },
  { value: "sparse", label: "Sparse / BM25" },
  { value: "dense", label: "Dense / HNSW" },
  { value: "hybrid", label: "Hybrid + graph" },
  { value: "distributed", label: "Distributed segment scan" },
  { value: "diskann", label: "DiskANN" },
];

export default function App() {
  const [tab, setTab] = useState<Tab>("search");
  const [health, setHealth] = useState<Health | null>(null);
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [table, setTable] = useState("articles");
  const [query, setQuery] = useState("Nikola Tesla alternating current");
  const [topK, setTopK] = useState(10);
  const [strategy, setStrategy] = useState("");
  const [explain, setExplain] = useState(false);
  const [graphExpand, setGraphExpand] = useState(false);
  const [searchResult, setSearchResult] = useState<SearchResponse | null>(null);
  const [sqlText, setSqlText] = useState(
    "SELECT tag, COUNT(*) FROM articles GROUP BY tag",
  );
  const [sqlResult, setSqlResult] = useState<SqlResult | null>(null);
  const [cliHint, setCliHint] = useState<{ query: string; sql: string } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [samples, setSamples] = useState<{ search: string[]; sql: string[] } | null>(null);
  const [indexStatus, setIndexStatus] = useState<IndexStatus | null>(null);

  const refreshMeta = useCallback(async () => {
    const [h, t] = await Promise.all([api.health(), api.tables()]);
    setHealth(h);
    setTables(t);
    if (t.length && !t.some((x) => x.name === table)) {
      setTable(t[0].name);
    }
  }, [table]);

  useEffect(() => {
    refreshMeta().catch((e) => setError(String(e)));
  }, [refreshMeta]);

  useEffect(() => {
    api.sampleQueries(table).then(setSamples).catch(() => setSamples(null));
  }, [table]);

  useEffect(() => {
    const indexing =
      (health?.indexing_tables?.length ?? 0) > 0 ||
      health?.indexing_tables?.includes(table);
    if (!indexing && indexStatus?.state !== "building") {
      return;
    }
    let cancelled = false;
    const poll = async () => {
      try {
        const st = await api.indexStatus(table);
        if (!cancelled) {
          setIndexStatus(st);
        }
      } catch {
        if (!cancelled) {
          setIndexStatus(null);
        }
      }
    };
    poll();
    const id = window.setInterval(poll, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [health?.indexing_tables, table, indexStatus?.state]);

  const indexingActive =
    indexStatus?.state === "building" ||
    (health?.indexing_tables?.includes(table) ?? false);

  const runSearch = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await api.search({
        table,
        query,
        top_k: topK,
        strategy: strategy || null,
        explain,
        graph_expand: graphExpand,
      });
      setSearchResult(res);
      const hint = await api.cliHint(table, query);
      setCliHint(hint);
    } catch (e) {
      const err = e as Error & { code?: string };
      if (err.code === "index_building") {
        setError("Search is disabled while indexes are building. Try again when finish completes.");
      } else {
        setError(err instanceof Error ? err.message : String(e));
      }
    } finally {
      setLoading(false);
    }
  };

  const runSql = async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await api.sql(sqlText);
      setSqlResult(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  const degraded = health?.status !== "ok";

  return (
    <div className="app">
      <header className="header">
        <div className="brand">
          <span className="logo-mark" aria-hidden />
          <div>
            <h1>ToraDB</h1>
            <p>Local retrieval database — live demo</p>
          </div>
        </div>
        <nav className="tabs">
          {(["search", "sql", "catalog"] as Tab[]).map((t) => (
            <button
              key={t}
              type="button"
              className={tab === t ? "active" : ""}
              onClick={() => setTab(t)}
            >
              {t === "search" ? "Search" : t === "sql" ? "SQL" : "Catalog"}
            </button>
          ))}
        </nav>
      </header>

      {indexingActive && (
        <div className="banner warn">
          <strong>Indexing in progress.</strong>{" "}
          {indexStatus?.segments_total
            ? `${indexStatus.segments_done ?? 0}/${indexStatus.segments_total} segments`
            : "Building BM25 indexes"}
          {indexStatus?.phase ? ` (${indexStatus.phase})` : ""}. Search is disabled until finish
          completes.
        </div>
      )}

      {degraded && !indexingActive && (
        <div className="banner warn">
          <strong>Setup required.</strong>{" "}
          {health?.hint ?? "Build the demo database and start the API."}
          <code className="inline">python examples/full_example.py</code>
        </div>
      )}

      {error && (
        <div className="banner err" role="alert">
          {error}
        </div>
      )}

      <main className="layout">
        <aside className="sidebar">
          <section>
            <h2>Database</h2>
            <p className="mono path">{health?.db_path ?? "…"}</p>
            <p className="muted">
              {health?.db_exists
                ? `${tables.reduce((n, t) => n + t.rows, 0)} rows across ${tables.length} tables`
                : "Not loaded"}
            </p>
          </section>

          <section>
            <h2>Table</h2>
            <select
              value={table}
              onChange={(e) => setTable(e.target.value)}
              disabled={!tables.length}
            >
              {tables.map((t) => (
                <option key={t.name} value={t.name}>
                  {t.name} ({t.rows})
                </option>
              ))}
            </select>
          </section>

          {samples && tab === "search" && (
            <section>
              <h2>Try a query</h2>
              <ul className="chips">
                {samples.search.map((q) => (
                  <li key={q}>
                    <button type="button" onClick={() => setQuery(q)}>
                      {q}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {samples && tab === "sql" && (
            <section>
              <h2>Example SQL</h2>
              <ul className="chips">
                {samples.sql.map((q) => (
                  <li key={q}>
                    <button type="button" onClick={() => setSqlText(q)}>
                      {q.length > 48 ? `${q.slice(0, 48)}…` : q}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}
        </aside>

        <div className="panel">
          {tab === "search" && (
            <>
              <form
                className="search-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  runSearch();
                }}
              >
                <label>
                  Query
                  <input
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Nikola Tesla alternating current"
                  />
                </label>
                <div className="row">
                  <label>
                    Top K
                    <input
                      type="number"
                      min={1}
                      max={100}
                      value={topK}
                      onChange={(e) => setTopK(Number(e.target.value))}
                    />
                  </label>
                  <label>
                    Strategy
                    <select
                      value={strategy}
                      onChange={(e) => setStrategy(e.target.value)}
                    >
                      {STRATEGIES.map((s) => (
                        <option key={s.value} value={s.value}>
                          {s.label}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
                <div className="checks">
                  <label>
                    <input
                      type="checkbox"
                      checked={explain}
                      onChange={(e) => setExplain(e.target.checked)}
                    />
                    Explain plan
                  </label>
                  <label>
                    <input
                      type="checkbox"
                      checked={graphExpand}
                      onChange={(e) => setGraphExpand(e.target.checked)}
                    />
                    Graph expand
                  </label>
                </div>
                <button
                  type="submit"
                  className="primary"
                  disabled={loading || degraded || indexingActive}
                >
                  {indexingActive ? "Indexing…" : loading ? "Searching…" : "Search"}
                </button>
              </form>

              {searchResult && (
                <div className="results">
                  <p className="meta">
                    {searchResult.hits.length} hits
                    {searchResult.total_ms != null ? (
                      <>
                        {" "}
                        · search {searchResult.search_ms ?? searchResult.latency_ms} ms · fetch{" "}
                        {searchResult.fetch_ms ?? 0} ms · total {searchResult.total_ms} ms
                      </>
                    ) : (
                      <> · {searchResult.latency_ms} ms</>
                    )}
                    {searchResult.strategy ? ` · ${searchResult.strategy}` : ""}
                  </p>
                  {searchResult.explain && (
                    <pre className="explain">{searchResult.explain}</pre>
                  )}
                  <ol className="hits">
                    {searchResult.hits.map((h) => (
                      <li key={h.id}>
                        <div className="hit-head">
                          <span className="id">#{h.id}</span>
                          <span className="score">{h.score.toFixed(4)}</span>
                        </div>
                        <p className="text">{h.text ?? "(text not on disk yet)"}</p>
                        {Object.keys(h.metadata).length > 0 && (
                          <div className="tags">
                            {Object.entries(h.metadata).map(([k, v]) => (
                              <span key={k}>
                                {k}:{v}
                              </span>
                            ))}
                          </div>
                        )}
                      </li>
                    ))}
                  </ol>
                  {cliHint && (
                    <div className="cli">
                      <h3>CLI equivalent</h3>
                      <code>{cliHint.query}</code>
                      <code>{cliHint.sql}</code>
                    </div>
                  )}
                </div>
              )}
            </>
          )}

          {tab === "sql" && (
            <>
              <form
                className="sql-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  runSql();
                }}
              >
                <label>
                  SQL
                  <textarea
                    value={sqlText}
                    onChange={(e) => setSqlText(e.target.value)}
                    rows={6}
                    spellCheck={false}
                  />
                </label>
                <button type="submit" className="primary" disabled={loading || degraded}>
                  {loading ? "Running…" : "Run SQL"}
                </button>
              </form>
              {sqlResult && (
                <div className="sql-out">
                  <p className="meta">{sqlResult.latency_ms} ms</p>
                  {sqlResult.kind === "message" ? (
                    <pre>{sqlResult.text}</pre>
                  ) : (
                    <div className="table-wrap">
                      <table>
                        <thead>
                          <tr>
                            {sqlResult.columns.map((c) => (
                              <th key={c}>{c}</th>
                            ))}
                          </tr>
                        </thead>
                        <tbody>
                          {sqlResult.rows.map((row, i) => (
                            <tr key={i}>
                              {sqlResult.columns.map((c) => (
                                <td key={c}>{String(row[c] ?? "")}</td>
                              ))}
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  )}
                </div>
              )}
            </>
          )}

          {tab === "catalog" && (
            <div className="catalog">
              <button type="button" onClick={() => refreshMeta()} disabled={loading}>
                Refresh
              </button>
              {tables.map((t) => (
                <article key={t.name} className="table-card">
                  <h3>
                    {t.name}
                    <span className="badge">{t.rows} rows</span>
                  </h3>
                  {t.describe && <pre>{t.describe}</pre>}
                </article>
              ))}
            </div>
          )}
        </div>
      </main>

      <footer className="footer">
        <a href="https://toradb.mintlify.app" target="_blank" rel="noreferrer">
          Documentation
        </a>
        <span>·</span>
        <a href="https://github.com/sophatvathana/toradb" target="_blank" rel="noreferrer">
          GitHub
        </a>
        <span>·</span>
        <span className="muted">Point TORADB_DB_PATH at a 1M+ corpus for large-scale demos</span>
      </footer>
    </div>
  );
}
