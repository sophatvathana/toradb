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
import { Switch } from "@/components/ui/switch";
import {
  authLogout,
  createApiKey,
  fetchAuthStatus,
  type AuthStatus,
} from "@/lib/api";
import { chatCompletion, fetchChatConfig } from "@/lib/chat/llm-client";
import {
  getDefaultEmbedder,
  useLlmSettingsStore,
} from "@/stores/llm-settings-store";

export default function SettingsPage() {
  const router = useRouter();
  const { toast } = useToast();
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [keyName, setKeyName] = useState("");
  const [newKey, setNewKey] = useState<string | null>(null);
  const [testingLlm, setTestingLlm] = useState(false);

  const llm = useLlmSettingsStore();
  const setProxyMeta = useLlmSettingsStore((s) => s.setProxyMeta);

  const refresh = useCallback(async () => {
    try {
      setStatus(await fetchAuthStatus());
    } catch (err) {
      toast({ title: "Failed to load settings", description: String(err), variant: "error" });
    }
  }, [toast]);

  useEffect(() => {
    void refresh();
    void fetchChatConfig().then((c) => {
      setProxyMeta(c.proxy_available, c.default_model ?? null);
      if (c.default_model && llm.model === "gpt-4o-mini") {
        useLlmSettingsStore.getState().setModel(c.default_model);
      }
    });
  }, [refresh, setProxyMeta, llm.model]);

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

  const testLlm = async () => {
    setTestingLlm(true);
    try {
      await chatCompletion(llm, {
        messages: [{ role: "user", content: "Reply with OK only." }],
        max_tokens: 16,
      });
      toast({ title: "LLM connection OK" });
    } catch (err) {
      toast({
        title: "LLM test failed",
        description: err instanceof Error ? err.message : String(err),
        variant: "error",
      });
    } finally {
      setTestingLlm(false);
    }
  };

  const embedder = llm.embedder ?? getDefaultEmbedder();

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Authentication, API keys, LLM for Chat, and platform configuration.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>LLM (Chat)</CardTitle>
          <CardDescription>
            Browser mode sends requests directly to your provider (key stays in
            localStorage). Server proxy uses{" "}
            <code className="text-xs">TORADB_LLM_*</code> env vars on the
            ingest server — recommended for production.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span>Server proxy:</span>
            {llm.proxyAvailable ? (
              <Badge variant="success">available</Badge>
            ) : (
              <Badge variant="outline">not configured</Badge>
            )}
          </div>

          <div className="flex items-center justify-between gap-4">
            <div>
              <p className="text-sm font-medium">Use server proxy</p>
              <p className="text-xs text-muted-foreground">
                Route Chat through ToraDB API (no browser API key required)
              </p>
            </div>
            <Switch
              checked={llm.useServerProxy}
              disabled={!llm.proxyAvailable}
              onCheckedChange={llm.setUseServerProxy}
            />
          </div>

          {!llm.useServerProxy && (
            <>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => llm.applyPreset("openai")}
                >
                  OpenAI preset
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => llm.applyPreset("openrouter")}
                >
                  OpenRouter
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => llm.applyPreset("ollama")}
                >
                  Ollama local
                </Button>
              </div>
              <label className="block space-y-1 text-xs">
                <span className="text-muted-foreground">API key</span>
                <Input
                  type="password"
                  value={llm.apiKey}
                  onChange={(e) => llm.setApiKey(e.target.value)}
                  placeholder="sk-…"
                  autoComplete="off"
                />
              </label>
              <label className="block space-y-1 text-xs">
                <span className="text-muted-foreground">Base URL</span>
                <Input
                  value={llm.baseUrl}
                  onChange={(e) => llm.setBaseUrl(e.target.value)}
                  placeholder="https://api.openai.com/v1"
                />
              </label>
            </>
          )}

          <label className="block space-y-1 text-xs">
            <span className="text-muted-foreground">Model</span>
            <Input
              value={llm.model}
              onChange={(e) => llm.setModel(e.target.value)}
            />
          </label>

          <div className="grid gap-3 sm:grid-cols-2">
            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">Max tokens</span>
              <Input
                type="number"
                min={256}
                max={128000}
                value={llm.maxTokens}
                onChange={(e) => llm.setMaxTokens(Number(e.target.value) || 4096)}
              />
            </label>
            <label className="space-y-1 text-xs">
              <span className="text-muted-foreground">Temperature</span>
              <Input
                type="number"
                min={0}
                max={2}
                step={0.1}
                value={llm.temperature}
                onChange={(e) => llm.setTemperature(Number(e.target.value) || 0.2)}
              />
            </label>
          </div>

          <Button type="button" onClick={() => void testLlm()} disabled={testingLlm}>
            {testingLlm ? "Testing…" : "Test LLM connection"}
          </Button>

          <p className="text-xs text-muted-foreground">
            Ollama and some local servers require CORS headers on the LLM HTTP
            server when using browser mode.
          </p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Embedder (vector search in Chat)</CardTitle>
          <CardDescription>
            Optional OpenAI-compatible embedding endpoint for dense/hybrid search
            tools.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex items-center justify-between gap-4">
            <p className="text-sm">Enable embedder</p>
            <Switch
              checked={llm.embedder !== null}
              onCheckedChange={(on) =>
                llm.setEmbedder(on ? { ...getDefaultEmbedder() } : null)
              }
            />
          </div>
          {llm.embedder && (
            <>
              <label className="block space-y-1 text-xs">
                <span className="text-muted-foreground">Embedder base URL</span>
                <Input
                  value={embedder.base_url}
                  onChange={(e) =>
                    llm.setEmbedder({ ...embedder, base_url: e.target.value })
                  }
                />
              </label>
              <label className="block space-y-1 text-xs">
                <span className="text-muted-foreground">Embedding model</span>
                <Input
                  value={embedder.model}
                  onChange={(e) =>
                    llm.setEmbedder({ ...embedder, model: e.target.value })
                  }
                />
              </label>
              <label className="block space-y-1 text-xs">
                <span className="text-muted-foreground">API key (optional)</span>
                <Input
                  type="password"
                  value={embedder.api_key}
                  onChange={(e) =>
                    llm.setEmbedder({ ...embedder, api_key: e.target.value })
                  }
                />
              </label>
            </>
          )}
        </CardContent>
      </Card>

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
