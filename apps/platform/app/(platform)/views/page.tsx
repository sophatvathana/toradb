"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { type ColumnDef } from "@tanstack/react-table";

import { ConfirmDialog } from "@/components/confirm-dialog";
import { DataTable } from "@/components/data-table";
import { useToast } from "@/components/toast-provider";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { runSql, SQL_TEMPLATES, type MaterializedViewInfo } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

export default function ViewsPage() {
  const { toast } = useToast();
  const materializedViews = usePlatformStore((s) => s.materializedViews);
  const refreshMaterializedViews = usePlatformStore((s) => s.refreshMaterializedViews);
  const createMvAction = usePlatformStore((s) => s.createMvAction);
  const refreshMvAction = usePlatformStore((s) => s.refreshMvAction);
  const dropMvAction = usePlatformStore((s) => s.dropMvAction);
  const setSql = usePlatformStore((s) => s.setSql);

  const [createOpen, setCreateOpen] = useState(false);
  const [mvName, setMvName] = useState("top_docs");
  const [mvQuery, setMvQuery] = useState<string>(SQL_TEMPLATES[0].sql);
  const [selected, setSelected] = useState<MaterializedViewInfo | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [sampleRows, setSampleRows] = useState<Record<string, unknown>[]>([]);

  const columns = useMemo<ColumnDef<MaterializedViewInfo>[]>(
    () => [
      {
        accessorKey: "name",
        header: "View",
        cell: ({ row }) => (
          <button
            type="button"
            className="font-medium text-primary hover:underline"
            onClick={() => setSelected(row.original)}
          >
            {row.original.name}
          </button>
        ),
      },
      { accessorKey: "row_count", header: "Rows" },
      {
        accessorKey: "query",
        header: "Query",
        cell: ({ row }) => (
          <span className="block max-w-md truncate font-mono text-xs text-muted-foreground">
            {row.original.query}
          </span>
        ),
      },
      {
        id: "actions",
        header: "",
        cell: ({ row }) => (
          <div className="flex gap-1">
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => {
                void refreshMvAction(row.original.name).then(() =>
                  toast({ title: "Refreshed", description: row.original.name }),
                );
              }}
            >
              Refresh
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => {
                setSql(`SELECT id, score FROM ${row.original.name} LIMIT 10`);
                toast({ title: "Query loaded", description: "Open Query workbench" });
              }}
            >
              Query
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => setDropTarget(row.original.name)}
            >
              Drop
            </Button>
          </div>
        ),
      },
    ],
    [refreshMvAction, setSql, toast],
  );

  async function loadSample(name: string) {
    const res = await runSql(`SELECT id, score FROM ${name} LIMIT 10`);
    setSampleRows((res.rows ?? []) as Record<string, unknown>[]);
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold">Materialized views</h2>
          <p className="text-sm text-muted-foreground">Cached retrieval results</p>
        </div>
        <div className="flex gap-2">
          <Button type="button" variant="outline" size="sm" onClick={() => void refreshMaterializedViews()}>
            Refresh list
          </Button>
          <Button type="button" size="sm" onClick={() => setCreateOpen(true)}>
            Create view
          </Button>
        </div>
      </div>

      <Card>
        <CardContent className="pt-6">
          <DataTable
            columns={columns}
            data={materializedViews}
            emptyMessage="No materialized views"
          />
        </CardContent>
      </Card>

      {createOpen && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">New materialized view</CardTitle>
            <CardDescription>SELECT query must be retrieval-only (no GROUP BY)</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <Input
              placeholder="view name"
              value={mvName}
              onChange={(e) => setMvName(e.target.value)}
            />
            <Textarea
              className="min-h-[120px] font-mono text-xs"
              value={mvQuery}
              onChange={(e) => setMvQuery(e.target.value)}
            />
            <div className="flex gap-2">
              <Button
                type="button"
                onClick={() => {
                  void createMvAction(mvName, mvQuery)
                    .then(() => {
                      toast({ title: "Created", description: mvName });
                      setCreateOpen(false);
                    })
                    .catch((err) =>
                      toast({
                        title: "Create failed",
                        description: String(err),
                        variant: "error",
                      }),
                    );
                }}
              >
                Create
              </Button>
              <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {selected && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{selected.name}</CardTitle>
            <CardDescription>
              <Badge variant="secondary">{selected.row_count} rows</Badge>
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <pre className="overflow-x-auto rounded-md border border-border bg-muted/30 p-3 font-mono text-xs">
              {selected.query}
            </pre>
            <div className="flex gap-2">
              <Button type="button" size="sm" variant="outline" onClick={() => void loadSample(selected.name)}>
                Load sample
              </Button>
              <Button type="button" size="sm" variant="outline" asChild>
                <Link href="/query" onClick={() => setSql(`SELECT id, score FROM ${selected.name} LIMIT 10`)}>
                  Open in Query
                </Link>
              </Button>
              <Button type="button" size="sm" variant="ghost" onClick={() => setSelected(null)}>
                Close
              </Button>
            </div>
            {sampleRows.length > 0 && (
              <pre className="max-h-48 overflow-auto rounded-md border border-border p-2 text-xs">
                {JSON.stringify(sampleRows, null, 2)}
              </pre>
            )}
          </CardContent>
        </Card>
      )}

      <ConfirmDialog
        open={dropTarget !== null}
        onOpenChange={(open) => !open && setDropTarget(null)}
        title="Drop materialized view?"
        description={`Permanently drop ${dropTarget}?`}
        confirmLabel="Drop"
        destructive
        onConfirm={() => {
          if (!dropTarget) return;
          void dropMvAction(dropTarget).then(() => {
            toast({ title: "Dropped", description: dropTarget });
            setDropTarget(null);
            if (selected?.name === dropTarget) setSelected(null);
          });
        }}
      />
    </div>
  );
}
