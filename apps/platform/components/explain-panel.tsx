import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type {
  DropRecord,
  DropStage,
  ProvenanceRecord,
  ScoredDoc,
  TierTrace,
} from "@/lib/api";

const DROP_STAGE_LABEL: Record<DropStage, string> = {
  metadata_filter: "Metadata filter",
  tier1_budget_cut: "Tier-1 budget",
  tier2_budget_cut: "Tier-2 budget",
  crag_filter: "CRAG filter",
  tier3_budget_cut: "Tier-3 budget",
};

function usToMs(us: number): string {
  return `${(us / 1000).toFixed(2)} ms`;
}

/** Horizontal bar comparing one tier's latency against the max across tiers. */
function LatencyBar({ us, maxUs }: { us: number; maxUs: number }) {
  const pct = maxUs > 0 ? Math.max(2, (us / maxUs) * 100) : 0;
  return (
    <div className="flex items-center gap-2">
      <div className="h-2 w-24 overflow-hidden rounded-full bg-muted">
        <div className="h-full rounded-full bg-primary" style={{ width: `${pct}%` }} />
      </div>
      <span className="font-mono text-xs text-muted-foreground">{usToMs(us)}</span>
    </div>
  );
}

function CandidateChips({ docs, max = 8 }: { docs: ScoredDoc[]; max?: number }) {
  if (docs.length === 0) {
    return <span className="text-xs text-muted-foreground">none</span>;
  }
  const shown = docs.slice(0, max);
  return (
    <div className="flex flex-wrap gap-1">
      {shown.map((d) => (
        <Badge key={d.id} variant="outline" className="font-mono">
          #{d.id}
          <span className="ml-1 text-muted-foreground">{d.score.toFixed(3)}</span>
        </Badge>
      ))}
      {docs.length > max && (
        <span className="text-xs text-muted-foreground">+{docs.length - max} more</span>
      )}
    </div>
  );
}

function TierRow({
  label,
  count,
  latencyUs,
  maxUs,
  children,
}: {
  label: string;
  count: number;
  latencyUs: number;
  maxUs: number;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-md border border-border p-3">
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold">{label}</span>
          <Badge variant="secondary">{count}</Badge>
        </div>
        <LatencyBar us={latencyUs} maxUs={maxUs} />
      </div>
      {children}
    </div>
  );
}

