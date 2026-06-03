"use client";

import { Download, Square } from "lucide-react";

import { Button } from "@/components/ui/button";
import { SEARCH_STRATEGIES } from "@/lib/api";
import { downloadText, threadToHtml, threadToMarkdown } from "@/lib/chat/report";
import type { ChatThread } from "@/lib/chat/types";
import { useChatStore } from "@/stores/chat-store";
import { usePlatformStore } from "@/stores/platform-store";

export function ChatHeader({
  thread,
  onStop,
  running,
}: {
  thread: ChatThread;
  onStop: () => void;
  running: boolean;
}) {
  const tables = usePlatformStore((s) => s.tables);
  const updateThreadMeta = useChatStore((s) => s.updateThreadMeta);

  const slug = thread.title.replace(/[^a-z0-9]+/gi, "-").slice(0, 32) || "report";
  const date = new Date().toISOString().slice(0, 10);

  return (
    <header className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-4 py-2">
      <label className="flex items-center gap-1 text-xs text-muted-foreground">
        Table
        <select
          className="h-8 rounded-md border border-input bg-background px-2 font-mono text-xs"
          value={thread.table}
          onChange={(e) =>
            updateThreadMeta(thread.id, { table: e.target.value })
          }
          aria-label="Active table"
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
      </label>

      <label className="flex items-center gap-1 text-xs text-muted-foreground">
        Strategy
        <select
          className="h-8 max-w-36 rounded-md border border-input bg-background px-2 text-xs"
          value={thread.strategy}
          onChange={(e) =>
            updateThreadMeta(thread.id, { strategy: e.target.value })
          }
          aria-label="Search strategy"
        >
          {SEARCH_STRATEGIES.map((s) => (
            <option key={s.value} value={s.value}>
              {s.label}
            </option>
          ))}
        </select>
      </label>

      <label className="flex items-center gap-1 text-xs text-muted-foreground">
        Top K
        <input
          type="number"
          min={1}
          max={100}
          className="h-8 w-14 rounded-md border border-input bg-background px-2 text-xs"
          value={thread.topK}
          onChange={(e) =>
            updateThreadMeta(thread.id, {
              topK: Number(e.target.value) || 10,
            })
          }
          aria-label="Top K"
        />
      </label>

      <label className="flex items-center gap-1 text-xs">
        <input
          type="checkbox"
          checked={thread.explain}
          onChange={(e) =>
            updateThreadMeta(thread.id, { explain: e.target.checked })
          }
        />
        Explain
      </label>

      <div className="ml-auto flex flex-wrap gap-1">
        {running && (
          <Button type="button" size="sm" variant="outline" onClick={onStop}>
            <Square className="size-3.5" />
            Stop
          </Button>
        )}
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() =>
            downloadText(
              `toradb-report-${slug}-${date}.md`,
              threadToMarkdown(thread),
              "text/markdown",
            )
          }
        >
          <Download className="size-3.5" />
          MD
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() =>
            downloadText(
              `toradb-report-${slug}-${date}.html`,
              threadToHtml(thread),
              "text/html",
            )
          }
        >
          <Download className="size-3.5" />
          HTML
        </Button>
      </div>
    </header>
  );
}
