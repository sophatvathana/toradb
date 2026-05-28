"use client";

import Link from "next/link";
import { type ColumnDef } from "@tanstack/react-table";
import { useMemo } from "react";

import type { TableInfo } from "@/lib/api";

import { DataTable } from "@/components/data-table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { usePlatformStore } from "@/stores/platform-store";

export default function CatalogPage() {
  const tables = usePlatformStore((s) => s.tables);
  const setSelectedTable = usePlatformStore((s) => s.setSelectedTable);

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
      <CardContent>
        <DataTable columns={columns} data={tables} emptyMessage="No tables in database" />
      </CardContent>
    </Card>
  );
}
