"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { authBootstrap, authLogin, fetchAuthStatus } from "@/lib/api";

export default function LoginPage() {
  const router = useRouter();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [needsBootstrap, setNeedsBootstrap] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void fetchAuthStatus().then((s) => {
      if (!s.auth_enabled) router.replace("/overview");
    });
  }, [router]);

  const submit = async (bootstrap: boolean) => {
    if (!name.trim() || !password) {
      setError("Username and password required");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (bootstrap) {
        await authBootstrap(name.trim(), password);
        await authLogin(name.trim(), password);
      } else {
        await authLogin(name.trim(), password);
      }
      router.replace("/overview");
    } catch (err) {
      const msg = String(err);
      if (!bootstrap && /invalid credentials/i.test(msg)) {
        setNeedsBootstrap(true);
      }
      setError(msg);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-6">
      <Card className="w-full max-w-sm">
        <CardHeader>
          <CardTitle>ToraDB Platform</CardTitle>
          <CardDescription>
            {needsBootstrap ? "Create the first admin account." : "Sign in to continue."}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-1">
            <label className="text-sm font-medium">Username</label>
            <Input value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          </div>
          <div className="space-y-1">
            <label className="text-sm font-medium">Password</label>
            <Input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void submit(needsBootstrap);
              }}
            />
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <Button className="w-full" disabled={busy} onClick={() => submit(needsBootstrap)}>
            {busy ? "…" : needsBootstrap ? "Create admin & sign in" : "Sign in"}
          </Button>
          {!needsBootstrap && (
            <button
              type="button"
              className="text-xs text-muted-foreground hover:underline"
              onClick={() => setNeedsBootstrap(true)}
            >
              First run? Create an admin account
            </button>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
