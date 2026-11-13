// Flowcharts and block diagrams drawn in the diagram editor.
// A diagram is stored as JSON inside a ```diagram block of the Markdown text, so it is
// versioned, compared and searched with the page it belongs to. This module turns that JSON
// into an SVG picture; the editor (editors/Diagram.tsx) writes the same JSON.

export type Shape = "rect" | "round" | "diamond" | "ellipse" | "note" | "cylinder" | "text";
export const SHAPES: { key: Shape; label: string }[] = [
  { key: "rect", label: "Step" },
  { key: "round", label: "Start or end" },
  { key: "diamond", label: "Decision" },
  { key: "ellipse", label: "State" },
  { key: "cylinder", label: "Data store" },
  { key: "note", label: "Note" },
  { key: "text", label: "Text only" },
];
export const DIAGRAM_COLORS = ["neutral", "blue", "green", "amber", "red", "purple", "teal"] as const;
export type DiagramColor = (typeof DIAGRAM_COLORS)[number];

export interface DiagramNode {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  shape: Shape;
  text: string;
  color?: DiagramColor;
}
export interface DiagramEdge {
  id: string;
  from: string;
  to: string;
  label?: string;
  style?: "solid" | "dashed";
  arrow?: "end" | "both" | "none";
}
export interface Diagram {
  nodes: DiagramNode[];
  edges: DiagramEdge[];
}

const num = (v: unknown, fallback: number, lo: number, hi: number) => (typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : fallback);
const str = (v: unknown, max: number) => (typeof v === "string" ? v.slice(0, max) : "");

/** Read a diagram from text, keeping only what a diagram may contain. Throws on anything else. */
export function parseDiagram(text: string): Diagram {
  const raw = JSON.parse(text);
  if (!raw || typeof raw !== "object" || !Array.isArray(raw.nodes)) throw new Error("A diagram needs a list of nodes.");
  const nodes: DiagramNode[] = raw.nodes.slice(0, 300).map((n: any, i: number) => ({
    id: str(n?.id, 40) || `n${i}`,
    x: num(n?.x, 0, -20000, 20000),
    y: num(n?.y, 0, -20000, 20000),
    w: num(n?.w, 150, 30, 800),
    h: num(n?.h, 56, 20, 600),
    shape: SHAPES.some((s) => s.key === n?.shape) ? n.shape : "rect",
    text: str(n?.text, 400),
    color: (DIAGRAM_COLORS as readonly string[]).includes(n?.color) ? n.color : "neutral",
  }));
  const ids = new Set(nodes.map((n) => n.id));
  const edges: DiagramEdge[] = (Array.isArray(raw.edges) ? raw.edges : [])
    .slice(0, 600)
    .map((e: any, i: number) => ({ id: str(e?.id, 40) || `e${i}`, from: str(e?.from, 40), to: str(e?.to, 40), label: str(e?.label, 120), style: e?.style === "dashed" ? "dashed" : "solid", arrow: ["both", "none"].includes(e?.arrow) ? e.arrow : "end" }))
    .filter((e: DiagramEdge) => ids.has(e.from) && ids.has(e.to) && e.from !== e.to);
  return { nodes, edges };
}

export function serializeDiagram(d: Diagram): string {
  const round = (n: number) => Math.round(n);
  return JSON.stringify({
    nodes: d.nodes.map((n) => ({ id: n.id, x: round(n.x), y: round(n.y), w: round(n.w), h: round(n.h), shape: n.shape, text: n.text, ...(n.color && n.color !== "neutral" ? { color: n.color } : {}) })),
    edges: d.edges.map((e) => ({ id: e.id, from: e.from, to: e.to, ...(e.label ? { label: e.label } : {}), ...(e.style === "dashed" ? { style: "dashed" } : {}), ...(e.arrow && e.arrow !== "end" ? { arrow: e.arrow } : {}) })),
  });
}

export const EMPTY_DIAGRAM: Diagram = {
  nodes: [
    { id: "a", x: 0, y: 0, w: 150, h: 56, shape: "round", text: "Start", color: "neutral" },
    { id: "b", x: 220, y: 0, w: 150, h: 70, shape: "diamond", text: "Ready?", color: "blue" },
    { id: "c", x: 440, y: 0, w: 150, h: 56, shape: "rect", text: "Do the work", color: "neutral" },
  ],
  edges: [
    { id: "e1", from: "a", to: "b" },
    { id: "e2", from: "b", to: "c", label: "yes" },
  ],
};

/** Where the line from the center of `n` toward (tx, ty) leaves the node's outline. */
export function borderPoint(n: DiagramNode, tx: number, ty: number): { x: number; y: number } {
  const cx = n.x + n.w / 2, cy = n.y + n.h / 2;
  const dx = tx - cx, dy = ty - cy;
  if (dx === 0 && dy === 0) return { x: cx, y: cy };
  const hw = n.w / 2, hh = n.h / 2;
  let t: number;
  if (n.shape === "diamond") t = 1 / (Math.abs(dx) / hw + Math.abs(dy) / hh);
  else if (n.shape === "ellipse") t = 1 / Math.sqrt((dx * dx) / (hw * hw) + (dy * dy) / (hh * hh));
  else t = Math.min(hw / Math.abs(dx || 1e-9), hh / Math.abs(dy || 1e-9));
  return { x: cx + dx * t, y: cy + dy * t };
}

