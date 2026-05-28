import type { QueryMetricsResponse } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

export function QueryMetricsCard({ metrics }: { metrics: QueryMetricsResponse | null }) {
  if (!metrics) return null;
  const items = [
    ["Tier-1 candidates", metrics.tier1_candidates],
    ["Tier-2 candidates", metrics.tier2_candidates],
    ["Tier-3 candidates", metrics.tier3_candidates],
    ["Segments scanned", metrics.segments_scanned],
    ["Segment workers", metrics.segment_workers],
    ["Cache hits", metrics.cache_hits],
    ["IO bytes", metrics.io_bytes],
    ["Decompressions", metrics.decompressions],
  ];
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Query metrics</CardTitle>
      </CardHeader>
      <CardContent>
        <dl className="grid grid-cols-2 gap-2 text-sm">
          {items.map(([label, value]) => (
            <div key={label} className="flex justify-between gap-2 rounded border border-border bg-muted/30 px-2 py-1">
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="font-medium">{value}</dd>
            </div>
          ))}
        </dl>
      </CardContent>
    </Card>
  );
}
