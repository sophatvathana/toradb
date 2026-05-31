"use client";

import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { DataTable } from "@/components/data-table";
import { runSql, type SqlResponse } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";
import { type ColumnDef } from "@tanstack/react-table";
import { useMemo } from "react";

const COLUMN_TYPES = [
  "text",
  "int",
  "float",
  "bool",
  "date",
  "timestamp",
  "json",
  "uuid",
  "vector",
] as const;

type ColumnRow = { name: string; type: string };

export default function SchemaPage() {
  const [tableName, setTableName] = useState("passages");
  const [namespace, setNamespace] = useState("");
  const [mode, setMode] = useState<"TEXT" | "VECTOR" | "HYBRID">("HYBRID");
  const [columns, setColumns] = useState<ColumnRow[]>([]);
  const [ddlResult, setDdlResult] = useState<SqlResponse | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  function addColumn() {
    setColumns((cols) => [...cols, { name: "", type: "text" }]);
  }
  function updateColumn(idx: number, patch: Partial<ColumnRow>) {
    setColumns((cols) => cols.map((c, i) => (i === idx ? { ...c, ...patch } : c)));
  }
  function removeColumn(idx: number) {
    setColumns((cols) => cols.filter((_, i) => i !== idx));
  }

  const refreshTables = usePlatformStore((s) => s.refreshTables);

  async function runDdl(sql: string) {
    setLoading(true);
    setError("");
    try {
      const res = await runSql(sql);
      setDdlResult(res);
      await refreshTables();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setDdlResult(null);
    } finally {
      setLoading(false);
    }
  }

  function onCreateTable() {
    const qualified = namespace.trim()
      ? `${namespace.trim()}.${tableName.trim()}`
      : tableName.trim();
    const cols = columns.filter((c) => c.name.trim());
    const colClause = cols.length
      ? ` (${cols.map((c) => `${c.name.trim()} ${c.type}`).join(", ")})`
      : "";
    void runDdl(`CREATE TABLE ${qualified}${colClause} USING ${mode}`);
  }

  const resultColumns = useMemo<ColumnDef<Record<string, unknown>>[]>(() => {
    if (!ddlResult?.columns.length) return [];
    return ddlResult.columns.map((col) => ({
      accessorKey: col,
      header: col,
      cell: ({ row }) => String(row.getValue(col) ?? ""),
    }));
  }, [ddlResult]);

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Create table</CardTitle>
          <CardDescription>Runs CREATE TABLE via /api/sql</CardDescription>
        </CardHeader>
        <CardContent className="grid max-w-lg gap-3">
          <label className="text-sm">
            <span className="text-muted-foreground">Namespace (optional)</span>
            <Input
              className="mt-1"
              value={namespace}
              onChange={(e) => setNamespace(e.target.value)}
              placeholder="db"
            />
          </label>
          <label className="text-sm">
            <span className="text-muted-foreground">Table name</span>
            <Input
              className="mt-1"
              value={tableName}
              onChange={(e) => setTableName(e.target.value)}
            />
          </label>
          <label className="text-sm">
            <span className="text-muted-foreground">Mode</span>
            <select
              className="mt-1 w-full rounded-md border border-border bg-card px-2 py-1.5 text-sm"
              value={mode}
              onChange={(e) => setMode(e.target.value as typeof mode)}
            >
              <option value="TEXT">TEXT</option>
              <option value="VECTOR">VECTOR</option>
              <option value="HYBRID">HYBRID</option>
            </select>
          </label>
          <div className="text-sm">
            <div className="mb-1 flex items-center justify-between">
              <span className="text-muted-foreground">Columns (optional)</span>
              <Button type="button" variant="outline" size="sm" onClick={addColumn}>
                + Add column
              </Button>
            </div>
            {columns.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                No declared columns — metadata stays untyped (text).
              </p>
            ) : (
              <div className="space-y-2">
                {columns.map((col, idx) => (
                  <div key={idx} className="flex gap-2">
                    <Input
                      className="flex-1"
                      value={col.name}
                      placeholder="column name"
                      onChange={(e) => updateColumn(idx, { name: e.target.value })}
                    />
                    <select
                      className="w-32 rounded-md border border-border bg-card px-2 py-1.5 text-sm"
                      value={col.type}
                      onChange={(e) => updateColumn(idx, { type: e.target.value })}
                    >
                      {COLUMN_TYPES.map((t) => (
                        <option key={t} value={t}>
                          {t}
                        </option>
                      ))}
                    </select>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => removeColumn(idx)}
                    >
                      ✕
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </div>
          <Button type="button" disabled={loading || !tableName.trim()} onClick={onCreateTable}>
            Create table
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Quick DDL</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={() => void runDdl("SHOW TABLES")}
          >
            SHOW TABLES
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={() => void runDdl("SHOW MATERIALIZED VIEWS")}
          >
            SHOW MATERIALIZED VIEWS
          </Button>
        </CardContent>
      </Card>

      {error && (
        <p className="rounded-md border border-destructive/50 bg-destructive/20 p-2 text-sm">
          {error}
        </p>
      )}

      {ddlResult && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Result ({ddlResult.kind})</CardTitle>
            <CardDescription>{Math.round(ddlResult.latency_ms)} ms</CardDescription>
          </CardHeader>
          <CardContent>
            {ddlResult.rows.length > 0 ? (
              <DataTable
                columns={resultColumns}
                data={ddlResult.rows as Record<string, unknown>[]}
              />
            ) : (
              <pre className="whitespace-pre-wrap text-xs text-muted-foreground">
                {JSON.stringify(ddlResult.rows, null, 2) ||
                  ddlResult.explain_text ||
                  "ok"}
              </pre>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
