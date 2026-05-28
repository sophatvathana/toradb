"use client";

import { FormEvent, useCallback } from "react";
import { Copy, LoaderCircle, Play, Search, Sparkles } from "lucide-react";

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
import { SQL_TEMPLATES } from "@/lib/api";
import { usePlatformStore } from "@/stores/platform-store";

type SqlEditorProps = {
  onExportCsv?: () => void;
  onCopyJson?: () => void;
};

export function SqlEditor({ onExportCsv, onCopyJson }: SqlEditorProps) {
  const tables = usePlatformStore((s) => s.tables);
  const sql = usePlatformStore((s) => s.sql);
  const setSql = usePlatformStore((s) => s.setSql);
  const savedQueries = usePlatformStore((s) => s.savedQueries);
  const addSavedQuery = usePlatformStore((s) => s.addSavedQuery);
  const removeSavedQuery = usePlatformStore((s) => s.removeSavedQuery);
  const loadSavedQuery = usePlatformStore((s) => s.loadSavedQuery);
  const previewQuery = usePlatformStore((s) => s.previewQuery);
  const setPreviewQuery = usePlatformStore((s) => s.setPreviewQuery);
  const selectedTable = usePlatformStore((s) => s.selectedTable);
  const setSelectedTable = usePlatformStore((s) => s.setSelectedTable);
  const queryLoading = usePlatformStore((s) => s.queryLoading);
  const rows = usePlatformStore((s) => s.rows);
  const runSqlQuery = usePlatformStore((s) => s.runSqlQuery);
  const runPreview = usePlatformStore((s) => s.runPreview);

  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void runSqlQuery(false);
      }
    },
    [runSqlQuery],
  );

  function onRunSql(ev: FormEvent) {
    ev.preventDefault();
    void runSqlQuery(false);
  }

  function onPreview(ev: FormEvent) {
    ev.preventDefault();
    void runPreview();
  }

  function copySql() {
    void navigator.clipboard.writeText(sql);
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Query Workbench</CardTitle>
        <CardDescription>
          Run retrieval SQL · Cmd/Ctrl+Enter to execute
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <label className="text-xs text-muted-foreground">Template</label>
          <select
            className="h-8 rounded-md border border-input bg-background px-2 text-xs"
            defaultValue=""
            onChange={(e) => {
              const t = SQL_TEMPLATES.find((x) => x.label === e.target.value);
              if (t) setSql(t.sql);
            }}
          >
            <option value="">—</option>
            {SQL_TEMPLATES.map((t) => (
              <option key={t.label} value={t.label}>
                {t.label}
              </option>
            ))}
          </select>
          <Button type="button" variant="ghost" size="sm" onClick={copySql}>
            <Copy className="size-3" />
            Copy SQL
          </Button>
          {onExportCsv && (
            <Button type="button" variant="ghost" size="sm" onClick={onExportCsv}>
              Export CSV
            </Button>
          )}
          {onCopyJson && (
            <Button type="button" variant="ghost" size="sm" onClick={onCopyJson}>
              Copy JSON
            </Button>
          )}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              const name = prompt("Saved query name");
              if (name) addSavedQuery(name, sql);
            }}
          >
            Save query
          </Button>
        </div>

        {savedQueries.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {savedQueries.map((q) => (
              <span key={q.id} className="inline-flex items-center gap-1 rounded border border-border px-2 py-0.5 text-xs">
                <button type="button" className="hover:text-primary" onClick={() => loadSavedQuery(q.id)}>
                  {q.name}
                </button>
                <button
                  type="button"
                  className="text-muted-foreground hover:text-destructive"
                  onClick={() => removeSavedQuery(q.id)}
                  aria-label={`Remove ${q.name}`}
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        )}

        <form className="flex flex-wrap gap-2" onSubmit={onPreview}>
          <select
            className="h-9 rounded-md border border-input bg-background px-3 text-sm"
            value={selectedTable}
            onChange={(e) => setSelectedTable(e.target.value)}
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
          <Input
            className="max-w-md flex-1"
            value={previewQuery}
            onChange={(e) => setPreviewQuery(e.target.value)}
            placeholder="Quick preview query"
          />
          <Button type="submit" variant="secondary" disabled={queryLoading || !selectedTable}>
            <Search className="size-4" />
            Preview
          </Button>
        </form>

        <form onSubmit={onRunSql} className="space-y-3">
          <Textarea
            value={sql}
            onChange={(e) => setSql(e.target.value)}
            onKeyDown={onKeyDown}
            className="min-h-40 font-mono text-xs"
          />
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="text-xs text-muted-foreground">
              POST /api/sql · result rows {rows.length}
            </div>
            <div className="flex gap-2">
              <Button
                type="button"
                variant="outline"
                disabled={queryLoading}
                onClick={() => void runSqlQuery(true)}
              >
                <Sparkles className="size-4" />
                Explain
              </Button>
              <Button type="submit" disabled={queryLoading}>
                {queryLoading ? (
                  <LoaderCircle className="size-4 animate-spin" />
                ) : (
                  <Play className="size-4" />
                )}
                Run
              </Button>
            </div>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
