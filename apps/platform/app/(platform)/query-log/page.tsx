"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { type ColumnDef } from "@tanstack/react-table";

import { DataTable } from "@/components/data-table";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { QueryHistoryEntry } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

export default function QueryLogPage() {
  const history = usePlatformStore((s) => s.history);
  const openQueryFromHistory = usePlatformStore((s) => s.openQueryFromHistory);
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [kindFilter, setKindFilter] = useState<string>("all");
  const [search, setSearch] = useState("");

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    return history.filter((h) => {
      if (statusFilter !== "all" && h.status !== statusFilter) return false;
      if (kindFilter !== "all" && h.kind !== kindFilter) return false;
      if (q && !h.query.toLowerCase().includes(q)) return false;
      return true;
    });
  }, [history, statusFilter, kindFilter, search]);

  function exportVisibleCsv() {
    const header = "query,status,kind,latency_ms,executed_at\n";
    const body = filtered
      .map((h) =>
        [
          JSON.stringify(h.query),
          h.status,
          h.kind,
          h.latency_ms,
          h.executed_at_unix_secs,
        ].join(","),
      )
      .join("\n");
    const blob = new Blob([header + body], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "query-log.csv";
    a.click();
    URL.revokeObjectURL(url);
  }

  const columns = useMemo<ColumnDef<QueryHistoryEntry>[]>(
    () => [
      {
        accessorKey: "query",
        header: "Query",
        cell: ({ row }) => (
          <span className="block max-w-xl truncate font-mono text-xs">{row.original.query}</span>
        ),
      },
      {
        accessorKey: "status",
        header: "Status",
        cell: ({ row }) => (
          <Badge variant={row.original.status === "ok" ? "success" : "warning"}>
            {row.original.status}
          </Badge>
        ),
      },
      { accessorKey: "kind", header: "Kind" },
      {
        accessorKey: "latency_ms",
        header: "Latency",
        cell: ({ row }) => `${Math.round(row.original.latency_ms)} ms`,
      },
      {
        accessorKey: "executed_at_unix_secs",
        header: "Executed",
        cell: ({ row }) =>
          new Date(row.original.executed_at_unix_secs * 1000).toLocaleString(),
      },
      {
        id: "actions",
        header: "",
        cell: ({ row }) => (
          <Button variant="outline" size="sm" asChild>
            <Link
              href="/query"
              onClick={() => openQueryFromHistory(row.original.query)}
            >
              Open in Query
            </Link>
          </Button>
        ),
      },
    ],
    [openQueryFromHistory],
  );

  return (
    <Card>
      <CardHeader className="flex-row flex-wrap items-center justify-between gap-2">
        <CardTitle>Recent Query Log</CardTitle>
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <Input
            className="h-8 w-48"
            placeholder="Search queries…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <Button type="button" variant="outline" size="sm" onClick={exportVisibleCsv}>
            Export CSV
          </Button>
          <select
            className="rounded-md border border-input bg-background px-2 py-1"
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value)}
          >
            <option value="all">All statuses</option>
            <option value="ok">ok</option>
            <option value="error">error</option>
          </select>
          <select
            className="rounded-md border border-input bg-background px-2 py-1"
            value={kindFilter}
            onChange={(e) => setKindFilter(e.target.value)}
          >
            <option value="all">All kinds</option>
            <option value="search">search</option>
            <option value="aggregate">aggregate</option>
            <option value="explain">explain</option>
            <option value="sql">sql</option>
          </select>
        </div>
      </CardHeader>
      <CardContent>
        <DataTable columns={columns} data={filtered} emptyMessage="No queries yet" />
      </CardContent>
    </Card>
  );
}
