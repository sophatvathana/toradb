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
import {
  createConnection,
  deleteConnection,
  fetchConnections,
  testConnectionUrl,
  uploadConnectionFile,
  type Connection,
} from "@/lib/api";

export default function ConnectionsPage() {
  const { toast } = useToast();
  const [connections, setConnections] = useState<Connection[]>([]);
  const [loading, setLoading] = useState(true);
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [mode, setMode] = useState<"url" | "upload">("url");
  const [file, setFile] = useState<File | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [dropTarget, setDropTarget] = useState<Connection | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setConnections(await fetchConnections());
    } catch (err) {
      toast({ title: "Failed to load connections", description: String(err), variant: "error" });
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const runTest = async () => {
    if (!url.trim()) return;
    setTesting(true);
    try {
      const res = await testConnectionUrl(url.trim());
      if (res.ok) {
        toast({ title: "Connection OK" });
      } else {
        toast({ title: "Connection failed", description: res.error ?? "unknown", variant: "error" });
      }
    } catch (err) {
      toast({ title: "Connection failed", description: String(err), variant: "error" });
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    if (mode === "upload") {
      if (!file) {
        toast({ title: "Choose a SQLite file to upload", variant: "error" });
        return;
      }
    } else if (!name.trim() || !url.trim()) {
      toast({ title: "Name and URL are required", variant: "error" });
      return;
    }
    setSaving(true);
    try {
      if (mode === "upload" && file) {
        await uploadConnectionFile(name.trim() || file.name, file);
      } else {
        await createConnection({ name: name.trim(), url: url.trim() });
      }
      toast({ title: "Connection created" });
      setCreateOpen(false);
      setName("");
      setUrl("");
      setFile(null);
      await refresh();
    } catch (err) {
      toast({ title: "Failed to create connection", description: String(err), variant: "error" });
    } finally {
      setSaving(false);
    }
  };

  const confirmDelete = async () => {
    if (!dropTarget) return;
    try {
      await deleteConnection(dropTarget.id);
      toast({ title: "Connection deleted", });
      await refresh();
    } catch (err) {
      toast({ title: "Failed to delete", description: String(err), variant: "error" });
    } finally {
      setDropTarget(null);
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Connections</h1>
          <p className="text-sm text-muted-foreground">
            External data sources for toraPipe sync (Postgres, MySQL, SQLite).
          </p>
        </div>
        <Button onClick={() => setCreateOpen((v) => !v)}>
          {createOpen ? "Cancel" : "New connection"}
        </Button>
      </div>

      {createOpen && (
        <Card>
          <CardHeader>
            <CardTitle>New connection</CardTitle>
            <CardDescription>
              Provide a connection URL such as{" "}
              <code>postgres://user:pw@host:5432/db</code> or{" "}
              <code>sqlite:///path/to.db</code>.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2">
              <Button
                variant={mode === "url" ? "default" : "outline"}
                size="sm"
                onClick={() => setMode("url")}
              >
                Connection URL
              </Button>
              <Button
                variant={mode === "upload" ? "default" : "outline"}
                size="sm"
                onClick={() => setMode("upload")}
              >
                Upload SQLite file
              </Button>
            </div>
            <div className="space-y-1">
              <label className="text-sm font-medium">Name</label>
              <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={mode === "upload" ? "(defaults to file name)" : "prod-postgres"}
              />
            </div>
            {mode === "url" ? (
              <>
                <div className="space-y-1">
                  <label className="text-sm font-medium">Connection URL</label>
                  <Input
                    value={url}
                    onChange={(e) => setUrl(e.target.value)}
                    placeholder="postgres://user:pw@host:5432/db"
                  />
                </div>
                <div className="flex gap-2">
                  <Button variant="outline" onClick={runTest} disabled={testing || !url.trim()}>
                    {testing ? "Testing…" : "Test"}
                  </Button>
                  <Button onClick={save} disabled={saving}>
                    {saving ? "Saving…" : "Create"}
                  </Button>
                </div>
              </>
            ) : (
              <>
                <div className="space-y-1">
                  <label className="text-sm font-medium">SQLite file</label>
                  <input
                    type="file"
                    accept=".db,.sqlite,.sqlite3"
                    onChange={(e) => setFile(e.target.files?.[0] ?? null)}
                    className="block w-full text-sm text-muted-foreground file:mr-3 file:rounded-md file:border file:border-border file:bg-transparent file:px-3 file:py-1.5 file:text-sm"
                  />
                  <p className="text-xs text-muted-foreground">
                    The file is uploaded and stored server-side; the connection
                    opens it read-only.
                  </p>
                </div>
                <Button onClick={save} disabled={saving || !file}>
                  {saving ? "Uploading…" : "Upload & create"}
                </Button>
              </>
            )}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardContent className="p-0">
          <table className="w-full text-sm">
            <thead className="border-b text-left text-muted-foreground">
              <tr>
                <th className="px-4 py-2">Name</th>
                <th className="px-4 py-2">Kind</th>
                <th className="px-4 py-2">URL</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {loading ? (
                <tr>
                  <td className="px-4 py-6 text-muted-foreground" colSpan={4}>
                    Loading…
                  </td>
                </tr>
              ) : connections.length === 0 ? (
                <tr>
                  <td className="px-4 py-6 text-muted-foreground" colSpan={4}>
                    No connections yet.
                  </td>
                </tr>
              ) : (
                connections.map((c) => (
                  <tr key={c.id} className="border-b last:border-0">
                    <td className="px-4 py-2 font-medium">{c.name}</td>
                    <td className="px-4 py-2">
                      <Badge variant="secondary">{c.kind}</Badge>
                    </td>
                    <td className="px-4 py-2 font-mono text-xs text-muted-foreground">
                      {c.url_masked}
                    </td>
                    <td className="px-4 py-2 text-right">
                      <Button variant="ghost" size="sm" onClick={() => setDropTarget(c)}>
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

      <ConfirmDialog
        open={dropTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDropTarget(null);
        }}
        title="Delete connection?"
        description={`This removes "${dropTarget?.name ?? ""}". Pipelines using it will fail until reconfigured.`}
        confirmLabel="Delete"
        destructive
        onConfirm={confirmDelete}
      />
    </div>
  );
}
