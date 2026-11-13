// What changed between two versions of a text, line by line, with the changed words marked.
// Side by side where there is room, one column on small screens. Additions and removals are
// told apart by a sign and a label as well as by color.
import { useEffect, useMemo, useState } from "react";
import { diffLines, diffWordsWithSpace } from "diff";
import { Columns2, Rows2 } from "lucide-react";

export interface Row {
  kind: "same" | "add" | "del" | "change";
  left?: { n: number; text: string };
  right?: { n: number; text: string };
}

const lines = (s: string) => (s.endsWith("\n") ? s.slice(0, -1) : s).split("\n");

/** Pair up the lines of two texts. A removal followed by an addition is one changed row. */
export function diffRows(before: string, after: string): Row[] {
  const rows: Row[] = [];
  let l = 0, r = 0;
  const parts = diffLines(before, after);
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i];
    if (!p.added && !p.removed) {
      for (const text of lines(p.value)) rows.push({ kind: "same", left: { n: ++l, text }, right: { n: ++r, text } });
    } else if (p.removed) {
      const gone = lines(p.value);
      const next = parts[i + 1]?.added ? lines(parts[++i].value) : [];
      const n = Math.max(gone.length, next.length);
      for (let k = 0; k < n; k++) {
        const a = k < gone.length ? { n: ++l, text: gone[k] } : undefined;
        const b = k < next.length ? { n: ++r, text: next[k] } : undefined;
        rows.push({ kind: a && b ? "change" : a ? "del" : "add", left: a, right: b });
      }
    } else {
      for (const text of lines(p.value)) rows.push({ kind: "add", right: { n: ++r, text } });
    }
  }
  return rows;
}

export function diffStats(rows: Row[]) {
  return { added: rows.filter((r) => r.kind === "add" || r.kind === "change").length, removed: rows.filter((r) => r.kind === "del" || r.kind === "change").length };
}

/** Rows to show: the changes with a few unchanged lines around each; long unchanged runs are folded. */
function fold(rows: Row[], context = 3): (Row | { gap: number; from: number })[] {
  const keep = new Uint8Array(rows.length);
  rows.forEach((row, i) => {
    if (row.kind === "same") return;
    for (let k = Math.max(0, i - context); k <= Math.min(rows.length - 1, i + context); k++) keep[k] = 1;
  });
  const out: (Row | { gap: number; from: number })[] = [];
  for (let i = 0; i < rows.length; i++) {
    if (keep[i]) {
      out.push(rows[i]);
      continue;
    }
    let j = i;
    while (j < rows.length && !keep[j]) j++;
    out.push({ gap: j - i, from: i });
    i = j - 1;
  }
  return out;
}

function Words({ a, b, side }: { a: string; b: string; side: "left" | "right" }) {
  const parts = useMemo(() => diffWordsWithSpace(a, b), [a, b]);
  return (
    <>
      {parts.map((p, i) => {
        if (p.added) return side === "right" ? <ins key={i}>{p.value}</ins> : null;
        if (p.removed) return side === "left" ? <del key={i}>{p.value}</del> : null;
        return <span key={i}>{p.value}</span>;
      })}
    </>
  );
}

const SIGN = { add: "+", del: "−", same: " ", change: "~" };
const WORD = { add: "Added", del: "Removed", same: "", change: "Changed" };

export default function DiffView({ before, after, beforeLabel = "Before", afterLabel = "After" }: { before: string; after: string; beforeLabel?: string; afterLabel?: string }) {
  const rows = useMemo(() => diffRows(before, after), [before, after]);
  const stats = diffStats(rows);
  const [open, setOpen] = useState<Set<number>>(new Set());
  const [narrow, setNarrow] = useState(() => window.matchMedia?.("(max-width: 860px)").matches ?? false);
  const [choice, setChoice] = useState<"split" | "inline" | null>(null);
  useEffect(() => {
    const mq = window.matchMedia?.("(max-width: 860px)");
    if (!mq) return;
    const on = () => setNarrow(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  const mode = narrow ? "inline" : (choice ?? "split");
  const shown = useMemo(() => {
    const folded = fold(rows);
    return folded.flatMap((x) => ("gap" in x && open.has(x.from) ? rows.slice(x.from, x.from + x.gap) : [x]));
  }, [rows, open]);
  if (stats.added === 0 && stats.removed === 0) return <p className="muted">No differences: the text is the same.</p>;
  const cell = (row: Row, side: "left" | "right") => {
    const here = side === "left" ? row.left : row.right;
    const kind = row.kind === "change" ? (side === "left" ? "del" : "add") : row.kind;
    // "diff-none", not "empty": that name belongs to the empty-state block and is not a table cell.
    if (!here) return <td className="diff-text diff-none" colSpan={2} />;
    return (
      <>
        <td className={`diff-n ${kind}`}>{here.n}</td>
        <td className={`diff-text ${kind}`}>
          <span className="diff-sign" aria-hidden="true">{SIGN[kind]}</span>
          {kind !== "same" && <span className="sr-only">{WORD[kind]}: </span>}
          {row.kind === "change" ? <Words a={row.left!.text} b={row.right!.text} side={side} /> : here.text || " "}
        </td>
      </>
    );
  };
  return (
    <div className="diff">
      <div className="diff-head">
        <span className="small">
          <strong className="diff-count add">+{stats.added}</strong> <strong className="diff-count del">−{stats.removed}</strong> <span className="muted">lines</span>
        </span>
        {!narrow && (
          <span className="mode-switch" role="group" aria-label="Layout">
            <button type="button" aria-pressed={mode === "split"} onClick={() => setChoice("split")}>
              <Columns2 /> Side by side
            </button>
            <button type="button" aria-pressed={mode === "inline"} onClick={() => setChoice("inline")}>
              <Rows2 /> Inline
            </button>
          </span>
        )}
      </div>
      <div className="diff-scroll" tabIndex={0} role="region" aria-label={`Changes from ${beforeLabel} to ${afterLabel}`}>
        <table className={`diff-table ${mode}`}>
          {/* The header cells span two columns each, so the widths are stated here. */}
          <colgroup>
            <col className="diff-col-n" />
            <col />
            {mode === "split" && <col className="diff-col-n" />}
            {mode === "split" && <col />}
          </colgroup>
          {mode === "split" && (
            <thead>
              <tr>
                <th colSpan={2}>{beforeLabel}</th>
                <th colSpan={2}>{afterLabel}</th>
              </tr>
            </thead>
          )}
          <tbody>
            {shown.map((x, i) => {
              if ("gap" in x)
                return (
                  <tr key={`g${x.from}`} className="diff-gap">
                    <td colSpan={4}>
                      <button type="button" className="btn ghost sm" onClick={() => setOpen((s) => new Set(s).add(x.from))}>
                        Show {x.gap} unchanged {x.gap === 1 ? "line" : "lines"}
                      </button>
                    </td>
                  </tr>
                );
              if (mode === "split")
                return (
                  <tr key={i}>
                    {cell(x, "left")}
                    {cell(x, "right")}
                  </tr>
                );
              // Inline: a changed line is shown as the old line, then the new one.
              return x.kind === "change" ? (
                [<tr key={`${i}a`}>{cell(x, "left")}</tr>, <tr key={`${i}b`}>{cell(x, "right")}</tr>]
              ) : (
                <tr key={i}>{cell(x, x.kind === "add" ? "right" : "left")}</tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
