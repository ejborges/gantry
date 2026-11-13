// The Markdown editor used everywhere text is written: messages on work items and requests,
// descriptions, and pages. One component, so a fix or a new button reaches all of them.
//
// It shows the Markdown as typed, the result as it will be read, or both next to each other.
// Mentions (@), links to work and pages ([[), dates, files, pictures with a chosen size,
// diagrams and embedded content are inserted from the toolbar or typed directly.
import { lazy, Suspense, useEffect, useId, useMemo, useRef, useState } from "react";
import { EditorSelection, EditorState, Compartment } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, highlightActiveLine, placeholder as placeholderExt } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import { autocompletion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";
import { yCollab, yUndoManagerKeymap } from "y-codemirror.next";
import {
  AppWindow, AtSign, Bold, CalendarDays, ChartNoAxesColumn, Code, Columns2, Ellipsis, Eye, Hash, Heading2, ImageDown, Italic, Link as LinkIcon, List, ListOrdered, ListTodo, Paperclip, PenLine, Quote, Shapes, Sigma, SquareCode,
  Strikethrough, Table, TriangleAlert, Workflow,
} from "lucide-react";
import { api, upload, type Attachment, type Person, type Refs } from "../api";
import { Markdown, Menu, Spinner, toast } from "../ui";
import Calendar from "../components/Calendar";
import { parseDiagram, serializeDiagram, type Diagram } from "../lib/diagram";
import type { SharedDoc } from "../realtime";

const DiagramDialog = lazy(() => import("./Diagram"));

export type EditorMode = "write" | "split" | "preview";

export interface MarkdownEditorProps {
  value: string;
  onChange: (markdown: string) => void;
  /** What this text is, for screen readers: "Message", "Description", "Page content". */
  label: string;
  placeholder?: string;
  /** Message-sized: shorter, with the less common tools behind "More". */
  compact?: boolean;
  /** Where files go (an attachments address). Without it, attaching is not offered. */
  uploadTo?: string;
  attachments?: Attachment[];
  onUploaded?: (file: Attachment) => void;
  /** Ctrl+Enter (Cmd+Enter on a Mac). */
  onSubmit?: () => void;
  /** Edit together with everyone else who has this open. */
  shared?: SharedDoc | null;
  /** The chosen layout is remembered under this name on this device. */
  storageKey?: string;
  autoFocus?: boolean;
  people?: Person[];
  defaultMode?: EditorMode;
}

export const IMAGE_SIZES = [
  { label: "Small", width: 240 },
  { label: "Medium", width: 480 },
  { label: "Large", width: 800 },
];

const SNIPPETS = {
  table: "\n| Column A | Column B |\n|---|---|\n|  |  |\n",
  code: "\n```\n\n```\n",
  mermaid: "\n```mermaid\nflowchart LR\n  A[Start] --> B{Decision}\n  B -- yes --> C[Do it]\n  B -- no --> D[Stop]\n```\n",
  tasks: "\n- [ ] First task\n- [ ] Second task\n",
  callout: "\n> [!note] Title\n> What the reader should notice.\n",
  math: "\n```math\nE = mc^2\n```\n",
  embed: "\n```embed\n<svg viewBox=\"0 0 200 60\" width=\"200\" role=\"img\" aria-label=\"Example\">\n  <rect width=\"200\" height=\"60\" rx=\"8\" fill=\"currentColor\" opacity=\"0.1\"/>\n  <text x=\"100\" y=\"36\" text-anchor=\"middle\" fill=\"currentColor\">Your HTML or SVG</text>\n</svg>\n```\n",
  viz: "\n```viz chartjs\nconst s = await gantry.api.get('/api/v1/analytics/items', { group_by: 'status_category' });\nconst canvas = root.appendChild(document.createElement('canvas'));\nconst chart = new Chart(canvas, {\n  type: 'doughnut',\n  data: { labels: s.groups.map(g => g.label), datasets: [{ data: s.groups.map(g => g.value), backgroundColor: [gantry.colors.todo, gantry.colors.progress, gantry.colors.done] }] },\n  options: { maintainAspectRatio: false },\n});\ngantry.cleanup(() => chart.destroy());\n```\n",
};

/** The image the cursor is on, if any: where it is in the line and its current width. */
export function imageAt(line: string, column: number): { from: number; to: number; alt: string; target: string; wiki: boolean; width: number | null } | null {
  const patterns: [RegExp, boolean][] = [[/!\[([^\]\n]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)(\{[^}\n]*\})?/g, false], [/!\[\[([^\]|\n]+)(?:\|([^\]\n]*))?\]\]/g, true]];
  for (const [re, wiki] of patterns) {
    let m: RegExpExecArray | null;
    while ((m = re.exec(line))) {
      if (column < m.index || column > m.index + m[0].length) continue;
      if (wiki) return { from: m.index, to: m.index + m[0].length, alt: "", target: m[1], wiki, width: /^\d+/.test(m[2] ?? "") ? parseInt(m[2], 10) : null };
      const size = /\|\s*(\d{1,4})(?:x\d+)?\s*$/.exec(m[1]);
      return { from: m.index, to: m.index + m[0].length, alt: size ? m[1].slice(0, size.index).trim() : m[1], target: m[2], wiki, width: size ? parseInt(size[1], 10) : null };
    }
  }
  return null;
}

