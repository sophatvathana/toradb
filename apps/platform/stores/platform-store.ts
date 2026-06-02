import { create } from "zustand";
import { persist } from "zustand/middleware";

import {
  beginIngest,
  cacheHitRatio,
  cancelIngestJob,
  compactTable,
  createMaterializedView,
  dropMaterializedView,
  dropTable,
  fetchHealth,
  fetchIngestJob,
  fetchIngestJobs,
  fetchJobs,
  fetchMaterializedViews,
  fetchMetrics,
  fetchQueryHistory,
  fetchTableDetail,
  fetchTableIndexes,
  fetchTableSample,
  fetchTables,
  fetchTasks,
  finishIngest,
  finishTable,
  ingestFromHf,
  refreshMaterializedView,
  resumeTable,
  runQueryPreview,
  runSql,
  uploadIngestFile,
  type HealthResponse,
  type IngestJob,
  type JobInfo,
  type MaterializedViewInfo,
  type MetricsResponse,
  type OpTask,
  type QueryHistoryEntry,
  type QueryMetricsResponse,
  type SavedQuery,
  type SavedSearch,
  type SqlResponse,
  type TableDetailResponse,
  type TableInfo,
  type TableSearchRequest,
} from "@/lib/api";

const DEFAULT_SQL =
  "SELECT id, score FROM passages SPARSE SEARCH text BM25('database engine') LIMIT 10";

/** Dedupes concurrent `watchIngestJob` calls (page + hydrate must not fight). */
let ingestWatchJobId: number | null = null;
let ingestWatchPromise: Promise<IngestJob> | null = null;

function mergeIngestJob(prev: IngestJob | null, job: IngestJob, jobId: number): IngestJob {
  if (prev?.id !== jobId) return job;
  if (job.progress == null) return job;
  if (prev.progress == null) return job;
  return { ...job, progress: Math.max(prev.progress, job.progress) };
}

