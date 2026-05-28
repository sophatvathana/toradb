"use client";

import { useMemo, useState } from "react";
import { type ColumnDef } from "@tanstack/react-table";

import { DataTable } from "@/components/data-table";
import { ExplainPanel } from "@/components/explain-panel";
import { QueryMetricsCard } from "@/components/query-metrics-card";
import { SqlEditor } from "@/components/sql-editor";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { usePlatformStore } from "@/stores/platform-store";

function exportCsv(columns: string[], rows: Record<string, unknown>[]) {
  const header = columns.join(",");
  const body = rows
    .map((row) =>
      columns.map((col) => JSON.stringify(row[col] ?? "")).join(","),
    )
    .join("\n");
  const blob = new Blob([[header, body].join("\n")], { type: "text/csv" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "query-results.csv";
  a.click();
  URL.revokeObjectURL(url);
}

export default function QueryPage() {
  const [resultFilter, setResultFilter] = useState("");
  const columns = usePlatformStore((s) => s.columns);
  const rows = usePlatformStore((s) => s.rows);
  const queryError = usePlatformStore((s) => s.queryError);
  const lastExplainText = usePlatformStore((s) => s.lastExplainText);
  const lastMetrics = usePlatformStore((s) => s.lastMetrics);

  const resultColumns = useMemo<ColumnDef<Record<string, unknown>>[]>(
    () =>
      columns.map((col) => ({
        accessorKey: col,
        header: col,
        cell: ({ row }) => String(row.getValue(col) ?? ""),
      })),
    [columns],
  );

  const filteredRows = useMemo(() => {
    const q = resultFilter.trim().toLowerCase();
    if (!q) return rows;
    return rows.filter((row) =>
      Object.values(row).some((v) => String(v ?? "").toLowerCase().includes(q)),
    );
  }, [rows, resultFilter]);

  return (
    <div className="space-y-4">
      <SqlEditor
        onExportCsv={
          columns.length > 0
            ? () => exportCsv(columns, filteredRows)
            : undefined
        }
        onCopyJson={
          rows.length > 0
            ? () => void navigator.clipboard.writeText(JSON.stringify(filteredRows, null, 2))
            : undefined
        }
      />

      {queryError && (
        <div className="rounded-md border border-destructive/50 bg-destructive/20 p-2 text-sm text-destructive-foreground">
          {queryError}
        </div>
      )}

      <ExplainPanel text={lastExplainText} />
      <QueryMetricsCard metrics={lastMetrics} />

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <div>
            <CardTitle>Result Grid</CardTitle>
            <CardDescription>Tabular response projection</CardDescription>
          </div>
          <Badge variant="outline">{columns.length} cols</Badge>
        </CardHeader>
        <CardContent className="space-y-3">
          <Input
            placeholder="Filter results…"
            value={resultFilter}
            onChange={(e) => setResultFilter(e.target.value)}
            className="max-w-sm"
          />
          <DataTable
            columns={resultColumns}
            data={filteredRows}
            emptyMessage="Run a query to see results"
            pageSize={50}
          />
        </CardContent>
      </Card>
    </div>
  );
}
