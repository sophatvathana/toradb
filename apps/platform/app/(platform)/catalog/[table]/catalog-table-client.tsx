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
import { deleteDocuments, runSql } from "@/lib/api";
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
  const [editVectorDim, setEditVectorDim] = useState("");
  const [alterRewrite, setAlterRewrite] = useState(false);
  const [typeAlterLoading, setTypeAlterLoading] = useState(false);
  const [indexName, setIndexName] = useState("");
  const [indexColumn, setIndexColumn] = useState("text");
  const [indexUsing, setIndexUsing] = useState("BM25");
  const [indexCreateLoading, setIndexCreateLoading] = useState(false);
  const [workersInput, setWorkersInput] = useState("");
  const [workersLoading, setWorkersLoading] = useState(false);
  const [deleteIds, setDeleteIds] = useState("");
  const [deleteLoading, setDeleteLoading] = useState(false);

  function formatAlterType(type: string, vectorDim: string): string {
    if (type === "vector" && vectorDim.trim()) {
      return `vector(${vectorDim.trim()})`;
    }
    return type;
  }

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

                {/* Segment workers editor — ALTER TABLE … SET SEGMENT_WORKERS */}
                <div className="flex flex-wrap items-end gap-2 border-t pt-3">
                  <div className="space-y-1">
                    <label className="text-xs text-muted-foreground">Segment workers</label>
                    <Input
                      className="h-8 w-24 text-xs"
                      type="number"
                      min={1}
                      placeholder={String(detail.segment_workers)}
                      value={workersInput}
                      onChange={(e) => setWorkersInput(e.target.value)}
                    />
                  </div>
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={workersLoading || !workersInput.trim()}
                    onClick={() => {
                      const n = Number(workersInput);
                      if (!Number.isInteger(n) || n < 1) {
                        toast({
                          title: "Invalid value",
                          description: "Segment workers must be a positive integer.",
                          variant: "error",
                        });
                        return;
                      }
                      setWorkersLoading(true);
                      void runSql(
                        `ALTER TABLE ${tableName} SET SEGMENT_WORKERS = ${n}`,
                      )
                        .then(async () => {
                          toast({
                            title: "Segment workers updated",
                            description: `${tableName} → ${n} workers`,
                          });
                          setWorkersInput("");
                          await fetchTableDetailAction(tableName);
                        })
                        .catch((err) => {
                          toast({
                            title: "Update failed",
                            description: err instanceof Error ? err.message : String(err),
                            variant: "error",
                          });
                        })
                        .finally(() => setWorkersLoading(false));
                    }}
                  >
                    {workersLoading ? "Applying…" : "Set workers"}
                  </Button>
                  <p className="self-end pb-1 text-xs text-muted-foreground">
                    Parallel segment scan threads for distributed search.
                  </p>
                </div>

                {/* Soft-delete documents by id — POST /api/tables/{name}/delete */}
                <div className="flex flex-wrap items-end gap-2 border-t pt-3">
                  <div className="flex-1 space-y-1">
                    <label className="text-xs text-muted-foreground">
                      Delete documents by id
                    </label>
                    <Input
                      className="h-8 font-mono text-xs"
                      placeholder="comma-separated ids, e.g. 3, 17, 42"
                      value={deleteIds}
                      onChange={(e) => setDeleteIds(e.target.value)}
                    />
                  </div>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled={deleteLoading || !deleteIds.trim()}
                    onClick={() => {
                      const ids = deleteIds
                        .split(",")
                        .map((s) => Number(s.trim()))
                        .filter((n) => Number.isInteger(n) && n >= 0);
                      if (ids.length === 0) {
                        toast({
                          title: "No valid ids",
                          description: "Enter comma-separated non-negative integers.",
                          variant: "error",
                        });
                        return;
                      }
                      setDeleteLoading(true);
                      void deleteDocuments(tableName, ids)
                        .then(async (res) => {
                          toast({
                            title: "Documents deleted",
                            description: `Soft-deleted ${res.deleted} of ${ids.length} id(s).`,
                          });
                          setDeleteIds("");
                          await fetchTableDetailAction(tableName);
                          await fetchTableSampleAction(tableName);
                        })
                        .catch((err) => {
                          toast({
                            title: "Delete failed",
                            description: err instanceof Error ? err.message : String(err),
                            variant: "error",
                          });
                        })
                        .finally(() => setDeleteLoading(false));
                    }}
                  >
                    {deleteLoading ? "Deleting…" : "Delete"}
                  </Button>
                  <p className="self-end pb-1 text-xs text-muted-foreground">
                    Soft delete (tombstoned; reclaimed on compaction).
                  </p>
                </div>
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
                {detail.needs_segment_rewrite && (
                  <p className="rounded-md border border-warning/40 bg-warning/10 p-2 text-xs text-warning">
                    Segments use the legacy layout. Run{" "}
                    <strong>Compact (full)</strong> below or enable{" "}
                    <strong>Rewrite segments</strong> when altering a column type.
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
                      onChange={(e) => {
                        setEditColType(e.target.value);
                        if (e.target.value !== "vector") setEditVectorDim("");
                      }}
                    >
                      {COLUMN_TYPES.map((t) => (
                        <option key={t} value={t}>
                          {t}
                        </option>
                      ))}
                    </select>
                  </div>
                  {editColType === "vector" && (
                    <div className="space-y-1">
                      <label className="text-xs text-muted-foreground">Vector dim</label>
                      <Input
                        className="h-8 text-xs"
                        value={editVectorDim}
                        placeholder="384"
                        onChange={(e) => setEditVectorDim(e.target.value)}
                      />
                    </div>
                  )}
                  <label className="flex items-center gap-1 self-end pb-1 text-xs text-muted-foreground">
                    <input
                      type="checkbox"
                      checked={alterRewrite}
                      onChange={(e) => setAlterRewrite(e.target.checked)}
                    />
                    Rewrite segments
                  </label>
                  <Button
                    type="button"
                    size="sm"
                    variant="secondary"
                    disabled={typeAlterLoading || !editColName.trim()}
                    onClick={() => {
                      const col = editColName.trim();
                      const typeSql = formatAlterType(editColType, editVectorDim);
                      const rewriteClause = alterRewrite ? " REWRITE" : "";
                      const sql = `ALTER TABLE ${tableName} ALTER COLUMN ${col} TYPE ${typeSql}${rewriteClause}`;
                      setTypeAlterLoading(true);
                      void runSql(sql)
                        .then(async () => {
                          toast({
                            title: "Column type updated",
                            description: `${col} → ${typeSql}`,
                          });
                          await fetchTableDetailAction(tableName);
                        })
                        .catch((err) => {
                          toast({
                            title: "ALTER failed",
                            description:
                              err instanceof Error ? err.message : String(err),
                            variant: "error",
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
            <CardContent className="space-y-3">
              <DataTable
                columns={indexColDefs}
                data={(tableIndexes?.rows ?? []) as Record<string, unknown>[]}
                emptyMessage="No indexes reported"
              />
              <div className="flex flex-wrap items-end gap-2 border-t pt-3">
                <div className="space-y-1">
                  <label className="text-xs text-muted-foreground">Index name</label>
                  <Input
                    className="h-8 w-32 font-mono text-xs"
                    placeholder="idx_text"
                    value={indexName}
                    onChange={(e) => setIndexName(e.target.value)}
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs text-muted-foreground">Column</label>
                  <Input
                    className="h-8 w-28 font-mono text-xs"
                    value={indexColumn}
                    onChange={(e) => setIndexColumn(e.target.value)}
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs text-muted-foreground">USING</label>
                  <select
                    className="h-8 rounded-md border bg-background px-2 text-xs"
                    value={indexUsing}
                    onChange={(e) => setIndexUsing(e.target.value)}
                  >
                    {["BM25", "HNSW", "DISKANN", "HYBRID"].map((u) => (
                      <option key={u} value={u}>
                        {u}
                      </option>
                    ))}
                  </select>
                </div>
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  disabled={indexCreateLoading || !indexName.trim()}
                  onClick={() => {
                    const name = indexName.trim();
                    const col = indexColumn.trim() || "text";
                    const sql = `CREATE INDEX ${name} ON ${tableName} (${col}) USING ${indexUsing}`;
                    setIndexCreateLoading(true);
                    void runSql(sql)
                      .then(async () => {
                        toast({ title: "Index created", description: name });
                        await fetchTableIndexesAction(tableName);
                        await fetchTableDetailAction(tableName);
                      })
                      .catch((err) => {
                        toast({
                          title: "CREATE INDEX failed",
                          description: err instanceof Error ? err.message : String(err),
                          variant: "error",
                        });
                      })
                      .finally(() => setIndexCreateLoading(false));
                  }}
                >
                  {indexCreateLoading ? "Creating…" : "Create index"}
                </Button>
              </div>
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
