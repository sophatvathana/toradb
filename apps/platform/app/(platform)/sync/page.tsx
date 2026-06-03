"use client";

import { useCallback, useEffect, useState } from "react";

import { ConfirmDialog } from "@/components/confirm-dialog";
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
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import {
  buildSelect,
  createPipeline,
  type CreatePipelineBody as CreatePipelineSpec,
  deletePipeline,
  fetchConnectionColumns,
  fetchConnections,
  fetchConnectionTables,
  fetchPipelineRuns,
  fetchPipelines,
  fetchSyncJob,
  introspectConnection,
  patchPipeline,
  runPipeline,
  type Connection,
  type Pipeline,
  type PipelineRun,
  type SourceField,
} from "@/lib/api";

type ColRole = "metadata" | "text" | "vector" | "id" | "cursor" | "skip";

type ColumnChoice = {
  name: string;
  dataType: string;
  include: boolean;
  alias: string;
  role: ColRole;
};

export default function SyncPage() {
  const { toast } = useToast();
  const [connections, setConnections] = useState<Connection[]>([]);
  const [pipelines, setPipelines] = useState<Pipeline[]>([]);
  const [loading, setLoading] = useState(true);
  const [createOpen, setCreateOpen] = useState(false);
  const [dropTarget, setDropTarget] = useState<Pipeline | null>(null);
  const [runsFor, setRunsFor] = useState<Pipeline | null>(null);
  const [runs, setRuns] = useState<PipelineRun[]>([]);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [conns, pipes] = await Promise.all([fetchConnections(), fetchPipelines()]);
      setConnections(conns);
      setPipelines(pipes);
    } catch (err) {
      toast({ title: "Failed to load sync state", description: String(err), variant: "error" });
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onRun = async (p: Pipeline) => {
    try {
      const { job_id } = await runPipeline(p.id);
      toast({ title: `Sync started for ${p.name}` });
      void pollJob(job_id, p.name);
    } catch (err) {
      toast({ title: "Failed to start sync", description: String(err), variant: "error" });
    }
  };

  const pollJob = async (jobId: number, name: string) => {
    for (let i = 0; i < 600; i++) {
      await new Promise((r) => setTimeout(r, 1000));
      try {
        const job = await fetchSyncJob(jobId);
        if (job.state === "done") {
          toast({ title: `${name}: synced ${job.rows_ingested} rows` });
          void refresh();
          return;
        }
        if (job.state === "failed") {
          toast({ title: `${name} failed`, description: job.message ?? "", variant: "error" });
          return;
        }
        if (job.state === "cancelled") return;
      } catch {
        return;
      }
    }
  };

  const onToggle = async (p: Pipeline, enabled: boolean) => {
    try {
      await patchPipeline(p.id, { enabled });
      await refresh();
    } catch (err) {
      toast({ title: "Failed to update", description: String(err), variant: "error" });
    }
  };

  const onDelete = async () => {
    if (!dropTarget) return;
    try {
      await deletePipeline(dropTarget.id);
      toast({ title: "Pipeline deleted" });
      await refresh();
    } catch (err) {
      toast({ title: "Failed to delete", description: String(err), variant: "error" });
    } finally {
      setDropTarget(null);
    }
  };

  const openRuns = async (p: Pipeline) => {
    setRunsFor(p);
    try {
      setRuns(await fetchPipelineRuns(p.id));
    } catch (err) {
      toast({ title: "Failed to load runs", description: String(err), variant: "error" });
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Sync</h1>
          <p className="text-sm text-muted-foreground">
            toraPipe pipelines: sync rows from a connection into a ToraDB table.
          </p>
        </div>
        <Button onClick={() => setCreateOpen((v) => !v)} disabled={connections.length === 0}>
          {createOpen ? "Cancel" : "New pipeline"}
        </Button>
      </div>

      {connections.length === 0 && !loading && (
        <Card>
          <CardContent className="py-6 text-sm text-muted-foreground">
            Create a connection first on the Connections page.
          </CardContent>
        </Card>
      )}

      {createOpen && (
        <NewPipelineForm
          connections={connections}
          onCreated={() => {
            setCreateOpen(false);
            void refresh();
          }}
          onError={(msg) => toast({ title: "Failed to create pipeline", description: msg, variant: "error" })}
          onSuccess={(name) => toast({ title: `Pipeline "${name}" created` })}
        />
      )}

      <Card>
        <CardContent className="p-0">
          <table className="w-full text-sm">
            <thead className="border-b text-left text-muted-foreground">
              <tr>
                <th className="px-4 py-2">Name</th>
                <th className="px-4 py-2">Target</th>
                <th className="px-4 py-2">Mode</th>
                <th className="px-4 py-2">Schedule</th>
                <th className="px-4 py-2">Cursor</th>
                <th className="px-4 py-2">Enabled</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td className="px-4 py-6 text-muted-foreground" colSpan={7}>
                    Loading…
                  </td>
                </tr>
              ) : pipelines.length === 0 ? (
                <tr>
                  <td className="px-4 py-6 text-muted-foreground" colSpan={7}>
                    No pipelines yet.
                  </td>
                </tr>
              ) : (
                pipelines.map((p) => (
                  <tr key={p.id} className="border-b last:border-0">
                    <td className="px-4 py-2 font-medium">{p.name}</td>
                    <td className="px-4 py-2">{p.target_table}</td>
                    <td className="px-4 py-2">
                      <Badge variant="secondary">{p.mode}</Badge>
                    </td>
                    <td className="px-4 py-2 text-muted-foreground">
                      {p.schedule_interval_secs ? `every ${p.schedule_interval_secs}s` : "manual"}
                    </td>
                    <td className="px-4 py-2 font-mono text-xs text-muted-foreground">
                      {p.last_cursor ?? "—"}
                    </td>
                    <td className="px-4 py-2">
                      <Switch
                        checked={p.enabled}
                        onCheckedChange={(v) => onToggle(p, v)}
                      />
                    </td>
                    <td className="px-4 py-2 text-right space-x-1">
                      <Button variant="outline" size="sm" onClick={() => onRun(p)}>
                        Run now
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => openRuns(p)}>
                        Runs
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setDropTarget(p)}>
                        Delete
                      </Button>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </CardContent>
      </Card>

      {runsFor && (
        <Card>
          <CardHeader>
            <CardTitle>Runs — {runsFor.name}</CardTitle>
            <CardDescription>Most recent sync executions.</CardDescription>
          </CardHeader>
          <CardContent className="p-0">
            <table className="w-full text-sm">
              <thead className="border-b text-left text-muted-foreground">
                <tr>
                  <th className="px-4 py-2">State</th>
                  <th className="px-4 py-2">Rows</th>
                  <th className="px-4 py-2">Started</th>
                  <th className="px-4 py-2">Cursor after</th>
                  <th className="px-4 py-2">Message</th>
                </tr>
              </thead>
              <tbody>
                {runs.length === 0 ? (
                  <tr>
                    <td className="px-4 py-4 text-muted-foreground" colSpan={5}>
                      No runs yet.
                    </td>
                  </tr>
                ) : (
                  runs.map((r) => (
                    <tr key={r.id} className="border-b last:border-0">
                      <td className="px-4 py-2">
                        <Badge
                          variant={
                            r.state === "failed"
                              ? "warning"
                              : r.state === "done"
                                ? "success"
                                : "secondary"
                          }
                        >
                          {r.state}
                        </Badge>
                      </td>
                      <td className="px-4 py-2">{r.rows_synced}</td>
                      <td className="px-4 py-2 text-muted-foreground">
                        {new Date(r.started_at * 1000).toLocaleString()}
                      </td>
                      <td className="px-4 py-2 font-mono text-xs">{r.cursor_after ?? "—"}</td>
                      <td className="px-4 py-2 text-muted-foreground">{r.message ?? ""}</td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </CardContent>
        </Card>
      )}

      <ConfirmDialog
        open={dropTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDropTarget(null);
        }}
        title="Delete pipeline?"
        description={`This removes "${dropTarget?.name ?? ""}" and its run history.`}
        confirmLabel="Delete"
        destructive
        onConfirm={onDelete}
      />
    </div>
  );
}

function NewPipelineForm({
  connections,
  onCreated,
  onError,
  onSuccess,
}: {
  connections: Connection[];
  onCreated: () => void;
  onError: (msg: string) => void;
  onSuccess: (name: string) => void;
}) {
  const [name, setName] = useState("");
  const [connectionId, setConnectionId] = useState(connections[0]?.id ?? "");
  const [buildMode, setBuildMode] = useState<"browse" | "query">("browse");

  // Browse-mode state: multiple tables can be selected; one is "active" (its
  // column mapping is shown/edited on the right). Column choices are kept per
  // table and loaded lazily on first selection.
  const [tables, setTables] = useState<string[]>([]);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [activeTable, setActiveTable] = useState<string | null>(null);
  const [tableCols, setTableCols] = useState<Record<string, ColumnChoice[]>>({});
  /// Optional prefix applied to per-table ToraDB target tables (`<prefix>_<table>`).
  const [targetPrefix, setTargetPrefix] = useState("");

  // Query-mode state
  const [query, setQuery] = useState("SELECT * FROM ");
  const [fields, setFields] = useState<SourceField[]>([]);
  const [textColumn, setTextColumn] = useState("");
  const [metadataColumns, setMetadataColumns] = useState("");
  const [vectorColumn, setVectorColumn] = useState("");
  const [idColumn, setIdColumn] = useState("");
  const [cursorColumn, setCursorColumn] = useState("");

  // Shared
  const [targetTable, setTargetTable] = useState("");
  const [incremental, setIncremental] = useState(false);
  const [scheduleSecs, setScheduleSecs] = useState("");
  const [dropTable, setDropTable] = useState(true);
  const [saving, setSaving] = useState(false);

  const loadTables = async () => {
    try {
      setTables(await fetchConnectionTables(connectionId));
    } catch (err) {
      onError(String(err));
    }
  };

  /// Lazily fetch and default a table's column mapping (once).
  const ensureColumns = async (table: string) => {
    if (tableCols[table]) return;
    try {
      const fetched = await fetchConnectionColumns(connectionId, table);
      // Heuristic defaults: first TEXT col = text; an `id` col = id.
      const firstText = fetched.find((f) => /char|text|string/i.test(f.data_type));
      const mapped: ColumnChoice[] = fetched.map((f) => ({
        name: f.name,
        dataType: f.data_type,
        include: true,
        alias: f.name,
        role:
          firstText && f.name === firstText.name
            ? "text"
            : f.name.toLowerCase() === "id"
              ? "id"
              : "metadata",
      }));
      setTableCols((prev) => ({ ...prev, [table]: mapped }));
    } catch (err) {
      onError(String(err));
    }
  };

  /// Toggle a table's inclusion in the batch (and load its columns / make active).
  const toggleTable = async (table: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(table)) next.delete(table);
      else next.add(table);
      return next;
    });
    setActiveTable(table);
    await ensureColumns(table);
  };

  /// Focus a table for editing without changing its inclusion.
  const focusTable = async (table: string) => {
    setActiveTable(table);
    await ensureColumns(table);
  };

  const setColRole = (table: string, idx: number, role: ColRole) => {
    setTableCols((prev) => {
      const cs = prev[table] ?? [];
      const updated = cs.map((c, i) => {
        if (i !== idx) {
          // Only one column may hold text/vector/id/cursor; demote others.
          if (
            (role === "text" || role === "vector" || role === "id" || role === "cursor") &&
            c.role === role
          ) {
            return { ...c, role: "metadata" as ColRole };
          }
          return c;
        }
        return { ...c, role, include: role !== "skip" };
      });
      return { ...prev, [table]: updated };
    });
  };

  const updateCol = (table: string, idx: number, patch: Partial<ColumnChoice>) => {
    setTableCols((prev) => {
      const cs = prev[table] ?? [];
      return { ...prev, [table]: cs.map((c, i) => (i === idx ? { ...c, ...patch } : c)) };
    });
  };

  const introspect = async () => {
    try {
      const f = await introspectConnection(connectionId, query);
      setFields(f);
      if (!textColumn && f.length) setTextColumn(f[0].name);
    } catch (err) {
      onError(String(err));
    }
  };

  const out = (c: ColumnChoice) => c.alias.trim() || c.name;

  /// Build the create-pipeline payload for one browse-mode table. Returns an
  /// error string if the table's mapping is invalid.
  const browsePayload = (table: string): { ok: CreatePipelineSpec } | { err: string } => {
    const cs = tableCols[table] ?? [];
    const picked = cs.filter((c) => c.include && c.role !== "skip");
    const text = picked.find((c) => c.role === "text");
    if (!text) return { err: `${table}: mark one column as the text column` };
    const byRole = (role: ColRole) => picked.find((c) => c.role === role);
    const vec = byRole("vector");
    const idc = byRole("id");
    const cur = byRole("cursor");
    if (incremental && !cur) {
      return { err: `${table}: incremental mode requires a cursor column` };
    }
    const target = targetPrefix.trim() ? `${targetPrefix.trim()}_${table}` : table;
    return {
      ok: {
        name: targetPrefix.trim() ? `${targetPrefix.trim()}-${table}` : table,
        connection_id: connectionId,
        query: buildSelect(
          table,
          picked.map((c) => ({ name: c.name, include: true, alias: c.alias })),
        ),
        target_table: target,
        text_column: out(text),
        metadata_columns: picked.filter((c) => c.role === "metadata").map(out),
        vector_column: vec ? out(vec) : null,
        id_column: idc ? out(idc) : null,
        cursor_column: cur ? out(cur) : null,
        mode: incremental ? "incremental" : "full",
        schedule_interval_secs: scheduleSecs ? Number(scheduleSecs) : null,
        drop_table_on_full: dropTable,
      },
    };
  };

  const save = async () => {
    let specs: CreatePipelineSpec[] = [];

    if (buildMode === "browse") {
      const selected = [...checked];
      if (selected.length === 0) {
        onError("select at least one source table");
        return;
      }
      for (const t of selected) {
        const r = browsePayload(t);
        if ("err" in r) {
          onError(r.err);
          return;
        }
        specs.push(r.ok);
      }
    } else {
      if (!name.trim() || !targetTable.trim() || !query.trim() || !textColumn.trim()) {
        onError("name, target table, query, and text column are required");
        return;
      }
      if (incremental && !cursorColumn.trim()) {
        onError("incremental mode requires a cursor column");
        return;
      }
      specs = [
        {
          name: name.trim(),
          connection_id: connectionId,
          query: query.trim(),
          target_table: targetTable.trim(),
          text_column: textColumn.trim(),
          metadata_columns: metadataColumns.split(",").map((s) => s.trim()).filter(Boolean),
          vector_column: vectorColumn.trim() || null,
          id_column: idColumn.trim() || null,
          cursor_column: cursorColumn.trim() || null,
          mode: incremental ? "incremental" : "full",
          schedule_interval_secs: scheduleSecs ? Number(scheduleSecs) : null,
          drop_table_on_full: dropTable,
        },
      ];
    }

    setSaving(true);
    let created = 0;
    try {
      for (const spec of specs) {
        await createPipeline(spec);
        created += 1;
      }
      onSuccess(
        specs.length === 1 ? specs[0].name : `${created} pipelines`,
      );
      onCreated();
    } catch (err) {
      onError(`created ${created}/${specs.length}; ${String(err)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>New pipeline</CardTitle>
        <CardDescription>
          Browse the connection&apos;s tables and map columns, or write a custom query.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid grid-cols-2 gap-3">
          {buildMode === "query" ? (
            <div className="space-y-1">
              <label className="text-sm font-medium">Name</label>
              <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="orders-sync" />
            </div>
          ) : (
            <div className="space-y-1">
              <label className="text-sm font-medium">Target name prefix (optional)</label>
              <Input
                value={targetPrefix}
                onChange={(e) => setTargetPrefix(e.target.value)}
                placeholder="(none → target = source table name)"
              />
            </div>
          )}
          <div className="space-y-1">
            <label className="text-sm font-medium">Connection</label>
            <select
              className="h-9 w-full rounded-md border border-border bg-transparent px-2 text-sm"
              value={connectionId}
              onChange={(e) => {
                setConnectionId(e.target.value);
                setTables([]);
                setChecked(new Set());
                setActiveTable(null);
                setTableCols({});
              }}
            >
              {connections.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </div>
        </div>

        <div className="flex gap-2">
          <Button
            variant={buildMode === "browse" ? "default" : "outline"}
            size="sm"
            onClick={() => setBuildMode("browse")}
          >
            Browse tables
          </Button>
          <Button
            variant={buildMode === "query" ? "default" : "outline"}
            size="sm"
            onClick={() => setBuildMode("query")}
          >
            Custom query
          </Button>
        </div>

        {buildMode === "browse" ? (
          <>
            <div className="grid grid-cols-[220px_1fr] gap-3">
              {/* Left: table list with multi-select checkboxes */}
              <div className="rounded-md border border-border">
                <div className="flex items-center justify-between border-b px-3 py-1.5">
                  <span className="text-xs font-medium text-muted-foreground">
                    Tables {checked.size > 0 ? `(${checked.size} selected)` : ""}
                  </span>
                  <Button variant="ghost" size="sm" onClick={loadTables} disabled={!connectionId}>
                    Load
                  </Button>
                </div>
                <ul className="max-h-72 overflow-y-auto text-sm">
                  {tables.length === 0 ? (
                    <li className="px-3 py-2 text-muted-foreground">Click Load.</li>
                  ) : (
                    tables.map((t) => (
                      <li
                        key={t}
                        className={`flex items-center gap-2 px-3 py-1.5 hover:bg-accent/20 ${
                          activeTable === t ? "bg-accent/30" : ""
                        }`}
                      >
                        <input
                          type="checkbox"
                          checked={checked.has(t)}
                          onChange={() => toggleTable(t)}
                          aria-label={`Select ${t}`}
                        />
                        <button
                          type="button"
                          onClick={() => focusTable(t)}
                          className={`flex-1 text-left ${activeTable === t ? "font-medium" : ""}`}
                        >
                          {t}
                        </button>
                      </li>
                    ))
                  )}
                </ul>
              </div>

              {/* Right: column mapping for the active table */}
              <div className="rounded-md border border-border">
                <div className="border-b px-3 py-1.5 text-xs font-medium text-muted-foreground">
                  {activeTable
                    ? `Columns of ${activeTable}${checked.has(activeTable) ? "" : " (not selected — tick the box to include)"}`
                    : "Select a table"}
                </div>
                {!activeTable || !(tableCols[activeTable]?.length) ? (
                  <div className="px-3 py-6 text-sm text-muted-foreground">
                    Pick a table to map its columns. Tick multiple tables to create
                    a pipeline for each.
                  </div>
                ) : (
                  <table className="w-full text-sm">
                    <thead className="border-b text-left text-xs text-muted-foreground">
                      <tr>
                        <th className="px-2 py-1">Sync</th>
                        <th className="px-2 py-1">Source column</th>
                        <th className="px-2 py-1">Rename to</th>
                        <th className="px-2 py-1">Role</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(tableCols[activeTable] ?? []).map((c, i) => (
                        <tr key={c.name} className="border-b last:border-0">
                          <td className="px-2 py-1">
                            <Switch
                              checked={c.include}
                              onCheckedChange={(v) => updateCol(activeTable, i, { include: v })}
                            />
                          </td>
                          <td className="px-2 py-1">
                            <span className="font-mono">{c.name}</span>
                            <span className="ml-1 text-xs text-muted-foreground">{c.dataType}</span>
                          </td>
                          <td className="px-2 py-1">
                            <Input
                              className="h-7"
                              value={c.alias}
                              disabled={!c.include}
                              onChange={(e) => updateCol(activeTable, i, { alias: e.target.value })}
                            />
                          </td>
                          <td className="px-2 py-1">
                            <select
                              className="h-7 rounded-md border border-border bg-transparent px-1 text-xs"
                              value={c.role}
                              disabled={!c.include}
                              onChange={(e) => setColRole(activeTable, i, e.target.value as ColRole)}
                            >
                              <option value="metadata">metadata</option>
                              <option value="text">text</option>
                              <option value="vector">vector</option>
                              <option value="id">id</option>
                              <option value="cursor">cursor</option>
                            </select>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </div>
            </div>
            {checked.size > 1 && (
              <p className="text-xs text-muted-foreground">
                {checked.size} tables selected — one pipeline will be created per
                table. Configure each table&apos;s mapping by clicking its name.
              </p>
            )}
          </>
        ) : (
          <>
            <div className="space-y-1">
              <label className="text-sm font-medium">Source query</label>
              <Textarea value={query} onChange={(e) => setQuery(e.target.value)} rows={3} />
              <Button variant="outline" size="sm" onClick={introspect} disabled={!connectionId}>
                Introspect columns
              </Button>
              {fields.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  Columns: {fields.map((f) => `${f.name} (${f.data_type})`).join(", ")}
                </p>
              )}
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1">
                <label className="text-sm font-medium">Text column</label>
                <Input value={textColumn} onChange={(e) => setTextColumn(e.target.value)} placeholder="body" />
              </div>
              <div className="space-y-1">
                <label className="text-sm font-medium">Metadata columns (comma-sep, blank = all)</label>
                <Input value={metadataColumns} onChange={(e) => setMetadataColumns(e.target.value)} placeholder="tag, author" />
              </div>
              <div className="space-y-1">
                <label className="text-sm font-medium">Vector column (optional)</label>
                <Input value={vectorColumn} onChange={(e) => setVectorColumn(e.target.value)} placeholder="embedding" />
              </div>
              <div className="space-y-1">
                <label className="text-sm font-medium">Id column (optional)</label>
                <Input value={idColumn} onChange={(e) => setIdColumn(e.target.value)} placeholder="id" />
              </div>
              <div className="space-y-1">
                <label className="text-sm font-medium">Cursor column (incremental)</label>
                <Input value={cursorColumn} onChange={(e) => setCursorColumn(e.target.value)} placeholder="updated_at" />
              </div>
            </div>
          </>
        )}

        {buildMode === "query" && (
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <label className="text-sm font-medium">Target table (in ToraDB)</label>
              <Input value={targetTable} onChange={(e) => setTargetTable(e.target.value)} placeholder="orders" />
            </div>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-4">
          <label className="flex items-center gap-2 text-sm">
            <Switch checked={incremental} onCheckedChange={setIncremental} /> Incremental
          </label>
          <label className="flex items-center gap-2 text-sm">
            <Switch checked={dropTable} onCheckedChange={setDropTable} /> Drop table on full sync
          </label>
          <div className="flex items-center gap-2 text-sm">
            <span>Schedule (secs)</span>
            <Input
              className="w-28"
              value={scheduleSecs}
              onChange={(e) => setScheduleSecs(e.target.value.replace(/[^0-9]/g, ""))}
              placeholder="manual"
            />
          </div>
        </div>
        <Button onClick={save} disabled={saving}>
          {saving
            ? "Creating…"
            : buildMode === "browse" && checked.size > 1
              ? `Create ${checked.size} pipelines`
              : "Create pipeline"}
        </Button>
      </CardContent>
    </Card>
  );
}