/** The image written again with the given width, or at its own size when `width` is null. */
export function resizeImage(img: NonNullable<ReturnType<typeof imageAt>>, width: number | null): string {
  if (img.wiki) return width ? `![[${img.target}|${width}]]` : `![[${img.target}]]`;
  return `![${img.alt}${width ? `|${width}` : ""}](${img.target})`;
}

/** The ```diagram block around a line, as line numbers (1-based, inclusive of both fences). */
export function diagramBlock(lines: string[], at: number): { from: number; to: number; body: string } | null {
  let start = -1;
  for (let i = Math.min(at, lines.length) - 1; i >= 0; i--) {
    if (/^\s{0,3}```+\s*diagram\b/.test(lines[i])) {
      start = i;
      break;
    }
    if (/^\s{0,3}```/.test(lines[i]) && i !== at - 1) return null;
  }
  if (start < 0) return null;
  for (let j = start + 1; j < lines.length; j++) if (/^\s{0,3}```+\s*$/.test(lines[j])) return j + 1 >= at ? { from: start + 1, to: j + 1, body: lines.slice(start + 1, j).join("\n") } : null;
  return null;
}

function useRefs(source: string, on: boolean): Refs | undefined {
  const [refs, setRefs] = useState<Refs>();
  const needed = on && /@[a-z0-9]|\[\[|[A-Z][A-Z0-9]+-\d/i.test(source);
  useEffect(() => {
    if (!needed) return;
    const t = setTimeout(() => {
      api<Refs>("POST", "/api/v1/refs/preview", { text: source.slice(0, 60_000) }).then(setRefs).catch(() => undefined);
    }, 400);
    return () => clearTimeout(t);
  }, [source, needed]);
  return needed ? refs : undefined;
}

/**
 * A cursor at the point where a long line wraps remembers which of the two rows it is on.
 * When the editor then loses focus, CodeMirror puts the browser's selection back at that
 * point to keep the cursor on its row, and in Chromium that pulls keyboard focus back into
 * the editor: what is typed next, in a dialog or a field beside the editor, lands in the
 * text. Forgetting the side as focus leaves costs nothing and keeps focus where it went.
 */
function releaseCursor(view: EditorView) {
  const sel = view.state.selection;
  if (!sel.ranges.some((r) => r.empty && r.assoc)) return;
  view.dispatch({ selection: EditorSelection.create(sel.ranges.map((r) => (r.empty ? EditorSelection.cursor(r.head) : r)), sel.mainIndex) });
}

/** The editor's accessible name and the hint shown while it is empty. */
function named(label: string, placeholder: string | undefined, hintId: string) {
  return [placeholderExt(placeholder ?? ""), EditorView.contentAttributes.of({ "aria-label": `${label} (Markdown)`, "aria-describedby": hintId, "aria-multiline": "true", role: "textbox" })];
}

export default function MarkdownEditor({ value, onChange, label, placeholder, compact, uploadTo, attachments, onUploaded, onSubmit, shared, storageKey, autoFocus, people, defaultMode }: MarkdownEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef({ onChange, onSubmit, people: people ?? [], uploadTo });
  latest.current = { onChange, onSubmit, people: people ?? [], uploadTo };
  const fileInput = useRef<HTMLInputElement>(null);
  const key = storageKey ? `gantry.editor.${storageKey}` : null;
  const [mode, setModeState] = useState<EditorMode>(() => {
    const saved = key ? (localStorage.getItem(key) as EditorMode | null) : null;
    return saved && ["write", "split", "preview"].includes(saved) ? saved : (defaultMode ?? (compact ? "write" : "split"));
  });
  const setMode = (m: EditorMode) => {
    setModeState(m);
    try {
      if (key) localStorage.setItem(key, m);
    } catch {
      /* not remembered on this device */
    }
  };
  const [preview, setPreview] = useState(value);
  const [uploading, setUploading] = useState(0);
  const [pop, setPop] = useState<"date" | "size" | null>(null);
  const [diagram, setDiagram] = useState<{ value: Diagram | null; range: { from: number; to: number } | null } | null>(null);
  const [own, setOwn] = useState<Attachment[]>([]);
  const files = useMemo(() => [...(attachments ?? []), ...own.filter((o) => !(attachments ?? []).some((a) => a.id === o.id))], [attachments, own]);
  const refs = useRefs(preview, mode !== "write");

  const naming = useRef(new Compartment());
  const hintId = useId();
  useEffect(() => {
    const ytext = shared?.ydoc.getText("t");
    const v = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: ytext ? ytext.toString() : value,
        extensions: [
          compact ? [] : [lineNumbers(), highlightActiveLine()],
          ytext ? [yCollab(ytext, shared!.awareness), keymap.of(yUndoManagerKeymap)] : [history(), keymap.of(historyKeymap)],
          keymap.of([
            { key: "Mod-b", run: () => (wrap("**"), true) },
            { key: "Mod-i", run: () => (wrap("*"), true) },
            { key: "Mod-k", run: () => (wrap("[", "](https://)"), true) },
            { key: "Mod-Enter", run: () => (latest.current.onSubmit ? (latest.current.onSubmit(), true) : false) },
            ...defaultKeymap,
            indentWithTab,
          ]),
          markdown(),
          syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
          autocompletion({ override: [complete], icons: false, activateOnTyping: true }),
          EditorView.lineWrapping,
          naming.current.of(named(label, placeholder, hintId)),
          new Compartment().of(EditorState.readOnly.of(!!shared && !shared.canWrite)),
          EditorView.updateListener.of((u) => u.docChanged && latest.current.onChange(u.state.doc.toString())),
          EditorView.domEventHandlers({
            paste: (e) => takeFiles(e.clipboardData?.files, e),
            drop: (e) => takeFiles(e.dataTransfer?.files, e),
            blur: (_e, view) => releaseCursor(view),
          }),
        ],
      }),
    });
    view.current = v;
    if (autoFocus) v.focus();
    return () => v.destroy();
    // The editor owns its document after mount; a different shared document means a new editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shared]);
  // What the box is called can change while it is open (a reply becomes an internal note).
  useEffect(() => {
    view.current?.dispatch({ effects: naming.current.reconfigure(named(label, placeholder, hintId)) });
  }, [label, placeholder]);
  // Content set from outside (a template, a cleared message). Not while editing together:
  // there the shared document is what everyone sees.
  useEffect(() => {
    const v = view.current;
    if (v && !shared && v.state.doc.toString() !== value) v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
    const t = setTimeout(() => setPreview(value), 200);
    return () => clearTimeout(t);
  }, [value, shared]);

  async function complete(ctx: CompletionContext): Promise<CompletionResult | null> {
    const mention = ctx.matchBefore(/(^|[\s(])@[\w.-]{0,30}$/);
    if (mention) {
      const from = mention.from + mention.text.indexOf("@");
      const q = mention.text.slice(mention.text.indexOf("@") + 1).toLowerCase();
      let list = latest.current.people;
      if (list.length === 0) list = await api<Person[]>("GET", `/api/v1/users?q=${encodeURIComponent(q)}&limit=12`).catch(() => []);
      const options = list
        .filter((p) => p.handle && (p.name.toLowerCase().includes(q) || p.handle.toLowerCase().includes(q)))
        .slice(0, 12)
        .map((p) => ({ label: `@${p.handle}`, displayLabel: p.name, detail: `@${p.handle}${p.kind === "agent" ? " · AI agent" : ""}`, apply: `@${p.handle} `, type: "text" }));
      return options.length ? { from, options, filter: false } : null;
    }
    const link = ctx.matchBefore(/!?\[\[[^\]|\n]{0,60}$/);
    if (link) {
      const open = link.text.indexOf("[[") + 2;
      const q = link.text.slice(open).trim();
      const mine = files.filter((f) => f.filename.toLowerCase().includes(q.toLowerCase())).slice(0, 5).map((f) => ({ label: f.filename, detail: "File", apply: `${f.filename}]]`, type: "text" }));
      if (q.length < 2) return mine.length ? { from: link.from + open, options: mine, filter: false } : null;
      const found = await api<{ results: { kind: string; label: string; title: string; url: string }[] }>("GET", `/api/v1/search?q=${encodeURIComponent(q)}&limit=8`).catch(() => ({ results: [] }));
      if (ctx.aborted) return null;
      const options = [
        ...mine,
        ...found.results.map((r) => {
          const itemKey = r.kind === "item" ? (r.url.split("/").pop() ?? "") : "";
          return { label: itemKey ? `${itemKey} ${r.title}` : r.title, detail: r.kind === "item" ? "Work item" : "Page", apply: `${itemKey || r.title}]]`, type: "text" };
        }),
      ];
      return options.length ? { from: link.from + open, options, filter: false } : null;
    }
    return null;
  }

  function wrap(before: string, after = before) {
    const v = view.current!;
    const { from, to } = v.state.selection.main;
    const text = v.state.sliceDoc(from, to) || "text";
    v.dispatch({ changes: { from, to, insert: `${before}${text}${after}` }, selection: { anchor: from + before.length, head: from + before.length + text.length } });
    v.focus();
  }
  function insert(text: string, select?: [number, number]) {
    const v = view.current!;
    const at = v.state.selection.main.to;
    v.dispatch({ changes: { from: at, insert: text }, selection: select ? { anchor: at + select[0], head: at + select[1] } : { anchor: at + text.length } });
    v.focus();
  }
  function linePrefix(prefix: string) {
    const v = view.current!;
    const { from, to } = v.state.selection.main;
    const changes = [];
    for (let n = v.state.doc.lineAt(from).number; n <= v.state.doc.lineAt(to).number; n++) changes.push({ from: v.state.doc.line(n).from, insert: prefix.replace("1.", `${n - v.state.doc.lineAt(from).number + 1}.`) });
    v.dispatch({ changes });
    v.focus();
  }
  function trigger(text: string) {
    insert(text);
    // Typing the opening characters is what brings up the suggestions.
    void import("@codemirror/autocomplete").then((m) => view.current && m.startCompletion(view.current));
  }

  function takeFiles(list: FileList | null | undefined, e: Event): boolean {
    if (!list || list.length === 0 || !latest.current.uploadTo) return false;
    e.preventDefault();
    void attach(Array.from(list));
    return true;
  }
  async function attach(list: File[]) {
    const to = latest.current.uploadTo;
    if (!to) return;
    for (const file of list) {
      setUploading((n) => n + 1);
      try {
        const a = await upload(to, file);
        const meta: Attachment = { id: a.id, filename: a.filename, content_type: file.type || "application/octet-stream", size: a.size };
        setOwn((o) => [...o, meta]);
        onUploaded?.(meta);
        const shown = /^(image|video|audio)\//.test(meta.content_type);
        const name = meta.filename.replace(/[[\]|]/g, " ");
        insert(`${shown ? "!" : ""}[${name}](/api/v1/attachments/${meta.id})${shown ? "\n" : " "}`);
        if (meta.content_type.startsWith("image/")) {
          // Leave the cursor on the picture and offer to make it smaller.
          const v = view.current!;
          const line = v.state.doc.lineAt(Math.max(0, v.state.selection.main.to - 2));
          v.dispatch({ selection: { anchor: line.from + 2 } });
          setPop("size");
        }
      } catch (err) {
        toast(err instanceof Error ? `${file.name}: ${err.message}` : `${file.name} could not be attached.`, "error");
      } finally {
        setUploading((n) => n - 1);
      }
    }
  }

  function currentImage() {
    const v = view.current;
    if (!v) return null;
    const pos = v.state.selection.main.head;
    const line = v.state.doc.lineAt(pos);
    const img = imageAt(line.text, pos - line.from);
    return img ? { ...img, line } : null;
  }
  function setImageWidth(width: number | null) {
    const v = view.current!;
    const img = currentImage();
    if (!img) return;
    v.dispatch({ changes: { from: img.line.from + img.from, to: img.line.from + img.to, insert: resizeImage(img, width) } });
    setPop(null);
    v.focus();
  }

  function openDiagram() {
    const v = view.current!;
    const lines = v.state.doc.toString().split("\n");
    const block = diagramBlock(lines, v.state.doc.lineAt(v.state.selection.main.head).number);
    if (!block) return setDiagram({ value: null, range: null });
    try {
      setDiagram({ value: parseDiagram(block.body), range: { from: v.state.doc.line(block.from).from, to: v.state.doc.line(block.to).to } });
    } catch {
      toast("The diagram here cannot be read. Fix its data, or delete the block and draw it again.", "error");
    }
  }
  function saveDiagram(d: Diagram) {
    const v = view.current!;
    const text = `\`\`\`diagram\n${serializeDiagram(d)}\n\`\`\``;
    if (diagram?.range) v.dispatch({ changes: { from: diagram.range.from, to: diagram.range.to, insert: text } });
    else insert(`\n${text}\n`);
    setDiagram(null);
  }

  const B = ({ name, onClick, children, pressed }: { name: string; onClick: () => void; children: React.ReactNode; pressed?: boolean }) => (
    <button type="button" className={`btn ghost icon sm ${pressed ? "on" : ""}`} title={name} aria-label={name} aria-pressed={pressed} onMouseDown={(e) => e.preventDefault()} onClick={onClick}>
      {children}
    </button>
  );
  const more: { name: string; icon: typeof Table; run: () => void }[] = [
    { name: "Table", icon: Table, run: () => insert(SNIPPETS.table) },
    { name: "Code block", icon: SquareCode, run: () => insert(SNIPPETS.code) },
    { name: "Callout", icon: TriangleAlert, run: () => insert(SNIPPETS.callout) },
    { name: "Formula", icon: Sigma, run: () => insert(SNIPPETS.math) },
    { name: "Diagram from text (Mermaid)", icon: Workflow, run: () => insert(SNIPPETS.mermaid) },
    { name: "Chart from a script", icon: ChartNoAxesColumn, run: () => insert(SNIPPETS.viz) },
    { name: "HTML or SVG", icon: AppWindow, run: () => insert(SNIPPETS.embed) },
  ];
  const img = pop === "size" ? currentImage() : null;

  return (
    <div className={`editor md-editor ${compact ? "compact" : ""} mode-${mode}`}>
      <div className="toolbar" role="toolbar" aria-label={`${label}: formatting`}>
        {!compact && <B name="Heading" onClick={() => linePrefix("## ")}><Heading2 /></B>}
        <B name="Bold (Ctrl+B)" onClick={() => wrap("**")}><Bold /></B>
        <B name="Italic (Ctrl+I)" onClick={() => wrap("*")}><Italic /></B>
        {!compact && <B name="Strikethrough" onClick={() => wrap("~~")}><Strikethrough /></B>}
        <B name="Code" onClick={() => wrap("`")}><Code /></B>
        <B name="Link (Ctrl+K)" onClick={() => wrap("[", "](https://)")}><LinkIcon /></B>
        <span className="sep" />
        <B name="Bulleted list" onClick={() => linePrefix("- ")}><List /></B>
        {!compact && <B name="Numbered list" onClick={() => linePrefix("1. ")}><ListOrdered /></B>}
        <B name="Checklist" onClick={() => linePrefix("- [ ] ")}><ListTodo /></B>
        <B name="Quote" onClick={() => linePrefix("> ")}><Quote /></B>
        <span className="sep" />
        <B name="Mention someone" onClick={() => trigger("@")}><AtSign /></B>
        <B name="Link a work item or page" onClick={() => trigger("[[")}><Hash /></B>
        <span className="menu-anchor">
          <B name="Insert a date" pressed={pop === "date"} onClick={() => setPop(pop === "date" ? null : "date")}><CalendarDays /></B>
          {pop === "date" && (
            <div className="menu left pop" role="dialog" aria-label="Insert a date">
              <Calendar
                onPick={(d) => {
                  insert(`[[${d}]] `);
                  setPop(null);
                }}
              />
            </div>
          )}
        </span>
        {uploadTo && (
          <>
            <input ref={fileInput} type="file" hidden multiple onChange={(e) => (void attach(Array.from(e.target.files ?? [])), (e.target.value = ""))} />
            <B name="Attach files" onClick={() => fileInput.current?.click()}>{uploading > 0 ? <Spinner /> : <Paperclip />}</B>
          </>
        )}
        <span className="menu-anchor">
          <B name="Image size" pressed={pop === "size"} onClick={() => setPop(pop === "size" ? null : "size")}><ImageDown /></B>
          {pop === "size" && (
            <div className="menu left pop" role="dialog" aria-label="Image size">
              {img ? (
                <div className="stack tight" style={{ padding: 10, width: 240 }}>
                  <span className="small muted">Show this image at</span>
                  <div className="row wrap">
                    {IMAGE_SIZES.map((s) => (
                      <button key={s.width} type="button" className={`btn sm ${img.width === s.width ? "primary" : ""}`} onClick={() => setImageWidth(s.width)}>
                        {s.label} <span className="muted small num">{s.width}</span>
                      </button>
                    ))}
                    <button type="button" className={`btn sm ${img.width === null ? "primary" : ""}`} onClick={() => setImageWidth(null)}>
                      Original
                    </button>
                  </div>
                  <form
                    className="row"
                    onSubmit={(e) => {
                      e.preventDefault();
                      const w = Number(new FormData(e.currentTarget).get("w"));
                      if (w >= 16 && w <= 4000) setImageWidth(Math.round(w));
                    }}
                  >
                    <input className="input" name="w" type="number" min={16} max={4000} defaultValue={img.width ?? ""} placeholder="Width in pixels" aria-label="Custom width in pixels" />
                    <button className="btn sm">Set</button>
                  </form>
                </div>
              ) : (
                <p className="small muted" style={{ padding: 10, width: 240 }}>Put the cursor on an image first. Images are attached with the paperclip, or by pasting or dropping a file.</p>
              )}
            </div>
          )}
        </span>
        <B name="Draw a diagram" onClick={openDiagram}><Shapes /></B>
        {compact ? (
          <Menu align="left" trigger={(open, toggle) => <B name="More to insert" pressed={open} onClick={toggle}><Ellipsis /></B>}>
            {(close) => (
              <>
                {more.map((m) => (
                  <button key={m.name} type="button" onClick={() => (m.run(), close())}>
                    <m.icon /> {m.name}
                  </button>
                ))}
              </>
            )}
          </Menu>
        ) : (
          more.map((m) => (
            <B key={m.name} name={m.name} onClick={m.run}>
              <m.icon />
            </B>
          ))
        )}
        <span className="grow" />
        <span className="mode-switch" role="group" aria-label="What to show">
          <button type="button" aria-pressed={mode === "write"} onClick={() => setMode("write")} title="The Markdown as typed">
            <PenLine /> <span className="hide-sm">Markdown</span>
          </button>
          <button type="button" aria-pressed={mode === "split"} onClick={() => setMode("split")} title="The Markdown and the result, together">
            <Columns2 /> <span className="hide-sm">Both</span>
          </button>
          <button type="button" aria-pressed={mode === "preview"} onClick={() => setMode("preview")} title="The result as it will be read">
            <Eye /> <span className="hide-sm">Result</span>
          </button>
        </span>
      </div>
      <div className="md-panes">
        <div ref={host} className="md-source" hidden={mode === "preview"} />
        {mode !== "write" && (
          <div className="preview" aria-label={`${label}: how it will read`} role="region" tabIndex={0}>
            <Markdown source={preview} refs={refs} attachments={files} compact={compact} empty="Nothing to show yet." />
          </div>
        )}
      </div>
      <div className="md-foot small muted" aria-live="polite">
        {uploading > 0 ? `Attaching ${uploading} ${uploading === 1 ? "file" : "files"}…` : shared && !shared.canWrite ? "You can follow this text but not change it." : compact ? "Markdown. @ to mention, [[ to link work or a page." : "Markdown, with GitHub, GitLab and Obsidian extensions. Type @ to mention someone and [[ to link work or a page."}
      </div>
      {/* Tab indents here, so the way out by keyboard is said to those who cannot see that it does. */}
      <span className="sr-only" id={hintId}>Tab indents. To move on with the keyboard, press Escape, then Tab.</span>
      {diagram && (
        <Suspense fallback={null}>
          <DiagramDialog value={diagram.value} onClose={() => setDiagram(null)} onSave={saveDiagram} />
        </Suspense>
      )}
    </div>
  );
}
