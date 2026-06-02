import * as React from "react";

export function HighlightedSnippet({
  snippet,
  className,
}: {
  snippet: string;
  className?: string;
}) {
  const parts = React.useMemo(() => splitSnippet(snippet), [snippet]);
  return (
    <p className={className}>
      {parts.map((part, i) =>
        part.match ? (
          <em
            key={i}
            className="rounded-sm bg-primary/25 not-italic font-medium text-primary-foreground"
          >
            {part.text}
          </em>
        ) : (
          <React.Fragment key={i}>{part.text}</React.Fragment>
        ),
      )}
    </p>
  );
}

type SnippetPart = { text: string; match: boolean };

function splitSnippet(snippet: string): SnippetPart[] {
  const parts: SnippetPart[] = [];
  let rest = snippet;
  while (rest.length > 0) {
    const open = rest.indexOf("<em>");
    if (open === -1) {
      parts.push({ text: rest, match: false });
      break;
    }
    if (open > 0) {
      parts.push({ text: rest.slice(0, open), match: false });
    }
    const close = rest.indexOf("</em>", open + 4);
    if (close === -1) {
      parts.push({ text: rest.slice(open), match: false });
      break;
    }
    parts.push({ text: rest.slice(open + 4, close), match: true });
    rest = rest.slice(close + 5);
  }
  return parts;
}
