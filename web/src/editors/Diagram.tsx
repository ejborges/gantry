// The diagram editor: flowcharts and block diagrams drawn by hand.
// Shapes are dragged, resized and connected; double-click a shape to change its text.
// The result is the JSON that lib/diagram.ts draws wherever the page is read.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background, BaseEdge, ConnectionMode, Controls, EdgeLabelRenderer, Handle, MarkerType, NodeResizer, Position, ReactFlow, ReactFlowProvider, addEdge, useEdgesState, useInternalNode, useNodesState, useReactFlow,
  type Connection, type Edge, type EdgeProps, type Node, type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ArrowRight, Trash2 } from "lucide-react";
import { Dialog, Field } from "../ui";
import { DIAGRAM_COLORS, EMPTY_DIAGRAM, SHAPES, borderPoint, wrapText, type Diagram, type DiagramColor, type DiagramEdge, type DiagramNode, type Shape } from "../lib/diagram";
import { reducedMotion } from "../lib/format";

type ShapeData = { text: string; shape: Shape; color: DiagramColor; onText?: (id: string, text: string) => void };
type ShapeNode = Node<ShapeData, "shape">;
type LinkData = { dashed?: boolean; arrow?: "end" | "both" | "none" };
type LinkEdge = Edge<LinkData, "link">;

function outline(shape: Shape, w: number, h: number): React.ReactNode {
  switch (shape) {
    case "round":
      return <rect className="shape" x={1} y={1} width={w - 2} height={h - 2} rx={Math.min(h / 2, 28)} />;
    case "diamond":
      return <polygon className="shape" points={`${w / 2},1 ${w - 1},${h / 2} ${w / 2},${h - 1} 1,${h / 2}`} />;
    case "ellipse":
      return <ellipse className="shape" cx={w / 2} cy={h / 2} rx={w / 2 - 1} ry={h / 2 - 1} />;
    case "note":
      return <path className="shape" d={`M1,1 H${w - 15} L${w - 1},15 V${h - 1} H1 Z M${w - 15},1 V15 H${w - 1}`} />;
    case "cylinder": {
      const r = Math.min(10, h / 5);
      return <path className="shape" d={`M1,${r} A${w / 2 - 1},${r} 0 0 1 ${w - 1},${r} V${h - r} A${w / 2 - 1},${r} 0 0 1 1,${h - r} Z M1,${r} A${w / 2 - 1},${r} 0 0 0 ${w - 1},${r}`} />;
    }
    case "text":
      return <rect className="shape bare" x={1} y={1} width={w - 2} height={h - 2} />;
    default:
      return <rect className="shape" x={1} y={1} width={w - 2} height={h - 2} rx={5} />;
  }
}

function ShapeView({ id, data, selected, width, height }: NodeProps<ShapeNode>) {
  const [editing, setEditing] = useState(false);
  const w = width ?? 150, h = height ?? 56;
  const lines = wrapText(data.text, w - (data.shape === "diamond" ? w * 0.56 : 20));
  return (
    <div className={`dg-node ${data.color} ${data.shape}`} style={{ width: w, height: h }} onDoubleClick={() => setEditing(true)}>
      <NodeResizer isVisible={selected} minWidth={40} minHeight={24} />
      <svg width={w} height={h} className="diagram" aria-hidden="true">
        <g className={`node ${data.color}`}>
          {outline(data.shape, w, h)}
          {!editing && (
            <text textAnchor="middle">
              {lines.map((line, i) => (
                <tspan key={i} x={w / 2} y={h / 2 - ((lines.length - 1) * 16) / 2 + 4.5 + i * 16}>
                  {line}
                </tspan>
              ))}
            </text>
          )}
        </g>
      </svg>
      {editing && (
        <textarea
          className="dg-text nodrag"
          autoFocus
          defaultValue={data.text}
          aria-label="Text in this shape"
          maxLength={400}
          onBlur={(e) => (data.onText?.(id, e.target.value), setEditing(false))}
          onKeyDown={(e) => {
            if (e.key === "Escape" || (e.key === "Enter" && !e.shiftKey)) (e.target as HTMLTextAreaElement).blur();
            e.stopPropagation();
          }}
        />
      )}
      {[Position.Top, Position.Right, Position.Bottom, Position.Left].map((p) => (
        <Handle key={p} id={p} type="source" position={p} aria-label={`Connect from the ${p} side`} />
      ))}
    </div>
  );
}