type PlatformState = {
  health: HealthResponse | null;
  tables: TableInfo[];
  metrics: MetricsResponse | null;
  jobs: JobInfo[];
  history: QueryHistoryEntry[];
  materializedViews: MaterializedViewInfo[];
  tasks: OpTask[];
  tableDetail: TableDetailResponse | null;
  tableIndexes: SqlResponse | null;
  sampleRows: Record<string, unknown>[];
  sampleColumns: string[];
  ingestJob: IngestJob | null;
  savedQueries: SavedQuery[];
  savedSearches: SavedSearch[];
  previousTasks: OpTask[];

  loading: boolean;
  error: string;

  sql: string;
  previewQuery: string;
  selectedTable: string;
  columns: string[];
  rows: Record<string, unknown>[];
  queryLoading: boolean;
  queryError: string;
  lastExplainText: string | null;
  lastMetrics: QueryMetricsResponse | null;

  ingestTable: string;
  ingestSource: "file" | "hf";
  ingestFormat: "parquet" | "jsonl";
  ingestDropTable: boolean;
  ingestUploading: boolean;
  ingestRowsIngested: number;
  ingestBulkActive: boolean;
  ingestLimit: number;
  hfDataset: string;
  hfConfig: string;
  hfSplit: string;
  hfTextColumn: string;

  pollTimer: number | null;
  ingestPollTimer: number | null;

  hydrate: () => Promise<void>;
  syncRunningIngestJob: () => Promise<void>;
  refreshAll: () => Promise<void>;
  refreshMetrics: () => Promise<void>;
  refreshJobs: () => Promise<void>;
  refreshTables: () => Promise<void>;
  refreshHistory: () => Promise<void>;
  refreshTasks: () => Promise<void>;
  refreshMaterializedViews: () => Promise<void>;
  startPolling: (ms?: number) => void;
  stopPolling: () => void;

  setSql: (sql: string) => void;
  setPreviewQuery: (query: string) => void;
  setSelectedTable: (name: string) => void;
  openQueryFromHistory: (query: string) => void;

  fetchTableDetailAction: (name: string) => Promise<void>;
  fetchTableSampleAction: (name: string) => Promise<void>;

  runSqlQuery: (explain?: boolean) => Promise<void>;
  runPreview: () => Promise<void>;

  finishTableAction: (name: string, compact?: boolean) => Promise<void>;
  resumeTableAction: (name: string, compact?: boolean) => Promise<void>;
  dropTableAction: (name: string) => Promise<void>;

  setIngestTable: (table: string) => void;
  setIngestSource: (source: "file" | "hf") => void;
  setIngestFormat: (format: "parquet" | "jsonl") => void;
  setIngestDropTable: (v: boolean) => void;
  setIngestLimit: (n: number) => void;
  setHfDataset: (v: string) => void;
  setHfConfig: (v: string) => void;
  setHfSplit: (v: string) => void;
  setHfTextColumn: (v: string) => void;
  beginIngestAction: () => Promise<void>;
  uploadIngestFileAction: (file: File) => Promise<number>;
  ingestFromHfAction: () => Promise<number>;
  watchIngestJob: (jobId: number) => Promise<IngestJob>;
  stopIngestJobWatch: () => void;
  pollIngestJob: (jobId: number) => Promise<IngestJob>;
  cancelIngestJobAction: (jobId: number) => Promise<void>;
  finishIngestAction: (compact?: boolean) => Promise<void>;
  compactTableAction: (name: string, full?: boolean) => Promise<void>;
  fetchTableIndexesAction: (name: string) => Promise<void>;
  createMvAction: (name: string, query: string) => Promise<void>;
  refreshMvAction: (name: string) => Promise<void>;
  dropMvAction: (name: string) => Promise<void>;
  addSavedQuery: (name: string, sql: string) => void;
  removeSavedQuery: (id: string) => void;
  loadSavedQuery: (id: string) => void;
  addSavedSearch: (name: string, request: TableSearchRequest) => void;
  removeSavedSearch: (id: string) => void;
};

function applySqlResult(
  data: SqlResponse,
  set: (partial: Partial<PlatformState>) => void,
) {
  set({
    columns: data.columns ?? [],
    rows: (data.rows ?? []) as Record<string, unknown>[],
    lastExplainText: data.explain_text ?? null,
    lastMetrics: data.metrics ?? null,
  });
}

