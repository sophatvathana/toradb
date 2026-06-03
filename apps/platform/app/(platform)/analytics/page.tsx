"use client";

import { useEffect, useId, useMemo, useState } from "react";
import { BarChart3, LoaderCircle, Play } from "lucide-react";

import { SqlBarChart, sqlResultToBarChart } from "@/components/sql-bar-chart";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  fetchTableDetail,
  runSql,
  type ColumnTypeEntry,
  type SqlResponse,
} from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

const AGG_FUNCS = ["COUNT(*)", "SUM", "AVG", "MIN", "MAX"] as const;
type AggFunc = (typeof AGG_FUNCS)[number];

/** Visual GROUP BY / aggregation builder. Composes a retrieval-SQL analytics query
 *  (SELECT <group>, <agg> FROM t [WHERE …] [SPARSE SEARCH …] GROUP BY <group>),
 *  runs it via /api/sql, and renders the result table + a CSS bar chart. */
export default function AnalyticsPage() {
  const tables = usePlatformStore((s) => s.tables);

  const [table, setTable] = useState("");
  const [columns, setColumns] = useState<ColumnTypeEntry[]>([]);
  const [groupBy, setGroupBy] = useState("");
  const [aggFunc, setAggFunc] = useState<AggFunc>("COUNT(*)");
  const [aggColumn, setAggColumn] = useState("");
  const [whereClause, setWhereClause] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [limit, setLimit] = useState(50);

  const [result, setResult] = useState<SqlResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!table && tables[0]) setTable(tables[0].name);
  }, [tables, table]);

  useEffect(() => {
    if (!table) return;
    void fetchTableDetail(table)
      .then((d) => {
        setColumns(d.column_types);
        // Default group-by to the first non-id column if unset.
        setGroupBy((g) => g || d.column_types.find((c) => c.name !== "id")?.name || "");
      })
      .catch(() => setColumns([]));
  }, [table]);

  const sql = useMemo(() => {
    if (!table || !groupBy.trim()) return "";
    const agg =
      aggFunc === "COUNT(*)"
        ? "COUNT(*)"
        : `${aggFunc}(${aggColumn.trim() || "value"})`;
    let q = `SELECT ${groupBy.trim()}, ${agg} FROM ${table}`;
    if (searchQuery.trim()) {
      q += ` SPARSE SEARCH body BM25('${searchQuery.trim().replace(/'/g, "")}')`;
    }
    if (whereClause.trim()) q += ` WHERE ${whereClause.trim()}`;
    q += ` GROUP BY ${groupBy.trim()}`;
    if (limit > 0) q += ` LIMIT ${limit}`;
    return q;
  }, [table, groupBy, aggFunc, aggColumn, searchQuery, whereClause, limit]);

  async function run() {
    if (!sql) return;
    setLoading(true);
    setError("");
    try {
      const data = await runSql(sql);
      setResult(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setResult(null);
    } finally {
      setLoading(false);
    }
  }

  const chart = useMemo(
    () => (result ? sqlResultToBarChart(result) : null),
    [result],
  );

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <BarChart3 className="size-4 text-primary" />
            Analytics
          </CardTitle>
          <CardDescription>
            Group-by aggregations over a table · compiles to retrieval SQL
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">Table</span>
              <select
                className="h-9 w-full rounded-md border border-input bg-background px-2 font-mono text-sm"
                value={table}
                onChange={(e) => {
                  setTable(e.target.value);
                  setGroupBy("");
                  setResult(null);
                }}
              >
                {tables.length === 0 ? (
                  <option value="">No tables</option>
                ) : (
                  tables.map((t) => (
                    <option key={t.name} value={t.name}>
                      {t.name}
                    </option>
                  ))
                )}
              </select>
            </label>

            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">Group by</span>
              <ColumnPicker
                value={groupBy}
                columns={columns}
                onChange={setGroupBy}
                placeholder="field"
              />
            </label>

            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">Aggregate</span>
              <select
                className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm"
                value={aggFunc}
                onChange={(e) => setAggFunc(e.target.value as AggFunc)}
              >
                {AGG_FUNCS.map((f) => (
                  <option key={f} value={f}>
                    {f}
                  </option>
                ))}
              </select>
            </label>

            {aggFunc !== "COUNT(*)" && (
              <label className="space-y-1 text-xs">
                <span className="text-muted-foreground">Of column</span>
                <ColumnPicker
                  value={aggColumn}
                  columns={columns}
                  onChange={setAggColumn}
                  placeholder="numeric field"
                />
              </label>
            )}
          </div>

          <div className="grid gap-3 sm:grid-cols-3">
            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">Search filter (optional)</span>
              <Input
                className="h-9"
                placeholder="BM25 query, e.g. tesla"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </label>
            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">WHERE (optional)</span>
              <Input
                className="h-9 font-mono"
                placeholder="tag = 'patent'"
                value={whereClause}
                onChange={(e) => setWhereClause(e.target.value)}
              />
            </label>
            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">Limit</span>
              <Input
                className="h-9 w-24"
                type="number"
                min={1}
                value={limit}
                onChange={(e) => setLimit(Number(e.target.value) || 50)}
              />
            </label>
          </div>

          {sql && (
            <pre className="overflow-x-auto rounded-md border border-border bg-muted/40 p-2 font-mono text-xs text-foreground">
              {sql}
            </pre>
          )}

          <Button type="button" onClick={() => void run()} disabled={loading || !sql}>
            {loading ? (
              <LoaderCircle className="size-4 animate-spin" />
            ) : (
              <Play className="size-4" />
            )}
            Run
          </Button>
        </CardContent>
      </Card>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/15 p-2 text-sm text-destructive">
          {error}
        </div>
      )}

      {chart && <SqlBarChart chart={chart} />}

      {result && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">
              Result{" "}
              <span className="font-normal text-muted-foreground">
                · {result.rows.length} rows · {Math.round(result.latency_ms)} ms
              </span>
            </CardTitle>
          </CardHeader>
          <CardContent className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-border text-left text-muted-foreground">
                  {result.columns.map((c) => (
                    <th key={c} className="px-2 py-1 font-mono font-medium">
                      {c}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {result.rows.map((row, i) => (
                  <tr key={i} className="border-b border-border/60 hover:bg-muted/50">
                    {result.columns.map((c) => (
                      <td key={c} className="px-2 py-1 font-mono">
                        {String(row[c] ?? "—")}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

/** Column name input with a datalist of the table's known columns. */
function ColumnPicker({
  value,
  columns,
  onChange,
  placeholder,
}: {
  value: string;
  columns: ColumnTypeEntry[];
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  const listId = useId();
  return (
    <>
      <Input
        className="h-9 font-mono"
        list={listId}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
      <datalist id={listId}>
        {columns.map((c) => (
          <option key={c.name} value={c.name}>
            {c.type}
          </option>
        ))}
      </datalist>
    </>
  );
}