function DropsTable({ drops }: { drops: DropRecord[] }) {
  if (drops.length === 0) return null;
  // Group by stage for a readable "what got dropped and why" view.
  const byStage = new Map<DropStage, DropRecord[]>();
  for (const d of drops) {
    const arr = byStage.get(d.stage) ?? [];
    arr.push(d);
    byStage.set(d.stage, arr);
  }
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Stage</TableHead>
          <TableHead>Dropped</TableHead>
          <TableHead>Doc IDs</TableHead>
          <TableHead>Reason</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {Array.from(byStage.entries()).map(([stage, recs]) => (
          <TableRow key={stage}>
            <TableCell>
              <Badge variant="warning">{DROP_STAGE_LABEL[stage] ?? stage}</Badge>
            </TableCell>
            <TableCell className="font-mono">{recs.length}</TableCell>
            <TableCell className="font-mono text-xs">
              {recs
                .slice(0, 12)
                .map((r) => `#${r.id}`)
                .join(", ")}
              {recs.length > 12 ? ` +${recs.length - 12}` : ""}
            </TableCell>
            <TableCell className="text-xs text-muted-foreground">
              {recs[0]?.reason}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

function tierCount(t: TierTrace): number {
  return Math.max(t.bm25_candidates.length, t.hnsw_candidates.length, t.rrf_merged.length);
}

export function ExplainPanel({
  text,
  provenance,
}: {
  text: string | null;
  provenance?: ProvenanceRecord | null;
}) {
  const [showRaw, setShowRaw] = useState(false);
  if (!provenance && !text) return null;

  if (!provenance) {
    // No structured record (e.g. legacy response) — fall back to plain text.
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Query plan</CardTitle>
        </CardHeader>
        <CardContent>
          <pre className="overflow-x-auto rounded-md border border-border bg-muted/40 p-3 font-mono text-xs leading-relaxed text-foreground">
            {text}
          </pre>
        </CardContent>
      </Card>
    );
  }

  const { tier1, tier2, tier3 } = provenance;
  const maxUs = Math.max(tier1.latency_us, tier2.latency_us, tier3.latency_us, 1);
  const allDrops = [...tier1.drops, ...tier2.drops, ...tier3.drops];

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="text-base">
          Retrieval provenance
          <span className="ml-2 font-mono text-xs text-muted-foreground">
            {provenance.total_latency_ms.toFixed(2)} ms total
          </span>
        </CardTitle>
        {text && (
          <button
            type="button"
            className="text-xs text-muted-foreground underline-offset-2 hover:underline"
            onClick={() => setShowRaw((v) => !v)}
          >
            {showRaw ? "Hide" : "Show"} raw plan
          </button>
        )}
      </CardHeader>
      <CardContent className="space-y-3">
        {/* Tier flow */}
        <TierRow
          label="Tier 1 · retrieve"
          count={tier1.bm25_candidates.length + tier1.hnsw_candidates.length}
          latencyUs={tier1.latency_us}
          maxUs={maxUs}
        >
          <div className="grid gap-2 sm:grid-cols-2">
            <div>
              <div className="mb-1 text-xs font-medium text-muted-foreground">
                BM25 (sparse) · {tier1.bm25_candidates.length}
              </div>
              <CandidateChips docs={tier1.bm25_candidates} />
            </div>
            <div>
              <div className="mb-1 text-xs font-medium text-muted-foreground">
                HNSW (dense) · {tier1.hnsw_candidates.length}
              </div>
              <CandidateChips docs={tier1.hnsw_candidates} />
            </div>
          </div>
        </TierRow>

        <TierRow
          label="Tier 2 · fuse (RRF)"
          count={tier2.rrf_merged.length}
          latencyUs={tier2.latency_us}
          maxUs={maxUs}
        >
          <CandidateChips docs={tier2.rrf_merged} />
        </TierRow>

        <TierRow
          label="Tier 3 · rank"
          count={provenance.final_ids.length || tierCount(tier3)}
          latencyUs={tier3.latency_us}
          maxUs={maxUs}
        >
          <div className="flex flex-wrap gap-1">
            {provenance.final_ids.map((id, i) => (
              <Badge key={id} variant={i === 0 ? "success" : "outline"} className="font-mono">
                #{id}
              </Badge>
            ))}
          </div>
        </TierRow>

        {/* Drops */}
        {allDrops.length > 0 && (
          <div>
            <div className="mb-2 text-sm font-semibold">
              Dropped <span className="text-muted-foreground">({allDrops.length})</span>
            </div>
            <DropsTable drops={allDrops} />
          </div>
        )}

        {provenance.score_breakdown && provenance.score_breakdown.length > 0 && (
          <div>
            <div className="mb-2 text-sm font-semibold">
              Score breakdown
              <span className="ml-2 text-xs font-normal text-muted-foreground">
                base × boost × decay → final
              </span>
            </div>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Doc</TableHead>
                  <TableHead>Base</TableHead>
                  <TableHead>Boost</TableHead>
                  <TableHead>Decay</TableHead>
                  <TableHead>Final</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {provenance.score_breakdown.slice(0, 20).map((b) => (
                  <TableRow key={b.id}>
                    <TableCell className="font-mono">#{b.id}</TableCell>
                    <TableCell className="font-mono text-xs">{b.base.toFixed(4)}</TableCell>
                    <TableCell className="font-mono text-xs">
                      {b.boost === 1 ? (
                        <span className="text-muted-foreground">1.00</span>
                      ) : (
                        <span className="text-primary">×{b.boost.toFixed(2)}</span>
                      )}
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {b.decay === 1 ? (
                        <span className="text-muted-foreground">1.00</span>
                      ) : (
                        <span className="text-warning">×{b.decay.toFixed(2)}</span>
                      )}
                    </TableCell>
                    <TableCell className="font-mono text-xs font-medium">
                      {b.final_score.toFixed(4)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}

        {showRaw && text && (
          <pre className="overflow-x-auto rounded-md border border-border bg-muted/40 p-3 font-mono text-xs leading-relaxed text-foreground">
            {text}
          </pre>
        )}
      </CardContent>
    </Card>
  );
}