export const usePlatformStore = create<PlatformState>()(
  persist(
    (set, get) => ({
      health: null,
      tables: [],
      metrics: null,
      jobs: [],
      history: [],
      materializedViews: [],
      tasks: [],
      tableDetail: null,
      tableIndexes: null,
      sampleRows: [],
      sampleColumns: [],
      ingestJob: null,
      savedQueries: [],
      savedSearches: [],
      previousTasks: [],

      loading: false,
      error: "",

      sql: DEFAULT_SQL,
      previewQuery: "database",
      selectedTable: "",
      columns: [],
      rows: [],
      queryLoading: false,
      queryError: "",
      lastExplainText: null,
      lastMetrics: null,

      ingestTable: "passages",
      ingestSource: "file",
      ingestFormat: "jsonl",
      ingestDropTable: false,
      ingestUploading: false,
      ingestRowsIngested: 0,
      ingestBulkActive: false,
      ingestLimit: 0,
      hfDataset: "Tevatron/msmarco-passage-corpus",
      hfConfig: "",
      hfSplit: "train",
      hfTextColumn: "text",

      pollTimer: null,
      ingestPollTimer: null,

      hydrate: async () => {
        set({ loading: true, error: "" });
        try {
          const [health, tables, metrics, jobs, history, mvs, tasks] = await Promise.all([
            fetchHealth(),
            fetchTables(),
            fetchMetrics(),
            fetchJobs(),
            fetchQueryHistory(),
            fetchMaterializedViews(),
            fetchTasks(),
          ]);
          set({
            health,
            tables,
            metrics,
            jobs,
            history: [...history].reverse(),
            materializedViews: mvs,
            tasks: [...tasks].reverse(),
            selectedTable: tables[0]?.name ?? "",
            ingestTable: tables[0]?.name ?? "passages",
            loading: false,
          });
          await get().syncRunningIngestJob();
        } catch (err) {
          set({
            loading: false,
            error: err instanceof Error ? err.message : String(err),
          });
        }
      },

      syncRunningIngestJob: async () => {
        try {
          const jobs = await fetchIngestJobs();
          const running = [...jobs].reverse().find((j) => j.state === "running");
          if (running) {
            set({ ingestJob: running, ingestBulkActive: true });
            void get().watchIngestJob(running.id).catch(() => {});
          }
        } catch {
          /* best-effort */
        }
      },

      refreshAll: async () => {
        await Promise.all([
          get().refreshMetrics(),
          get().refreshJobs(),
          get().refreshTables(),
          get().refreshHistory(),
          get().refreshTasks(),
          get().refreshMaterializedViews(),
        ]);
      },

      refreshMetrics: async () => {
        try {
          set({ metrics: await fetchMetrics() });
        } catch {
          /* best-effort */
        }
      },

      refreshJobs: async () => {
        try {
          set({ jobs: await fetchJobs() });
        } catch {
          /* best-effort */
        }
      },

      refreshTables: async () => {
        try {
          const tables = await fetchTables();
          const { selectedTable } = get();
          set({
            tables,
            selectedTable:
              selectedTable && tables.some((t) => t.name === selectedTable)
                ? selectedTable
                : (tables[0]?.name ?? ""),
          });
        } catch {
          /* best-effort */
        }
      },

      refreshHistory: async () => {
        try {
          const history = await fetchQueryHistory();
          set({ history: [...history].reverse() });
        } catch {
          /* best-effort */
        }
      },

      refreshTasks: async () => {
        try {
          const tasks = await fetchTasks();
          const reversed = [...tasks].reverse();
          set({ previousTasks: get().tasks, tasks: reversed });
        } catch {
          /* best-effort */
        }
      },

      refreshMaterializedViews: async () => {
        try {
          set({ materializedViews: await fetchMaterializedViews() });
        } catch {
          /* best-effort */
        }
      },

      startPolling: (ms = 5000) => {
        const { pollTimer, stopPolling } = get();
        if (pollTimer) stopPolling();
        const tick = () => {
          const state = get();
          const busy =
            state.tasks.some((t) => t.state === "running") ||
            state.jobs.some((j) => j.state === "building") ||
            state.ingestJob?.state === "running";
          const interval = busy ? 2000 : ms;
          void state.refreshMetrics();
          void state.refreshJobs();
          void state.refreshTables();
          void state.refreshTasks();
          const next = window.setTimeout(tick, interval);
          set({ pollTimer: next });
        };
        tick();
      },

      stopPolling: () => {
        const { pollTimer } = get();
        if (pollTimer) window.clearTimeout(pollTimer);
        set({ pollTimer: null });
      },

      setSql: (sql) => set({ sql }),
      setPreviewQuery: (previewQuery) => set({ previewQuery }),
      setSelectedTable: (name) => set({ selectedTable: name }),
      openQueryFromHistory: (query) =>
        set({ sql: query, queryError: "", lastExplainText: null }),

      fetchTableDetailAction: async (name) => {
        try {
          const detail = await fetchTableDetail(name);
          set({ tableDetail: detail });
        } catch (err) {
          set({ error: err instanceof Error ? err.message : String(err) });
        }
      },

      fetchTableSampleAction: async (name) => {
        try {
          const data = await fetchTableSample(name, 20);
          set({
            sampleColumns: data.columns ?? [],
            sampleRows: (data.rows ?? []) as Record<string, unknown>[],
          });
        } catch (err) {
          set({ error: err instanceof Error ? err.message : String(err) });
        }
      },

      runSqlQuery: async (explain = false) => {
        let { sql } = get();
        if (explain && !sql.trim().toUpperCase().startsWith("EXPLAIN")) {
          sql = `EXPLAIN ${sql}`;
        }
        set({ queryLoading: true, queryError: "" });
        try {
          const data = await runSql(sql);
          applySqlResult(data, set);
          set({ queryLoading: false });
          await get().refreshHistory();
          await get().refreshMetrics();
        } catch (err) {
          set({
            queryLoading: false,
            queryError: err instanceof Error ? err.message : String(err),
          });
          await get().refreshHistory();
        }
      },

      runPreview: async () => {
        const { selectedTable, previewQuery, tables } = get();
        const table = selectedTable || tables[0]?.name;
        if (!table) return;
        set({ queryLoading: true, queryError: "" });
        try {
          const data = await runQueryPreview(table, previewQuery, 8);
          applySqlResult(data.result, set);
          set({ queryLoading: false });
        } catch (err) {
          set({
            queryLoading: false,
            queryError: err instanceof Error ? err.message : String(err),
          });
        }
      },

      finishTableAction: async (name, compact = false) => {
        await finishTable(name, compact);
        await get().refreshTasks();
        await get().refreshJobs();
      },

      resumeTableAction: async (name, compact = false) => {
        await resumeTable(name, compact);
        await get().refreshTasks();
        await get().refreshJobs();
      },

      dropTableAction: async (name) => {
        await dropTable(name);
        set({ tableDetail: null });
        await get().hydrate();
      },

      setIngestTable: (ingestTable) => set({ ingestTable }),
      setIngestSource: (ingestSource) => set({ ingestSource }),
      setIngestFormat: (ingestFormat) => set({ ingestFormat }),
      setIngestDropTable: (ingestDropTable) => set({ ingestDropTable }),
      setIngestLimit: (ingestLimit) => set({ ingestLimit }),
      setHfDataset: (hfDataset) => set({ hfDataset }),
      setHfConfig: (hfConfig) => set({ hfConfig }),
      setHfSplit: (hfSplit) => set({ hfSplit }),
      setHfTextColumn: (hfTextColumn) => set({ hfTextColumn }),

      beginIngestAction: async () => {
        const { ingestTable, ingestDropTable } = get();
        set({ error: "" });
        const res = await beginIngest(ingestTable, ingestDropTable);
        set({ ingestBulkActive: res.bulk_active });
        await get().refreshTables();
      },

      stopIngestJobWatch: () => {
        const t = get().ingestPollTimer;
        if (t) window.clearTimeout(t);
        set({ ingestPollTimer: null });
        ingestWatchJobId = null;
        ingestWatchPromise = null;
      },

      watchIngestJob: (jobId) => {
        if (ingestWatchJobId === jobId && ingestWatchPromise) {
          return ingestWatchPromise;
        }
        get().stopIngestJobWatch();

        let pollFailures = 0;
        ingestWatchJobId = jobId;
        ingestWatchPromise = new Promise<IngestJob>((resolve, reject) => {
          const tick = async () => {
            try {
              const job = await fetchIngestJob(jobId);
              pollFailures = 0;
              const merged = mergeIngestJob(get().ingestJob, job, jobId);
              set({
                ingestJob: merged,
                ingestRowsIngested: merged.rows_ingested,
                ingestBulkActive:
                  merged.state === "running" || merged.state === "done",
              });
              if (merged.state === "done") {
                get().stopIngestJobWatch();
                await get().refreshTables();
                resolve(merged);
                return;
              }
              if (merged.state === "failed" || merged.state === "cancelled") {
                get().stopIngestJobWatch();
                reject(new Error(merged.message ?? merged.state));
                return;
              }
              const timer = window.setTimeout(() => void tick(), 1000);
              set({ ingestPollTimer: timer });
            } catch (err) {
              pollFailures += 1;
              if (pollFailures < 5) {
                const timer = window.setTimeout(
                  () => void tick(),
                  1000 * pollFailures,
                );
                set({ ingestPollTimer: timer });
                return;
              }
              get().stopIngestJobWatch();
              reject(err instanceof Error ? err : new Error(String(err)));
            }
          };
          void tick();
        });
        return ingestWatchPromise;
      },

      uploadIngestFileAction: async (file) => {
        const { ingestTable, ingestFormat, ingestLimit } = get();
        set({ error: "" });
        const res = await uploadIngestFile(
          ingestTable,
          ingestFormat,
          file,
          ingestLimit,
        );
        const job = await fetchIngestJob(res.job_id);
        set({ ingestJob: job, ingestBulkActive: true });
        return res.job_id;
      },

      ingestFromHfAction: async () => {
        const {
          ingestTable,
          hfDataset,
          hfConfig,
          hfSplit,
          hfTextColumn,
          ingestLimit,
        } = get();
        set({ error: "" });
        const res = await ingestFromHf({
          table: ingestTable,
          dataset: hfDataset,
          config: hfConfig || null,
          split: hfSplit,
          text_column: hfTextColumn,
          limit: ingestLimit,
        });
        const job = await fetchIngestJob(res.job_id);
        set({ ingestJob: job, ingestBulkActive: true });
        return res.job_id;
      },

      pollIngestJob: async (jobId) => {
        const job = await fetchIngestJob(jobId);
        set({
          ingestJob: job,
          ingestRowsIngested: job.rows_ingested,
        });
        if (job.state === "done") {
          await get().refreshTables();
        }
        return job;
      },

      cancelIngestJobAction: async (jobId) => {
        await cancelIngestJob(jobId);
        await get().pollIngestJob(jobId);
      },

      compactTableAction: async (name, full = false) => {
        await compactTable(name, full);
        await get().refreshTasks();
        await get().refreshJobs();
      },

      fetchTableIndexesAction: async (name) => {
        try {
          const tableIndexes = await fetchTableIndexes(name);
          set({ tableIndexes });
        } catch (err) {
          set({ error: err instanceof Error ? err.message : String(err) });
        }
      },

      createMvAction: async (name, query) => {
        await createMaterializedView(name, query);
        await get().refreshMaterializedViews();
      },

      refreshMvAction: async (name) => {
        await refreshMaterializedView(name);
        await get().refreshMaterializedViews();
      },

      dropMvAction: async (name) => {
        await dropMaterializedView(name);
        await get().refreshMaterializedViews();
      },

      addSavedQuery: (name, sql) => {
        const id = crypto.randomUUID();
        set({ savedQueries: [...get().savedQueries, { id, name, sql }] });
      },

      removeSavedQuery: (id) => {
        set({ savedQueries: get().savedQueries.filter((q) => q.id !== id) });
      },

      addSavedSearch: (name, request) => {
        const id = crypto.randomUUID();
        set({ savedSearches: [...get().savedSearches, { id, name, request }] });
      },

      removeSavedSearch: (id) => {
        set({ savedSearches: get().savedSearches.filter((s) => s.id !== id) });
      },

      loadSavedQuery: (id) => {
        const q = get().savedQueries.find((s) => s.id === id);
        if (q) set({ sql: q.sql, queryError: "", lastExplainText: null });
      },

      finishIngestAction: async (compact = false) => {
        const { ingestTable } = get();
        await finishIngest(ingestTable, compact);
        set({ ingestBulkActive: false });
        await get().refreshTasks();
        await get().refreshJobs();
        await get().refreshTables();
      },
    }),
    {
      name: "toradb-platform",
      partialize: (state) => ({
        sql: state.sql,
        savedQueries: state.savedQueries,
        savedSearches: state.savedSearches,
      }),
    },
  ),
);

export { cacheHitRatio };