const NS = "http://www.w3.org/2000/svg";
function el<K extends keyof SVGElementTagNameMap>(name: K, attrs: Record<string, string | number>, parent?: Element): SVGElementTagNameMap[K] {
  const e = document.createElementNS(NS, name);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  parent?.appendChild(e);
  return e;
}

/** Break text into lines that fit `width`, by words. Explicit line breaks are kept. */
export function wrapText(text: string, width: number, charWidth = 7.2): string[] {
  const max = Math.max(4, Math.floor(width / charWidth));
  const out: string[] = [];
  for (const para of text.split("\n")) {
    let line = "";
    for (const word of para.split(/\s+/).filter(Boolean)) {
      if (line && (line + " " + word).length > max) {
        out.push(line);
        line = word;
      } else line = line ? `${line} ${word}` : word;
    }
    out.push(line);
  }
  return out.slice(0, 12);
}

function shapeFor(n: DiagramNode, g: Element) {
  const { x, y, w, h } = n;
  const common = { class: "shape" };
  switch (n.shape) {
    case "round":
      return el("rect", { ...common, x, y, width: w, height: h, rx: Math.min(h / 2, 28) }, g);
    case "diamond":
      return el("polygon", { ...common, points: `${x + w / 2},${y} ${x + w},${y + h / 2} ${x + w / 2},${y + h} ${x},${y + h / 2}` }, g);
    case "ellipse":
      return el("ellipse", { ...common, cx: x + w / 2, cy: y + h / 2, rx: w / 2, ry: h / 2 }, g);
    case "note":
      return el("path", { ...common, d: `M${x},${y} H${x + w - 14} L${x + w},${y + 14} V${y + h} H${x} Z M${x + w - 14},${y} V${y + 14} H${x + w}` }, g);
    case "cylinder": {
      const r = Math.min(10, h / 5);
      return el("path", { ...common, d: `M${x},${y + r} A${w / 2},${r} 0 0 1 ${x + w},${y + r} V${y + h - r} A${w / 2},${r} 0 0 1 ${x},${y + h - r} Z M${x},${y + r} A${w / 2},${r} 0 0 0 ${x + w},${y + r}` }, g);
    }
    case "text":
      return el("rect", { class: "shape bare", x, y, width: w, height: h }, g);
    default:
      return el("rect", { ...common, x, y, width: w, height: h, rx: 5 }, g);
  }
}

let seq = 0;

/** Draw a diagram as an SVG element. Colors come from the page's style sheet, so it follows the theme. */
export function diagramSvg(d: Diagram, title = "Diagram"): SVGSVGElement {
  const pad = 16;
  const minX = Math.min(...d.nodes.map((n) => n.x), 0) - pad, minY = Math.min(...d.nodes.map((n) => n.y), 0) - pad;
  const maxX = Math.max(...d.nodes.map((n) => n.x + n.w), 100) + pad, maxY = Math.max(...d.nodes.map((n) => n.y + n.h), 60) + pad;
  const id = `dg${++seq}`;
  const svg = el("svg", { class: "diagram", viewBox: `${minX} ${minY} ${maxX - minX} ${maxY - minY}`, width: maxX - minX, role: "img", "aria-label": `${title}: ${d.nodes.length} shapes, ${d.edges.length} connections` });
  const defs = el("defs", {}, svg);
  for (const kind of ["end", "start"]) {
    const m = el("marker", { id: `${id}-${kind}`, viewBox: "0 0 10 10", refX: kind === "end" ? 9 : 1, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" }, defs);
    el("path", { d: kind === "end" ? "M0,0 L10,5 L0,10 Z" : "M10,0 L0,5 L10,10 Z", class: "arrowhead" }, m);
  }
  const byId = new Map(d.nodes.map((n) => [n.id, n]));
  for (const e of d.edges) {
    const a = byId.get(e.from)!, b = byId.get(e.to)!;
    const p1 = borderPoint(a, b.x + b.w / 2, b.y + b.h / 2), p2 = borderPoint(b, a.x + a.w / 2, a.y + a.h / 2);
    const attrs: Record<string, string | number> = { class: `edge ${e.style === "dashed" ? "dashed" : ""}`, x1: p1.x, y1: p1.y, x2: p2.x, y2: p2.y };
    if (e.arrow !== "none") attrs["marker-end"] = `url(#${id}-end)`;
    if (e.arrow === "both") attrs["marker-start"] = `url(#${id}-start)`;
    el("line", attrs, svg);
    if (e.label) {
      const mx = (p1.x + p2.x) / 2, my = (p1.y + p2.y) / 2;
      const width = e.label.length * 6.6 + 10;
      el("rect", { class: "edge-label-bg", x: mx - width / 2, y: my - 9, width, height: 18, rx: 4 }, svg);
      el("text", { class: "edge-label", x: mx, y: my + 4, "text-anchor": "middle" }, svg).textContent = e.label;
    }
  }
  for (const n of d.nodes) {
    const g = el("g", { class: `node ${n.color ?? "neutral"} ${n.shape}` }, svg);
    shapeFor(n, g);
    const inset = n.shape === "diamond" ? n.w * 0.28 : 10;
    const lines = wrapText(n.text, n.w - inset * 2);
    const top = n.y + n.h / 2 - ((lines.length - 1) * 16) / 2 + 4.5;
    const text = el("text", { x: n.x + n.w / 2, "text-anchor": "middle" }, g);
    lines.forEach((line, i) => (el("tspan", { x: n.x + n.w / 2, y: top + i * 16 }, text).textContent = line));
  }
  return svg;
}
