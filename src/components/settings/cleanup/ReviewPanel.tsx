/* eslint-disable i18next/no-literal-string -- DotFlow review UI is English-first (not yet localized). */
import React, { useEffect, useMemo, useState } from "react";
import { commands, type TextSuggestion } from "@/bindings";

// DotFlow — Grammarly-style review of a block of text. Harper (offline) returns each issue as a char span +
// replacements (`analyze_text`); here the user sees them underlined and accepts/rejects each. We analyze
// ONCE and track apply/ignore as flags over the ORIGINAL text's stable offsets, then compute the result by
// splicing the accepted fixes — so offsets never drift as fixes are toggled.

interface ReviewPanelProps {
  text: string;
  /** Called with the corrected text whenever the accepted set changes. */
  onResult?: (result: string) => void;
}

// Greedy non-overlapping filter (earliest-wins) so adjacent lints never fight over the same characters.
const dedupeOverlaps = (items: TextSuggestion[]): TextSuggestion[] => {
  const sorted = [...items].sort((a, b) => a.start - b.start);
  const out: TextSuggestion[] = [];
  let lastEnd = 0;
  for (const s of sorted) {
    if (s.start >= lastEnd && s.replacements.length > 0) {
      out.push(s);
      lastEnd = s.end;
    }
  }
  return out;
};

export const ReviewPanel: React.FC<ReviewPanelProps> = ({ text, onResult }) => {
  const [suggestions, setSuggestions] = useState<TextSuggestion[]>([]);
  const [applied, setApplied] = useState<Set<number>>(new Set());
  const [ignored, setIgnored] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(false);

  const chars = useMemo(() => Array.from(text), [text]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    commands
      .analyzeText(text)
      .then((s) => {
        if (cancelled) return;
        setSuggestions(dedupeOverlaps(s));
        setApplied(new Set());
        setIgnored(new Set());
      })
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [text]);

  // Result = original text with every accepted fix spliced in, applied back-to-front so offsets stay valid.
  const resultText = useMemo(() => {
    const out = [...chars];
    const toApply = suggestions
      .map((s, i) => ({ s, i }))
      .filter(({ i }) => applied.has(i))
      .sort((a, b) => b.s.start - a.s.start);
    for (const { s } of toApply) {
      const rep = Array.from(s.replacements[0] ?? "");
      out.splice(s.start, s.end - s.start, ...rep);
    }
    return out.join("");
  }, [chars, suggestions, applied]);

  useEffect(() => {
    onResult?.(resultText);
  }, [resultText, onResult]);

  const accept = (i: number, replacementIndex = 0) => {
    // Store the chosen replacement as first so resultText uses it.
    setSuggestions((prev) =>
      prev.map((s, idx) => {
        if (idx !== i || replacementIndex === 0) return s;
        const reps = [...s.replacements];
        const [chosen] = reps.splice(replacementIndex, 1);
        return { ...s, replacements: [chosen, ...reps] };
      }),
    );
    setApplied((prev) => new Set(prev).add(i));
    setIgnored((prev) => {
      const n = new Set(prev);
      n.delete(i);
      return n;
    });
  };

  const ignore = (i: number) => {
    setIgnored((prev) => new Set(prev).add(i));
    setApplied((prev) => {
      const n = new Set(prev);
      n.delete(i);
      return n;
    });
  };

  const acceptAll = () => setApplied(new Set(suggestions.map((_, i) => i)));

  // Build display segments over the original text.
  const segments = useMemo(() => {
    const sorted = suggestions
      .map((s, i) => ({ s, i }))
      .sort((a, b) => a.s.start - b.s.start);
    const segs: React.ReactNode[] = [];
    let cursor = 0;
    for (const { s, i } of sorted) {
      if (s.start > cursor) {
        segs.push(
          <span key={`p${cursor}`}>
            {chars.slice(cursor, s.start).join("")}
          </span>,
        );
      }
      const orig = chars.slice(s.start, s.end).join("");
      if (applied.has(i)) {
        segs.push(
          <span
            key={`a${i}`}
            className="text-accent bg-accent-tint rounded-sm px-0.5"
          >
            {s.replacements[0] || ""}
          </span>,
        );
      } else if (ignored.has(i)) {
        segs.push(<span key={`i${i}`}>{orig}</span>);
      } else {
        segs.push(
          <button
            key={`u${i}`}
            onClick={() => accept(i)}
            title={s.message}
            className="underline decoration-wavy decoration-amber-500 underline-offset-2 cursor-pointer hover:bg-amber-500/10 rounded-sm"
          >
            {orig}
          </button>,
        );
      }
      cursor = s.end;
    }
    if (cursor < chars.length) {
      segs.push(<span key="pend">{chars.slice(cursor).join("")}</span>);
    }
    return segs;
  }, [chars, suggestions, applied, ignored]);

  const openCount = suggestions.filter(
    (_, i) => !applied.has(i) && !ignored.has(i),
  ).length;

  if (loading) {
    return <div className="text-sm text-muted px-1 py-2">Checking…</div>;
  }

  if (suggestions.length === 0) {
    return (
      <div className="px-3 py-2 text-sm bg-panel border border-hairline rounded-lg text-muted">
        No issues found — looks clean. ✓
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {/* The text with issues underlined; click one to accept its top fix. */}
      <div className="px-3 py-2.5 text-sm leading-relaxed bg-panel border border-hairline rounded-lg whitespace-pre-wrap">
        {segments}
      </div>

      <div className="flex items-center justify-between">
        <span className="text-xs text-faint">
          {openCount === 0
            ? "All reviewed"
            : `${openCount} suggestion${openCount === 1 ? "" : "s"}`}
        </span>
        {openCount > 0 && (
          <button
            onClick={acceptAll}
            className="text-xs font-medium px-2.5 py-1 rounded-md bg-accent text-white hover:brightness-95"
          >
            Accept all
          </button>
        )}
      </div>

      {/* Per-issue cards for full control (multiple replacements + ignore). */}
      <div className="flex flex-col gap-1.5">
        {suggestions.map((s, i) =>
          applied.has(i) || ignored.has(i) ? null : (
            <div
              key={i}
              className="flex items-center gap-3 px-3 py-2 bg-panel border border-hairline rounded-lg"
            >
              <div className="flex-1 min-w-0">
                <div className="text-[13px]">
                  <span className="line-through text-muted">
                    {chars.slice(s.start, s.end).join("") || "∅"}
                  </span>{" "}
                  <span className="text-faint">→</span>{" "}
                  {s.replacements.slice(0, 3).map((r, ri) => (
                    <button
                      key={ri}
                      onClick={() => accept(i, ri)}
                      className="text-accent font-medium hover:underline mr-2"
                    >
                      {r || "(remove)"}
                    </button>
                  ))}
                </div>
                <div className="text-[11px] text-faint truncate">
                  {s.kind} · {s.message}
                </div>
              </div>
              <button
                onClick={() => ignore(i)}
                className="text-xs text-muted hover:text-text px-2 py-1 rounded-md hover:bg-text/8"
              >
                Ignore
              </button>
            </div>
          ),
        )}
      </div>
    </div>
  );
};
