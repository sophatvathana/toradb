"use client";

import { ChartPart } from "@/components/chat/parts/chart-part";
import { ErrorPart } from "@/components/chat/parts/error-part";
import { ReportPart } from "@/components/chat/parts/report-part";
import { SearchPart } from "@/components/chat/parts/search-part";
import { SqlPart } from "@/components/chat/parts/sql-part";
import { TextPart } from "@/components/chat/parts/text-part";
import type { ChatPart } from "@/lib/chat/types";

export function MessageParts({
  parts,
  onRetry,
}: {
  parts: ChatPart[];
  onRetry?: () => void;
}) {
  return (
    <div className="space-y-3">
      {parts.map((part, i) => {
        switch (part.type) {
          case "text":
            return <TextPart key={i} content={part.content} />;
          case "search_results":
            return <SearchPart key={i} part={part} />;
          case "sql_result":
            return <SqlPart key={i} part={part} />;
          case "chart":
            return <ChartPart key={i} part={part} />;
          case "report":
            return <ReportPart key={i} part={part} />;
          case "error":
            return <ErrorPart key={i} part={part} onRetry={onRetry} />;
          default:
            return null;
        }
      })}
    </div>
  );
}
