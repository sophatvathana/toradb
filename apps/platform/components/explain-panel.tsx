import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

export function ExplainPanel({ text }: { text: string | null }) {
  if (!text) return null;
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
