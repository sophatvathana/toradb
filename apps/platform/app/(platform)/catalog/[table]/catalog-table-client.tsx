"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { type ColumnDef } from "@tanstack/react-table";

import { ConfirmDialog } from "@/components/confirm-dialog";
import { DataTable } from "@/components/data-table";
import { TableSearchInput } from "@/components/table-search-input";
import { matchesSearchQuery } from "@/lib/search";
import { useToast } from "@/components/toast-provider";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { runSql } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

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

export function CatalogTableClient({ tableName }: { tableName: string }) {
  const { toast } = useToast();
  const tableDetail = usePlatformStore((s) => s.tableDetail);
  const tableIndexes = usePlatformStore((s) => s.tableIndexes);
  const sampleColumns = usePlatformStore((s) => s.sampleColumns);
  const sampleRows = usePlatformStore((s) => s.sampleRows);
  const fetchTableDetailAction = usePlatformStore((s) => s.fetchTableDetailAction);
  const fetchTableSampleAction = usePlatformStore((s) => s.fetchTableSampleAction);
  const fetchTableIndexesAction = usePlatformStore((s) => s.fetchTableIndexesAction);
  const finishTableAction = usePlatformStore((s) => s.finishTableAction);
  const resumeTableAction = usePlatformStore((s) => s.resumeTableAction);
  const dropTableAction = usePlatformStore((s) => s.dropTableAction);
  const compactTableAction = usePlatformStore((s) => s.compactTableAction);
  const setSelectedTable = usePlatformStore((s) => s.setSelectedTable);
  const setIngestTable = usePlatformStore((s) => s.setIngestTable);

  const [dropOpen, setDropOpen] = useState(false);
  const [compactFull, setCompactFull] = useState(false);
  const [tab, setTab] = useState("overview");
  const [sampleFilter, setSampleFilter] = useState("");
  const [editColName, setEditColName] = useState("");
  const [editColType, setEditColType] = useState<string>("int");
  const [typeAlterLoading, setTypeAlterLoading] = useState(false);

  useEffect(() => {
    if (tableName) {
      void fetchTableDetailAction(tableName);
      void fetchTableSampleAction(tableName);
    }
  }, [tableName, fetchTableDetailAction, fetchTableSampleAction]);

  useEffect(() => {
    if (tab === "indexes" && tableName) {
      void fetchTableIndexesAction(tableName);
    }
  }, [tab, tableName, fetchTableIndexesAction]);

  const sampleColDefs = useMemo<ColumnDef<Record<string, unknown>>[]>(
    () =>
      sampleColumns.map((col) => ({
        accessorKey: col,
        header: col,
        cell: ({ row }) => String(row.getValue(col) ?? ""),
      })),
    [sampleColumns],
  );

  const indexColDefs = useMemo<ColumnDef<Record<string, unknown>>[]>(
    () => [
      {
        accessorKey: "index",
        header: "Index / sidecar",
        cell: ({ row }) => String(row.getValue("index") ?? ""),
      },
    ],
    [],
  );

  const detail = tableDetail?.name === tableName ? tableDetail : null;

  const filteredSampleRows = useMemo(() => {
    const q = sampleFilter.trim();
    if (!q) return sampleRows;
    return sampleRows.filter((row) =>
      matchesSearchQuery(q, Object.values(row) as (string | number | null | undefined)[]),
    );
  }, [sampleRows, sampleFilter]);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold">{tableName}</h2>
          <p className="text-sm text-muted-foreground">Table detail</p>
        </div>
        <Button variant="outline" size="sm" asChild>
          <Link href="/catalog">Back to catalog</Link>
        </Button>
      </div>

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="sample">Sample</TabsTrigger>
          <TabsTrigger value="indexes">Indexes</TabsTrigger>
          <TabsTrigger value="sidecars">Sidecars</TabsTrigger>
        </TabsList>

        <TabsContent value="overview">
          {detail && (
            <div className="grid grid-cols-3 gap-3">
              <Card>
                <CardHeader className="pb-1">
                  <CardTitle className="text-sm font-medium text-muted-foreground">Rows</CardTitle>
                </CardHeader>
                <CardContent className="text-2xl font-semibold">{detail.rows}</CardContent>
              </Card>
              <Card>
                <CardHeader className="pb-1">
                  <CardTitle className="text-sm font-medium text-muted-foreground">
                    Segments
                  </CardTitle>
                </CardHeader>
                <CardContent className="text-2xl font-semibold">{detail.segment_count}</CardContent>
              </Card>
              <Card>
                <CardHeader className="pb-1">
                  <CardTitle className="text-sm font-medium text-muted-foreground">State</CardTitle>
                </CardHeader>
                <CardContent>
                  <Badge variant={detail.state === "ready" ? "success" : "secondary"}>
                    {detail.state}
                  </Badge>
                </CardContent>
              </Card>
            </div>
          )}
          {detail && (
            <Card className="mt-3">
              <CardHeader>
                <CardTitle className="text-base">Configuration</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm">
                <p>
                  <span className="text-muted-foreground">Query mode:</span> {detail.query_mode}
                </p>
                <p>
                  <span className="text-muted-foreground">Segment workers:</span>{" "}
                  {detail.segment_workers}
                </p>
                <p>
                  <span className="text-muted-foreground">Vector dim:</span>{" "}
                  {detail.vector_dim ?? "—"}
                </p>
                <p>
                  <span className="text-muted-foreground">Bulk ingest:</span>{" "}
                  {detail.bulk_ingest_active ? "active" : "no"}
                </p>
              </CardContent>
            </Card>
          )}
          {detail && (
            <Card className="mt-3">
              <CardHeader>
                <CardTitle className="text-base">Column types</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-sm">
                {detail.column_types.length > 0 ? (
                  <div className="space-y-1">
                    {detail.column_types.map((c) => (
                      <p key={c.name} className="flex items-center gap-2">
                        <span className="font-mono">{c.name}</span>
                        <Badge variant="outline">{c.type}</Badge>
                      </p>
                    ))}
                  </div>
                ) : (
                  <p className="text-muted-foreground">
                    No typed columns declared. Metadata filters use legacy string heuristics.
                  </p>
                )}
                <div className="flex flex-wrap items-end gap-2 border-t pt-3">
                  <div className="space-y-1">
                    <label className="text-xs text-muted-foreground">Column</label>
                    <Input
                      className="h-8 w-36 font-mono text-xs"
                      placeholder="rank"
                      value={editColName}
                      onChange={(e) => setEditColName(e.target.value)}
                    />
                  </div>
                  <div className="space-y-1">
                    <label className="text-xs text-muted-foreground">Type</label>
                    <select
                      className="h-8 rounded-md border bg-background px-2 text-xs"
                      value={editColType}
                      onChange={(e) => setEditColType(e.target.value)}
                    >
                      {COLUMN_TYPES.map((t) => (
                        <option key={t} value={t}>
                          {t}
                        </option>
                      ))}
                    </select>
                  </div>
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={typeAlterLoading || !editColName.trim()}
                    onClick={() => {
                      const col = editColName.trim();
                      const sql = `ALTER TABLE ${tableName} ALTER COLUMN ${col} TYPE ${editColType}`;
                      setTypeAlterLoading(true);
                      void runSql(sql)
                        .then(async () => {
                          toast({
                            title: "Column type updated",
                            description: `${col} → ${editColType}`,
                          });
                          await fetchTableDetailAction(tableName);
                        })
                        .catch((err) => {
                          toast({
                            title: "ALTER failed",
                            description:
                              err instanceof Error ? err.message : String(err),
                            variant: "destructive",
                          });
                        })
                        .finally(() => setTypeAlterLoading(false));
                    }}
                  >
                    {typeAlterLoading ? "Applying…" : "Set type"}
                  </Button>
                </div>
              </CardContent>
            </Card>
          )}
        </TabsContent>

        <TabsContent value="sample">
          <Card>
            <CardHeader>
              <CardTitle>Sample rows</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <TableSearchInput
                value={sampleFilter}
                onChange={setSampleFilter}
                placeholder="Filter sample rows…"
              />
              <DataTable
                columns={sampleColDefs}
                data={filteredSampleRows}
                emptyMessage={
                  sampleFilter.trim() ? "No rows match your filter" : "No sample data"
                }
                pageSize={20}
              />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="indexes">
          <Card>
            <CardHeader>
              <CardTitle>Indexes</CardTitle>
            </CardHeader>
            <CardContent>
              <DataTable
                columns={indexColDefs}
                data={(tableIndexes?.rows ?? []) as Record<string, unknown>[]}
                emptyMessage="No indexes reported"
              />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="sidecars">
          <Card>
            <CardHeader>
              <CardTitle>Index sidecars on disk</CardTitle>
            </CardHeader>
            <CardContent>
              {detail && detail.index_sidecars.length > 0 ? (
                <ul className="list-inside list-disc text-sm">
                  {detail.index_sidecars.map((s) => (
                    <li key={s} className="font-mono text-xs">
                      {s}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-muted-foreground">No sidecars</p>
              )}
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>

      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          onClick={() => {
            void finishTableAction(tableName).then(() =>
              toast({ title: "Finish started", description: tableName }),
            );
          }}
        >
          Finish index
        </Button>
        <Button
          type="button"
          variant="secondary"
          onClick={() => {
            void resumeTableAction(tableName).then(() =>
              toast({ title: "Resume started", description: tableName }),
            );
          }}
        >
          Resume index
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            void compactTableAction(tableName, compactFull).then(() =>
              toast({ title: "Compact started", description: tableName }),
            );
          }}
        >
          Compact{compactFull ? " (full)" : ""}
        </Button>
        <label className="flex items-center gap-1 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={compactFull}
            onChange={(e) => setCompactFull(e.target.checked)}
          />
          Full compact
        </label>
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            setSelectedTable(tableName);
            setIngestTable(tableName);
          }}
          asChild
        >
          <Link href="/ingest">Ingest data</Link>
        </Button>
        <Button type="button" variant="outline" onClick={() => setSelectedTable(tableName)} asChild>
          <Link href="/query">Open in Query</Link>
        </Button>
        <Button type="button" variant="outline" onClick={() => setSelectedTable(tableName)} asChild>
          <Link href={`/search?table=${encodeURIComponent(tableName)}`}>Search</Link>
        </Button>
        <Button type="button" variant="outline" asChild>
          <Link href={`/schema?table=${encodeURIComponent(tableName)}`}>Schema</Link>
        </Button>
        <Button type="button" variant="outline" onClick={() => setDropOpen(true)}>
          Drop table
        </Button>
      </div>

      <ConfirmDialog
        open={dropOpen}
        onOpenChange={setDropOpen}
        title={`Drop table ${tableName}?`}
        description="This permanently removes the table directory from disk."
        confirmLabel="Drop"
        destructive
        onConfirm={async () => {
          await dropTableAction(tableName);
          toast({ title: "Table dropped", description: tableName });
        }}
      />
    </div>
  );
}
