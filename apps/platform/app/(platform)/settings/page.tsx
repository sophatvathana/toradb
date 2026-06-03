"use client";

import { useCallback, useEffect, useState } from "react";
import { useRouter } from "next/navigation";

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
  authLogout,
  createApiKey,
  fetchAuthStatus,
  type AuthStatus,
} from "@/lib/api";

export default function SettingsPage() {
  const router = useRouter();
  const { toast } = useToast();
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [keyName, setKeyName] = useState("");
  const [newKey, setNewKey] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await fetchAuthStatus());
    } catch (err) {
      toast({ title: "Failed to load settings", description: String(err), variant: "error" });
    }
  }, [toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onCreateKey = async () => {
    if (!keyName.trim()) {
      toast({ title: "Key name required", variant: "error" });
      return;
    }
    try {
      const { key } = await createApiKey(keyName.trim());
      setNewKey(key);
      setKeyName("");
      toast({ title: "API key created — copy it now" });
    } catch (err) {
      toast({ title: "Failed to create key", description: String(err), variant: "error" });
    }
  };

  const onLogout = async () => {
    try {
      await authLogout();
    } catch {
      /* ignore */
    }
    router.replace("/login");
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Authentication, API keys, and platform configuration.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Authentication</CardTitle>
          <CardDescription>Session and access control for this server.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex items-center gap-2 text-sm">
            <span>Status:</span>
            {status?.auth_enabled ? (
              <Badge variant="success">enabled</Badge>
            ) : (
              <Badge variant="warning">open (no auth)</Badge>
            )}
          </div>
          {status?.auth_enabled && (
            <Button variant="outline" size="sm" onClick={onLogout}>
              Log out
            </Button>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>API keys</CardTitle>
          <CardDescription>
            Bearer tokens for programmatic access (CLI, scripts). Shown once at
            creation.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex gap-2">
            <Input
              value={keyName}
              onChange={(e) => setKeyName(e.target.value)}
              placeholder="ci-pipeline"
            />
            <Button onClick={onCreateKey}>Create key</Button>
          </div>
          {newKey && (
            <div className="rounded-md border border-border bg-muted/40 p-3">
              <p className="text-xs text-muted-foreground">
                Copy this key now — it will not be shown again:
              </p>
              <code className="break-all text-sm">{newKey}</code>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
