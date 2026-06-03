"use client";

import { MessageSquarePlus, Pencil, Trash2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useChatStore } from "@/stores/chat-store";

function formatRelative(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return new Date(ts).toLocaleDateString();
}

export function ThreadSidebar({ defaultTable }: { defaultTable: string }) {
  const threads = useChatStore((s) => s.threads);
  const activeThreadId = useChatStore((s) => s.activeThreadId);
  const newThread = useChatStore((s) => s.newThread);
  const deleteThread = useChatStore((s) => s.deleteThread);
  const renameThread = useChatStore((s) => s.renameThread);
  const setActiveThread = useChatStore((s) => s.setActiveThread);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");

  return (
    <aside
      className="flex w-56 shrink-0 flex-col border-r border-border bg-card md:w-60"
      aria-label="Chat threads"
    >
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-xs font-medium text-muted-foreground">Threads</span>
        <Button
          type="button"
          size="icon"
          variant="ghost"
          className="size-8"
          title="New chat"
          onClick={() => newThread(defaultTable)}
        >
          <MessageSquarePlus className="size-4" />
        </Button>
      </div>
      <ul className="flex-1 overflow-y-auto p-2">
        {threads.length === 0 && (
          <li className="px-2 py-4 text-center text-xs text-muted-foreground">
            No threads yet
          </li>
        )}
        {threads.map((t) => (
          <li key={t.id} className="mb-1">
            {renamingId === t.id ? (
              <Input
                className="h-8 text-xs"
                value={renameValue}
                autoFocus
                onChange={(e) => setRenameValue(e.target.value)}
                onBlur={() => {
                  if (renameValue.trim()) renameThread(t.id, renameValue.trim());
                  setRenamingId(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    if (renameValue.trim()) renameThread(t.id, renameValue.trim());
                    setRenamingId(null);
                  }
                  if (e.key === "Escape") setRenamingId(null);
                }}
              />
            ) : (
              <button
                type="button"
                className={`group flex w-full items-start gap-1 rounded-md px-2 py-2 text-left text-sm transition-colors ${
                  activeThreadId === t.id
                    ? "bg-primary/10 text-foreground"
                    : "hover:bg-muted"
                }`}
                onClick={() => setActiveThread(t.id)}
              >
                <span className="min-w-0 flex-1 truncate font-medium">{t.title}</span>
                <span className="flex shrink-0 gap-0.5 opacity-0 group-hover:opacity-100">
                  <span
                    role="button"
                    tabIndex={0}
                    className="rounded p-0.5 hover:bg-muted"
                    title="Rename"
                    onClick={(e) => {
                      e.stopPropagation();
                      setRenamingId(t.id);
                      setRenameValue(t.title);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.stopPropagation();
                        setRenamingId(t.id);
                        setRenameValue(t.title);
                      }
                    }}
                  >
                    <Pencil className="size-3" />
                  </span>
                  <span
                    role="button"
                    tabIndex={0}
                    className="rounded p-0.5 hover:bg-destructive/20"
                    title="Delete"
                    onClick={(e) => {
                      e.stopPropagation();
                      deleteThread(t.id);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.stopPropagation();
                        deleteThread(t.id);
                      }
                    }}
                  >
                    <Trash2 className="size-3" />
                  </span>
                </span>
              </button>
            )}
            <p className="px-2 text-[10px] text-muted-foreground">
              {formatRelative(t.updatedAt)}
            </p>
          </li>
        ))}
      </ul>
    </aside>
  );
}
