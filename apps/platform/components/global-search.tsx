"use client";

import * as Dialog from "@radix-ui/react-dialog";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Search } from "lucide-react";

import { Input } from "@/components/ui/input";
import { matchesSearchQuery } from "@/lib/search";
import { usePlatformStore } from "@/stores/platform-store";

const NAV_TARGETS = [
  { href: "/overview", label: "Workspace Overview" },
  { href: "/search", label: "Search" },
  { href: "/query", label: "Query Workbench" },
  { href: "/query-log", label: "Query Log" },
  { href: "/catalog", label: "Catalog" },
  { href: "/schema", label: "Schema" },
  { href: "/views", label: "Materialized Views" },
  { href: "/ingest", label: "Ingest" },
  { href: "/jobs", label: "Background Jobs" },
] as const;

type SearchItem = {
  id: string;
  label: string;
  hint?: string;
  href: string;
  onSelect?: () => void;
};

export function GlobalSearch() {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);

  const tables = usePlatformStore((s) => s.tables);
  const history = usePlatformStore((s) => s.history);
  const savedQueries = usePlatformStore((s) => s.savedQueries);
  const openQueryFromHistory = usePlatformStore((s) => s.openQueryFromHistory);
  const setSelectedTable = usePlatformStore((s) => s.setSelectedTable);

  const items = useMemo<SearchItem[]>(() => {
    const q = query.trim();
    const out: SearchItem[] = [];

    for (const nav of NAV_TARGETS) {
      if (!matchesSearchQuery(q, [nav.label, nav.href])) continue;
      out.push({
        id: `nav-${nav.href}`,
        label: nav.label,
        hint: "Page",
        href: nav.href,
      });
    }

    for (const t of tables) {
      if (!matchesSearchQuery(q, [t.name, t.state, String(t.rows)])) continue;
      out.push({
        id: `table-${t.name}`,
        label: t.name,
        hint: `${t.rows} rows · ${t.state}`,
        href: `/catalog/${encodeURIComponent(t.name)}`,
      });
      out.push({
        id: `search-${t.name}`,
        label: `Search in ${t.name}`,
        hint: "BM25",
        href: `/search?table=${encodeURIComponent(t.name)}`,
        onSelect: () => setSelectedTable(t.name),
      });
    }

    for (const h of history.slice(0, 30)) {
      if (!matchesSearchQuery(q, [h.query, h.kind, h.status])) continue;
      out.push({
        id: `hist-${h.executed_at_unix_secs}-${h.query.slice(0, 24)}`,
        label: h.query.length > 72 ? `${h.query.slice(0, 72)}…` : h.query,
        hint: `${h.kind} · ${h.status}`,
        href: "/query",
        onSelect: () => openQueryFromHistory(h.query),
      });
    }

    for (const s of savedQueries) {
      if (!matchesSearchQuery(q, [s.name, s.sql])) continue;
      out.push({
        id: `saved-${s.id}`,
        label: s.name,
        hint: "Saved query",
        href: "/query",
        onSelect: () => openQueryFromHistory(s.sql),
      });
    }

    return out.slice(0, 24);
  }, [query, tables, history, savedQueries, openQueryFromHistory, setSelectedTable]);

  const selectItem = useCallback(
    (item: SearchItem) => {
      item.onSelect?.();
      setOpen(false);
      setQuery("");
      router.push(item.href);
    },
    [router],
  );

  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((v) => !v);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((i) => Math.min(i + 1, Math.max(0, items.length - 1)));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter" && items[activeIndex]) {
        e.preventDefault();
        selectItem(items[activeIndex]);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, items, activeIndex, selectItem]);

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="flex h-9 min-w-[200px] items-center gap-2 rounded-md border border-input bg-background px-3 text-sm text-muted-foreground transition-colors hover:border-primary/30 hover:text-foreground"
        aria-label="Open search"
      >
        <Search className="size-4 shrink-0" />
        <span className="flex-1 text-left">Search…</span>
        <kbd className="hidden rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-[10px] sm:inline">
          ⌘K
        </kbd>
      </button>

      <Dialog.Root open={open} onOpenChange={setOpen}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-[60] bg-black/60" />
          <Dialog.Content className="fixed left-1/2 top-[12%] z-[60] w-full max-w-lg -translate-x-1/2 rounded-lg border border-border bg-card p-0 shadow-xl">
            <Dialog.Title className="sr-only">Global search</Dialog.Title>
            <div className="border-b border-border p-3">
              <Input
                autoFocus
                placeholder="Tables, pages, queries…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                className="border-0 bg-transparent shadow-none focus-visible:ring-0"
              />
            </div>
            <ul className="max-h-80 overflow-y-auto p-1" role="listbox">
              {items.length === 0 ? (
                <li className="px-3 py-6 text-center text-sm text-muted-foreground">
                  No matches
                </li>
              ) : (
                items.map((item, i) => (
                  <li key={item.id}>
                    <Link
                      href={item.href}
                      className={`flex w-full items-center justify-between rounded-md px-3 py-2 text-sm transition-colors ${
                        i === activeIndex
                          ? "bg-primary/15 text-primary"
                          : "hover:bg-muted"
                      }`}
                      onClick={(e) => {
                        e.preventDefault();
                        selectItem(item);
                      }}
                    >
                      <span className="truncate font-medium">{item.label}</span>
                      {item.hint && (
                        <span className="ml-2 shrink-0 text-xs text-muted-foreground">
                          {item.hint}
                        </span>
                      )}
                    </Link>
                  </li>
                ))
              )}
            </ul>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </>
  );
}
