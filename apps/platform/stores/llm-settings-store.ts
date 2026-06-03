import { create } from "zustand";
import { persist } from "zustand/middleware";

export type EmbedderSettings = {
  base_url: string;
  model: string;
  api_key: string;
  dim: number;
};

export type LlmSettings = {
  apiKey: string;
  baseUrl: string;
  model: string;
  maxTokens: number;
  temperature: number;
  useServerProxy: boolean;
  embedder: EmbedderSettings | null;
};

type LlmSettingsState = LlmSettings & {
  proxyAvailable: boolean;
  serverDefaultModel: string | null;
  setApiKey: (v: string) => void;
  setBaseUrl: (v: string) => void;
  setModel: (v: string) => void;
  setMaxTokens: (v: number) => void;
  setTemperature: (v: number) => void;
  setUseServerProxy: (v: boolean) => void;
  setEmbedder: (v: EmbedderSettings | null) => void;
  setProxyMeta: (available: boolean, defaultModel: string | null) => void;
  applyPreset: (preset: "openai" | "openrouter" | "ollama") => void;
};

const DEFAULT_EMBEDDER: EmbedderSettings = {
  base_url: "https://api.openai.com/v1",
  model: "text-embedding-3-small",
  api_key: "",
  dim: 1536,
};

export const useLlmSettingsStore = create<LlmSettingsState>()(
  persist(
    (set) => ({
      apiKey: "",
      baseUrl: "https://api.openai.com/v1",
      model: "gpt-4o-mini",
      maxTokens: 4096,
      temperature: 0.2,
      useServerProxy: false,
      embedder: null,
      proxyAvailable: false,
      serverDefaultModel: null,
      setApiKey: (apiKey) => set({ apiKey }),
      setBaseUrl: (baseUrl) => set({ baseUrl }),
      setModel: (model) => set({ model }),
      setMaxTokens: (maxTokens) => set({ maxTokens }),
      setTemperature: (temperature) => set({ temperature }),
      setUseServerProxy: (useServerProxy) => set({ useServerProxy }),
      setEmbedder: (embedder) => set({ embedder }),
      setProxyMeta: (proxyAvailable, serverDefaultModel) =>
        set({ proxyAvailable, serverDefaultModel }),
      applyPreset: (preset) => {
        if (preset === "openai") {
          set({
            baseUrl: "https://api.openai.com/v1",
            model: "gpt-4o-mini",
          });
        } else if (preset === "openrouter") {
          set({
            baseUrl: "https://openrouter.ai/api/v1",
            model: "openai/gpt-4o-mini",
          });
        } else {
          set({
            baseUrl: "http://localhost:11434/v1",
            model: "llama3.2",
          });
        }
      },
    }),
    {
      name: "toradb-llm",
      partialize: (s) => ({
        apiKey: s.apiKey,
        baseUrl: s.baseUrl,
        model: s.model,
        maxTokens: s.maxTokens,
        temperature: s.temperature,
        useServerProxy: s.useServerProxy,
        embedder: s.embedder,
      }),
    },
  ),
);

export function getDefaultEmbedder(): EmbedderSettings {
  return { ...DEFAULT_EMBEDDER };
}
