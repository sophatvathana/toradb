"use client";

import { useEffect, useMemo, useState } from "react";
import { LoaderCircle, RefreshCw, ScrollText } from "lucide-react";

import { ExplainPanel } from "@/components/explain-panel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { fetchSearchLog, type ProvenanceRecord } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

export default function ProvenancePage() {
  const tables = usePlatformStore((s) => s.tables);
  const [table, setTable] = useState("");
  const [records, setRecords] = useState<ProvenanceRecord[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!table && tables[0]) setTable(tables[0].name);
  }, [tables, table]);

  async function load(name: string) {
    if (!name) return;
    setLoading(true);
    setError("");
    try {
      const data = await fetchSearchLog(name, 50);
      // Newest first — the log is appended chronologically.
      setRecords([...data.records].reverse());
      setSelected(data.records.length ? 0 : null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setRecords([]);
      setSelected(null);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (table) void load(table);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [table]);

  const active = selected != null ? records[selected] : null;

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader className="flex-row items-center justify-between gap-3 space-y-0">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <ScrollText className="size-4 text-primary" />
              Provenance explorer
            </CardTitle>
            <CardDescription>
              Persisted retrieval traces · GET /api/tables/{"{name}"}/search-log
            </CardDescription>
          </div>
          <div className="flex items-center gap-2">
            <select
              className="h-8 rounded-md border border-input bg-background px-2 font-mono text-xs"
              value={table}
              onChange={(e) => setTable(e.target.value)}
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
            <Button
              variant="outline"
              size="sm"
              onClick={() => void load(table)}
              disabled={loading || !table}
            >
              {loading ? (
                <LoaderCircle className="size-4 animate-spin" />
              ) : (
                <RefreshCw className="size-4" />
              )}
              Refresh
            </Button>
          </div>
        </CardHeader>
      </Card>

      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/15 p-2 text-sm text-destructive">
          {error}
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-[320px_1fr]">
        {/* Log list */}
        <Card className="lg:max-h-[70vh] lg:overflow-y-auto">
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">
              Recent queries{" "}
              <span className="font-normal text-muted-foreground">· {records.length}</span>
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-1">
            {records.length === 0 ? (
              <p className="rounded-md border border-dashed border-border p-6 text-center text-xs text-muted-foreground">
                {loading
                  ? "Loading…"
                  : "No provenance recorded yet. Run a search with “explain” on, then refresh."}
              </p>
            ) : (
              records.map((rec, i) => (
                <LogRow
                  key={i}
                  record={rec}
                  selected={i === selected}
                  onClick={() => setSelected(i)}
                />
              ))
            )}
          </CardContent>
        </Card>

        {/* Detail */}
        <div className="min-w-0 space-y-4">
          {active ? (
            <ExplainPanel text={null} provenance={active} />
          ) : (
            <Card>
              <CardContent className="p-8 text-center text-sm text-muted-foreground">
                Select a query to inspect its retrieval provenance.
              </CardContent>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}

function LogRow({
  record,
  selected,
  onClick,
}: {
  record: ProvenanceRecord;
  selected: boolean;
  onClick: () => void;
}) {
  const hits = record.final_ids.length;
  return (
    <button
      type="button"
      onClick={onClick}
      className={`w-full rounded-md border-l-2 px-2.5 py-2 text-left transition-colors ${
        selected
          ? "border-primary bg-accent"
          : "border-transparent hover:bg-accent hover:text-foreground"
      }`}
    >
      <div className="truncate font-mono text-xs">{record.query || "(empty query)"}</div>
      <div className="mt-1 flex items-center gap-1.5">
        {record.strategy && (
          <Badge variant="outline" className="text-[10px]">
            {record.strategy}
          </Badge>
        )}
        <span className="font-mono text-[11px] text-muted-foreground">
          {hits} hit{hits === 1 ? "" : "s"} · {record.total_latency_ms.toFixed(1)} ms
        </span>
      </div>
    </button>
  );
}