/** A straight line between the outlines of two shapes, as the page will draw it. */
function LinkView({ id, source, target, label, data, selected, markerEnd, markerStart }: EdgeProps<LinkEdge>) {
  const a = useInternalNode<ShapeNode>(source), b = useInternalNode<ShapeNode>(target);
  if (!a || !b) return null;
  const box = (n: NonNullable<typeof a>): DiagramNode => ({ id: n.id, x: n.internals.positionAbsolute.x, y: n.internals.positionAbsolute.y, w: n.measured.width ?? 150, h: n.measured.height ?? 56, shape: n.data.shape, text: "" });
  const na = box(a), nb = box(b);
  const p1 = borderPoint(na, nb.x + nb.w / 2, nb.y + nb.h / 2), p2 = borderPoint(nb, na.x + na.w / 2, na.y + na.h / 2);
  return (
    <>
      <BaseEdge id={id} path={`M${p1.x},${p1.y} L${p2.x},${p2.y}`} markerEnd={markerEnd} markerStart={markerStart} className={`dg-edge ${data?.dashed ? "dashed" : ""} ${selected ? "selected" : ""}`} />
      {label && (
        <EdgeLabelRenderer>
          <span className="dg-edge-label" style={{ transform: `translate(-50%, -50%) translate(${(p1.x + p2.x) / 2}px, ${(p1.y + p2.y) / 2}px)` }}>
            {label}
          </span>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

const nodeTypes = { shape: ShapeView };
const edgeTypes = { link: LinkView };
const arrow = { type: MarkerType.ArrowClosed, width: 18, height: 18 };

function toFlow(d: Diagram): { nodes: ShapeNode[]; edges: LinkEdge[] } {
  return {
    nodes: d.nodes.map((n) => ({ id: n.id, type: "shape", position: { x: n.x, y: n.y }, width: n.w, height: n.h, data: { text: n.text, shape: n.shape, color: n.color ?? "neutral" } })),
    edges: d.edges.map((e) => linkFrom(e)),
  };
}
function linkFrom(e: DiagramEdge): LinkEdge {
  return { id: e.id, type: "link", source: e.from, target: e.to, label: e.label || undefined, data: { dashed: e.style === "dashed", arrow: e.arrow ?? "end" }, markerEnd: e.arrow === "none" ? undefined : arrow, markerStart: e.arrow === "both" ? arrow : undefined };
}
function fromFlow(nodes: ShapeNode[], edges: LinkEdge[]): Diagram {
  return {
    nodes: nodes.map((n) => ({ id: n.id, x: n.position.x, y: n.position.y, w: n.width ?? n.measured?.width ?? 150, h: n.height ?? n.measured?.height ?? 56, shape: n.data.shape, text: n.data.text, color: n.data.color })),
    edges: edges.map((e) => ({ id: e.id, from: e.source, to: e.target, label: typeof e.label === "string" ? e.label : "", style: e.data?.dashed ? "dashed" : "solid", arrow: e.data?.arrow ?? "end" })),
  };
}

const uid = (prefix: string) => `${prefix}${Math.random().toString(36).slice(2, 8)}`;

function Canvas({ initial, onChange }: { initial: Diagram; onChange: (d: Diagram) => void }) {
  const start = useMemo(() => toFlow(initial), []); // eslint-disable-line react-hooks/exhaustive-deps
  const [nodes, setNodes, onNodesChange] = useNodesState<ShapeNode>(start.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<LinkEdge>(start.edges);
  const flow = useReactFlow();
  const report = useRef(onChange);
  report.current = onChange;
  // A block body on purpose: whatever the caller's function returns must not be taken for a cleanup.
  useEffect(() => {
    report.current(fromFlow(nodes, edges));
  }, [nodes, edges]);

  const setText = useCallback((id: string, text: string) => setNodes((ns) => ns.map((n) => (n.id === id ? { ...n, data: { ...n.data, text } } : n))), [setNodes]);
  const withHandlers = useMemo(() => nodes.map((n) => ({ ...n, data: { ...n.data, onText: setText } })), [nodes, setText]);
  const selectedNode = nodes.find((n) => n.selected);
  const selectedEdge = edges.find((e) => e.selected);
  const patchNode = (data: Partial<ShapeData>) => selectedNode && setNodes((ns) => ns.map((n) => (n.id === selectedNode.id ? { ...n, data: { ...n.data, ...data } } : n)));
  const patchEdge = (change: (e: LinkEdge) => LinkEdge) => selectedEdge && setEdges((es) => es.map((e) => (e.id === selectedEdge.id ? change(e) : e)));

  function add(shape: Shape) {
    const size = shape === "diamond" ? { w: 150, h: 80 } : shape === "text" ? { w: 150, h: 36 } : { w: 150, h: 56 };
    const id = uid("n");
    setNodes((ns) => {
      const box = (n: ShapeNode) => ({ x: n.position.x, y: n.position.y, w: n.width ?? 150, h: n.height ?? 56 });
      const taken = ns.map(box);
      const from = ns.find((n) => n.selected);
      // Beside the selected shape when there is one (the usual next step in a flow), otherwise
      // under everything; then moved down until it covers nothing.
      let x: number, y: number;
      if (from) {
        const r = box(from);
        x = r.x + r.w + 70;
        y = r.y + (r.h - size.h) / 2;
      } else if (taken.length) {
        x = Math.min(...taken.map((r) => r.x));
        y = Math.max(...taken.map((r) => r.y + r.h)) + 40;
      } else {
        const center = flow.screenToFlowPosition({ x: window.innerWidth / 2, y: window.innerHeight / 2 });
        x = center.x - size.w / 2;
        y = center.y - size.h / 2;
      }
      const covers = () => taken.some((r) => x < r.x + r.w + 16 && r.x < x + size.w + 16 && y < r.y + r.h + 16 && r.y < y + size.h + 16);
      for (let tries = 0; tries < 80 && covers(); tries++) y += size.h + 30;
      return [...ns.map((n) => ({ ...n, selected: false })), { id, type: "shape", position: { x: Math.round(x), y: Math.round(y) }, width: size.w, height: size.h, selected: true, data: { text: SHAPES.find((s) => s.key === shape)?.label ?? "", shape, color: "neutral" } }];
    });
    // The new shape may be outside what is on screen: bring the whole drawing into view.
    requestAnimationFrame(() => void flow.fitView({ padding: 0.2, maxZoom: 1, duration: reducedMotion() ? 0 : 200 }));
  }
  const onConnect = useCallback((c: Connection) => c.source !== c.target && setEdges((es) => addEdge({ ...c, id: uid("e"), type: "link", sourceHandle: null, targetHandle: null, data: { arrow: "end" }, markerEnd: arrow }, es) as LinkEdge[]), [setEdges]);

  return (
    <div className="dg">
      <div className="toolbar" role="toolbar" aria-label="Add a shape">
        {SHAPES.map((s) => (
          <button key={s.key} type="button" className="btn ghost sm" onClick={() => add(s.key)}>
            <svg width="22" height="16" viewBox="0 0 22 16" className="diagram dg-icon" aria-hidden="true">
              <g className="node neutral">{outline(s.key, 22, 16)}</g>
            </svg>
            {s.label}
          </button>
        ))}
      </div>
      <div className="dg-body">
        <div className="dg-canvas">
          <ReactFlow
            nodes={withHandlers}
            edges={edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            connectionMode={ConnectionMode.Loose}
            snapToGrid
            snapGrid={[10, 10]}
            fitView
            fitViewOptions={{ padding: 0.25, maxZoom: 1.2 }}
            deleteKeyCode={["Backspace", "Delete"]}
            proOptions={{ hideAttribution: true }}
            aria-label="Diagram canvas. Tab moves between shapes, arrow keys move the selected shape."
          >
            <Background gap={20} />
            <Controls showInteractive={false} />
          </ReactFlow>
        </div>
        <aside className="dg-side" aria-label="Selected shape or connection">
          {selectedNode ? (
            <div className="stack">
              <Field label="Text">
                <textarea className="textarea" rows={3} maxLength={400} value={selectedNode.data.text} onChange={(e) => patchNode({ text: e.target.value })} />
              </Field>
              <Field label="Shape">
                <select className="select" value={selectedNode.data.shape} onChange={(e) => patchNode({ shape: e.target.value as Shape })}>
                  {SHAPES.map((s) => (
                    <option key={s.key} value={s.key}>{s.label}</option>
                  ))}
                </select>
              </Field>
              <Field label="Color">
                <div className="row wrap" role="radiogroup">
                  {DIAGRAM_COLORS.map((c) => (
                    <button key={c} type="button" role="radio" aria-checked={selectedNode.data.color === c} aria-label={c} title={c} className={`dg-swatch ${c}`} onClick={() => patchNode({ color: c })} />
                  ))}
                </div>
              </Field>
              <button type="button" className="btn danger sm" onClick={() => flow.deleteElements({ nodes: [{ id: selectedNode.id }] })}>
                <Trash2 /> Remove shape
              </button>
            </div>
          ) : selectedEdge ? (
            <div className="stack">
              <Field label="Label">
                <input className="input" maxLength={120} value={typeof selectedEdge.label === "string" ? selectedEdge.label : ""} onChange={(e) => patchEdge((x) => ({ ...x, label: e.target.value || undefined }))} />
              </Field>
              <Field label="Arrowheads">
                <select className="select" value={selectedEdge.data?.arrow ?? "end"} onChange={(e) => patchEdge((x) => linkFrom({ ...fromFlow([], [x]).edges[0], arrow: e.target.value as DiagramEdge["arrow"] }))}>
                  <option value="end">At the end</option>
                  <option value="both">At both ends</option>
                  <option value="none">None</option>
                </select>
              </Field>
              <label className="check">
                <input type="checkbox" checked={!!selectedEdge.data?.dashed} onChange={(e) => patchEdge((x) => ({ ...x, data: { ...x.data, dashed: e.target.checked } }))} /> Dashed line
              </label>
              <button type="button" className="btn danger sm" onClick={() => flow.deleteElements({ edges: [{ id: selectedEdge.id }] })}>
                <Trash2 /> Remove connection
              </button>
            </div>
          ) : (
            <div className="stack tight small muted">
              <p>Add shapes from the bar above and drag them into place.</p>
              <p>
                Drag from a dot on a shape's edge to another shape to connect them <ArrowRight width={12} height={12} aria-hidden="true" />
              </p>
              <p>Double-click a shape to type in it. Select a shape or a line to change or remove it.</p>
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}

/** The editor in a dialog. `onSave` receives the diagram when the person chooses to keep it. */
export default function DiagramDialog({ value, onSave, onClose }: { value?: Diagram | null; onSave: (d: Diagram) => void; onClose: () => void }) {
  const current = useRef<Diagram>(value ?? EMPTY_DIAGRAM);
  return (
    <Dialog
      title={value ? "Edit diagram" : "Draw a diagram"}
      wide="canvas"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="btn primary" onClick={() => onSave(current.current)}>
            {value ? "Update diagram" : "Insert diagram"}
          </button>
        </>
      }
    >
      <ReactFlowProvider>
        <Canvas initial={value ?? EMPTY_DIAGRAM} onChange={(d) => (current.current = d)} />
      </ReactFlowProvider>
    </Dialog>
  );
}
