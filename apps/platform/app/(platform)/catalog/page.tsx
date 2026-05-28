"use client";

import Link from "next/link";
import { type ColumnDef } from "@tanstack/react-table";
import { useMemo, useState } from "react";

import type { TableInfo } from "@/lib/api";

import { DataTable } from "@/components/data-table";
import { TableSearchInput } from "@/components/table-search-input";
import { matchesSearchQuery } from "@/lib/search";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { usePlatformStore } from "@/stores/platform-store";

export default function CatalogPage() {
  const tables = usePlatformStore((s) => s.tables);
  const setSelectedTable = usePlatformStore((s) => s.setSelectedTable);
  const [search, setSearch] = useState("");

  const filteredTables = useMemo(
    () =>
      tables.filter((t) =>
        matchesSearchQuery(search, [t.name, t.state, t.rows, t.vector_dim]),
      ),
    [tables, search],
  );

  const columns = useMemo<ColumnDef<TableInfo>[]>(
    () => [
      {
        accessorKey: "name",
        header: "Table",
        cell: ({ row }) => (
          <Link
            href={`/catalog/${encodeURIComponent(row.original.name)}`}
            className="font-medium text-primary hover:underline"
          >
            {row.original.name}
          </Link>
        ),
      },
      { accessorKey: "rows", header: "Rows" },
      {
        accessorKey: "vector_dim",
        header: "Vector Dim",
        cell: ({ row }) => row.original.vector_dim ?? "—",
      },
      {
        accessorKey: "state",
        header: "State",
        cell: ({ row }) => (
          <Badge
            variant={
              row.original.state === "ready"
                ? "success"
                : row.original.state === "failed"
                  ? "warning"
                  : "secondary"
            }
          >
            {row.original.state}
          </Badge>
        ),
      },
      {
        id: "actions",
        header: "",
        cell: ({ row }) => (
          <Button variant="outline" size="sm" asChild>
            <Link href="/query" onClick={() => setSelectedTable(row.original.name)}>
              Use in Query
            </Link>
          </Button>
        ),
      },
    ],
    [setSelectedTable],
  );

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle>Catalog Explorer</CardTitle>
        <Button variant="outline" size="sm" asChild>
          <Link href="/schema">Create table</Link>
        </Button>
      </CardHeader>
      <CardContent className="space-y-3">
        <TableSearchInput
          value={search}
          onChange={setSearch}
          placeholder="Search tables…"
        />
        <DataTable
          columns={columns}
          data={filteredTables}
          emptyMessage={search.trim() ? "No tables match your search" : "No tables in database"}
        />
      </CardContent>
    </Card>
  );
}
