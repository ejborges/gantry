// The visual workflow editor: stages are boxes, the moves work may make between them are
// arrows. Drag from one stage to another to allow a move; branches are simply several arrows
// leaving one stage. Everything on the canvas is also in the two lists beneath it, so the
// workflow can be built with the keyboard alone and read by a screen reader.
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Background, BaseEdge, ConnectionMode, Controls, EdgeLabelRenderer, Handle, MarkerType, Position, ReactFlow, ReactFlowProvider, getBezierPath, getSmoothStepPath, useEdgesState, useNodesState,
  type Connection, type Edge, type EdgeProps, type Node, type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Flag, LayoutGrid, Plus, Trash2 } from "lucide-react";
import type { FieldDef, Status } from "../api";
import { Field } from "../ui";
import { ANY, CATEGORIES, NODE_H, NODE_W, arrange, deadEnds, fromMoves, keyFor, problems, toMoves, unreachable, type WorkflowDef } from "../lib/workflow";

type StageData = { name: string; category: Status["category"]; initial: boolean; any?: boolean };
type StageNode = Node<StageData, "stage">;
type MoveData = { name: string; requires: string[] };
type MoveEdge = Edge<MoveData, "move">;

function StageView({ data, selected }: NodeProps<StageNode>) {
  return (
    <div className={`wf-node ${data.any ? "any" : data.category} ${selected ? "selected" : ""}`} style={{ width: NODE_W, minHeight: NODE_H }}>
      {data.initial && (
        <span className="wf-start" title="New work starts here">
          <Flag aria-hidden="true" /> Start
        </span>
      )}
      <strong className="truncate">{data.name || "Unnamed"}</strong>
      <span className="small">{data.any ? "Moves from here are always allowed" : CATEGORIES.find((c) => c.key === data.category)?.label}</span>
      {!data.any && <Handle id="in" type="target" position={Position.Left} aria-label="Moves arrive here" />}
      {!data.any && <Handle id="any" type="target" position={Position.Top} className="wf-any-in" isConnectable={false} aria-hidden="true" />}
      <Handle id="out" type="source" position={data.any ? Position.Bottom : Position.Right} aria-label="Drag from here to another box to allow a move" />
    </div>
  );
}

function MoveView({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data, selected, markerEnd }: EdgeProps<MoveEdge>) {
  // A move back to an earlier stage goes around beneath the row, so it does not cover the
  // forward moves; it is drawn dashed to tell the two apart at a glance.
  const back = sourcePosition === Position.Right && targetX < sourceX;
  const [path, lx, ly] = back
    ? getSmoothStepPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, borderRadius: 12, offset: 26 })
    : getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, curvature: 0.25 });
  return (
    <>
      <BaseEdge id={id} path={path} markerEnd={markerEnd} className={`dg-edge ${selected ? "selected" : ""} ${back ? "dashed" : ""}`} />
      <EdgeLabelRenderer>
        <span className={`dg-edge-label ${selected ? "selected" : ""}`} style={{ transform: `translate(-50%, -50%) translate(${lx}px, ${ly}px)` }}>
          {data?.name}
          {data?.requires.length ? " *" : ""}
        </span>
      </EdgeLabelRenderer>
    </>
  );
}

const nodeTypes = { stage: StageView };
const edgeTypes = { move: MoveView };
const arrow = { type: MarkerType.ArrowClosed, width: 18, height: 18 };
const uid = () => `m${Math.random().toString(36).slice(2, 9)}`;

const edgeFor = (id: string, from: string, to: string, data: MoveData, selected = false): MoveEdge => ({ id, type: "move", source: from, target: to, sourceHandle: "out", targetHandle: from === ANY ? "any" : "in", markerEnd: arrow, selected, data });

function toFlow(def: WorkflowDef, noun: string): { nodes: StageNode[]; edges: MoveEdge[] } {
  const moves = toMoves(def.transitions);
  const at = arrange(def.statuses, moves, def.layout);
  const places = Object.values(at);
  const top = Math.min(0, ...places.map((p) => p.y));
  const left = Math.min(0, ...places.map((p) => p.x)), right = Math.max(0, ...places.map((p) => p.x));
  const nodes: StageNode[] = def.statuses.map((s) => ({ id: s.key, type: "stage", position: at[s.key], data: { name: s.name, category: s.category, initial: s.key === def.initial } }));
  // "Any" sits above the middle of the row, and its moves come down onto the boxes.
  nodes.push({ id: ANY, type: "stage", position: { x: Math.round((left + right) / 2), y: top - 150 }, deletable: false, data: { name: `Any ${noun}`, category: "todo", initial: false, any: true } });
  return { nodes, edges: moves.map((m) => edgeFor(m.id, m.from, m.to, { name: m.name, requires: m.requires })) };
}

function fromFlow(nodes: StageNode[], edges: MoveEdge[]): WorkflowDef {
  const stages = nodes.filter((n) => !n.data.any);
  const keys = new Set(stages.map((n) => n.id));
  return {
    initial: stages.find((n) => n.data.initial)?.id ?? "",
    statuses: stages.map((n) => ({ key: n.id, name: n.data.name.trim(), category: n.data.category })),
    transitions: fromMoves(edges.filter((e) => keys.has(e.target) && (e.source === ANY || keys.has(e.source))).map((e) => ({ id: e.id, name: (e.data?.name ?? "").trim(), from: e.source, to: e.target, requires: e.data?.requires ?? [] }))),
    layout: Object.fromEntries(stages.map((n) => [n.id, { x: Math.round(n.position.x), y: Math.round(n.position.y) }])),
  };
}

interface Props {
  value: WorkflowDef;
  onChange: (def: WorkflowDef) => void;
  /** Fields a move may require to be filled in. Leave out where moves cannot require any. */
  fields?: FieldDef[];
  /** What a stage is called here: "status" for work, "stage" for pages. */
  noun?: string;
  readOnly?: boolean;
}

function Editor({ value, onChange, fields, noun = "status", readOnly }: Props) {
  const start = useMemo(() => toFlow(value, noun), []); // eslint-disable-line react-hooks/exhaustive-deps
  const [nodes, setNodes, onNodesChange] = useNodesState<StageNode>(start.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<MoveEdge>(start.edges);
  const [newName, setNewName] = useState("");
  const [newMove, setNewMove] = useState({ from: "", to: "" });
  const first = useRef(true);
  const def = useMemo(() => fromFlow(nodes, edges), [nodes, edges]);
  const serial = JSON.stringify(def);
  useEffect(() => {
    if (first.current) first.current = false;
    else onChange(def);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serial]);

  const stages = nodes.filter((n) => !n.data.any);
  const plural = noun === "status" ? "statuses" : `${noun}s`;
  const nameOf = (id: string) => (id === ANY ? `Any ${noun}` : (nodes.find((n) => n.id === id)?.data.name ?? id));
  const selectedNode = stages.find((n) => n.selected);
  const selectedEdge = edges.find((e) => e.selected);
  const patchNode = (id: string, data: Partial<StageData>) => setNodes((ns) => ns.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...data } } : data.initial ? { ...n, data: { ...n.data, initial: false } } : n)));
  const patchEdge = (id: string, data: Partial<MoveData>) => setEdges((es) => es.map((e) => (e.id === id ? { ...e, data: { ...e.data!, ...data } } : e)));
  const select = (kind: "node" | "edge", id: string) => {
    setNodes((ns) => ns.map((n) => ({ ...n, selected: kind === "node" && n.id === id })));
    setEdges((es) => es.map((e) => ({ ...e, selected: kind === "edge" && e.id === id })));
  };
  function addMove(from: string, to: string) {
    if (!from || !to || from === to || to === ANY) return;
    if (edges.some((e) => e.source === from && e.target === to)) return;
    const id = uid();
    setEdges((es) => [...es.map((e) => ({ ...e, selected: false })), edgeFor(id, from, to, { name: nameOf(to), requires: [] }, true)]);
    setNodes((ns) => ns.map((n) => ({ ...n, selected: false })));
  }
  function addStage() {
    const name = newName.trim();
    if (!name) return;
    const key = keyFor(name, nodes.map((n) => n.id));
    const right = Math.max(0, ...stages.map((n) => n.position.x)) + NODE_W + 70;
    const y = stages.length ? stages[stages.length - 1].position.y : 40;
    setNodes((ns) => [...ns.map((n) => ({ ...n, selected: false })), { id: key, type: "stage", position: { x: right, y }, selected: true, data: { name, category: stages.length === 0 ? "todo" : "in_progress", initial: stages.length === 0 } }]);
    setNewName("");
  }
  function removeStage(id: string) {
    setEdges((es) => es.filter((e) => e.source !== id && e.target !== id));
    setNodes((ns) => ns.filter((n) => n.id !== id));
  }
  function tidy() {
    const at = arrange(def.statuses, toMoves(def.transitions), null);
    setNodes((ns) => ns.map((n) => (at[n.id] ? { ...n, position: at[n.id] } : n)));
  }
  const issues = problems(def);
  const orphans = issues.length ? [] : unreachable(def);
  const stuck = issues.length ? [] : deadEnds(def);

  return (
    <div className="wf">
      <div className="toolbar wf-bar">
        {!readOnly && (
          <form className="row" onSubmit={(e) => (e.preventDefault(), addStage())}>
            <input className="input" style={{ maxWidth: 220 }} value={newName} maxLength={60} onChange={(e) => setNewName(e.target.value)} placeholder={`Name of a new ${noun}`} aria-label={`Name of a new ${noun}`} />
            <button className="btn sm" disabled={!newName.trim() || stages.length >= 60}>
              <Plus /> Add {noun}
            </button>
          </form>
        )}
        <span className="grow" />
        {!readOnly && (
          <button type="button" className="btn sm" onClick={tidy} title="Arrange the boxes in the order work flows">
            <LayoutGrid /> Tidy up
          </button>
        )}
      </div>
      <div className="dg-body">
        <div className="dg-canvas">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={(c: Connection) => addMove(c.source, c.target)}
            onNodesDelete={(gone) => setEdges((es) => es.filter((e) => !gone.some((g) => g.id === e.source || g.id === e.target)))}
            isValidConnection={(c) => c.source !== c.target && c.target !== ANY}
            connectionMode={ConnectionMode.Strict}
            nodesDraggable={!readOnly}
            nodesConnectable={!readOnly}
            elementsSelectable
            snapToGrid
            snapGrid={[10, 10]}
            fitView
            fitViewOptions={{ padding: 0.15, maxZoom: 1 }}
            minZoom={0.3}
            // The editor usually opens inside a dialog that is still settling into its size;
            // fit once more when it has.
            onInit={(flow) => void setTimeout(() => flow.fitView({ padding: 0.15, maxZoom: 1 }), 120)}
            deleteKeyCode={readOnly ? null : ["Backspace", "Delete"]}
            proOptions={{ hideAttribution: true }}
            aria-label={`Workflow canvas. Each box is a ${noun}; each arrow is an allowed move. The same workflow is listed beneath.`}
          >
            <Background gap={20} />
            <Controls showInteractive={false} />
          </ReactFlow>
        </div>
        <aside className="dg-side" aria-label={`Selected ${noun} or move`}>
          {selectedNode ? (
            <div className="stack">
              <Field label="Name">
                <input className="input" value={selectedNode.data.name} maxLength={60} disabled={readOnly} onChange={(e) => patchNode(selectedNode.id, { name: e.target.value })} />
              </Field>
              <Field label="Counts as" hint={CATEGORIES.find((c) => c.key === selectedNode.data.category)?.hint}>
                <select className="select" value={selectedNode.data.category} disabled={readOnly} onChange={(e) => patchNode(selectedNode.id, { category: e.target.value as Status["category"] })}>
                  {CATEGORIES.map((c) => (
                    <option key={c.key} value={c.key}>{c.label}</option>
                  ))}
                </select>
              </Field>
              <label className="check">
                <input type="checkbox" checked={selectedNode.data.initial} disabled={readOnly || selectedNode.data.initial} onChange={() => patchNode(selectedNode.id, { initial: true })} /> New work starts here
              </label>
              <p className="small muted">
                Key: <code>{selectedNode.id}</code>
              </p>
              {!readOnly && (
                <button type="button" className="btn sm danger" onClick={() => removeStage(selectedNode.id)}>
                  <Trash2 /> Remove this {noun}
                </button>
              )}
            </div>
          ) : selectedEdge ? (
            <div className="stack">
              <p className="small muted" style={{ margin: 0 }}>
                From <strong>{nameOf(selectedEdge.source)}</strong> to <strong>{nameOf(selectedEdge.target)}</strong>
              </p>
              <Field label="Name of the move" hint="Shown on the button people press">
                <input className="input" value={selectedEdge.data?.name ?? ""} maxLength={60} disabled={readOnly} onChange={(e) => patchEdge(selectedEdge.id, { name: e.target.value })} />
              </Field>
              {fields && fields.length > 0 && (
                <fieldset className="stack tight">
                  <legend className="label">Must be filled in first</legend>
                  {fields.map((f) => (
                    <label key={f.key} className="check">
                      <input
                        type="checkbox"
                        disabled={readOnly}
                        checked={selectedEdge.data?.requires.includes(f.key) ?? false}
                        onChange={(e) => patchEdge(selectedEdge.id, { requires: e.target.checked ? [...(selectedEdge.data?.requires ?? []), f.key] : (selectedEdge.data?.requires ?? []).filter((k) => k !== f.key) })}
                      />{" "}
                      {f.name}
                    </label>
                  ))}
                </fieldset>
              )}
              {!readOnly && (
                <button type="button" className="btn sm danger" onClick={() => setEdges((es) => es.filter((e) => e.id !== selectedEdge.id))}>
                  <Trash2 /> Remove this move
                </button>
              )}
            </div>
          ) : (
            <div className="small muted stack">
              <p style={{ margin: 0 }}>Select a box or an arrow to change it.</p>
              {!readOnly && (
                <>
                  <p style={{ margin: 0 }}>Drag from the dot on the right of a box to another box to allow that move. Several arrows leaving one box are a branch.</p>
                  <p style={{ margin: 0 }}>Drag from <strong>Any {noun}</strong> for a move that is always allowed, such as canceling.</p>
                </>
              )}
            </div>
          )}
        </aside>
      </div>
      {issues.length > 0 && (
        <div className="notice danger" role="alert">
          {issues.join(" ")}
        </div>
      )}
      {orphans.length > 0 && <div className="notice warn">Nothing leads to {orphans.map((s) => s.name).join(", ")}: work can never get there.</div>}
      {stuck.length > 0 && <div className="notice warn">Nothing leads out of {stuck.map((s) => s.name).join(", ")}, and {stuck.length === 1 ? "it does" : "they do"} not count as done: work would stay there.</div>}
      <details className="wf-lists" open={readOnly}>
        <summary>
          The same workflow as lists: {stages.length} {stages.length === 1 ? noun : plural}, {edges.length} {edges.length === 1 ? "move" : "moves"}
        </summary>
        <div className="table-wrap">
          <table className="table">
            <caption className="sr-only">{noun === "stage" ? "Stages" : "Statuses"}</caption>
            <thead>
              <tr>
                <th scope="col">Name</th>
                <th scope="col">Counts as</th>
                <th scope="col">Start</th>
                <th scope="col"><span className="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody>
              {stages.map((n) => (
                <tr key={n.id}>
                  <td>
                    <input className="input" value={n.data.name} maxLength={60} disabled={readOnly} aria-label={`Name of ${n.data.name}`} onChange={(e) => patchNode(n.id, { name: e.target.value })} onFocus={() => select("node", n.id)} />
                  </td>
                  <td>
                    <select className="select" value={n.data.category} disabled={readOnly} aria-label={`What ${n.data.name} counts as`} onChange={(e) => patchNode(n.id, { category: e.target.value as Status["category"] })}>
                      {CATEGORIES.map((c) => (
                        <option key={c.key} value={c.key}>{c.label}</option>
                      ))}
                    </select>
                  </td>
                  <td>
                    <input type="radio" name="wf-initial" checked={n.data.initial} disabled={readOnly} aria-label={`New work starts in ${n.data.name}`} onChange={() => patchNode(n.id, { initial: true })} />
                  </td>
                  <td className="right">
                    {!readOnly && (
                      <button type="button" className="btn ghost icon sm" aria-label={`Remove ${n.data.name}`} onClick={() => removeStage(n.id)}>
                        <Trash2 />
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="table-wrap">
          <table className="table">
            <caption className="sr-only">Moves</caption>
            <thead>
              <tr>
                <th scope="col">From</th>
                <th scope="col">To</th>
                <th scope="col">Name of the move</th>
                <th scope="col"><span className="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody>
              {edges.map((e) => (
                <tr key={e.id}>
                  <td>{nameOf(e.source)}</td>
                  <td>{nameOf(e.target)}</td>
                  <td>
                    <input className="input" value={e.data?.name ?? ""} maxLength={60} disabled={readOnly} aria-label={`Name of the move from ${nameOf(e.source)} to ${nameOf(e.target)}`} onChange={(ev) => patchEdge(e.id, { name: ev.target.value })} onFocus={() => select("edge", e.id)} />
                    {e.data?.requires.length ? <span className="small muted"> Requires {e.data.requires.map((k) => fields?.find((f) => f.key === k)?.name ?? k).join(", ")}</span> : null}
                  </td>
                  <td className="right">
                    {!readOnly && (
                      <button type="button" className="btn ghost icon sm" aria-label={`Remove the move from ${nameOf(e.source)} to ${nameOf(e.target)}`} onClick={() => setEdges((es) => es.filter((x) => x.id !== e.id))}>
                        <Trash2 />
                      </button>
                    )}
                  </td>
                </tr>
              ))}
              {!readOnly && (
                <tr>
                  <td>
                    <select className="select" value={newMove.from} aria-label="A new move from" onChange={(e) => setNewMove({ ...newMove, from: e.target.value })}>
                      <option value="">From…</option>
                      <option value={ANY}>Any {noun}</option>
                      {stages.map((n) => (
                        <option key={n.id} value={n.id}>{n.data.name}</option>
                      ))}
                    </select>
                  </td>
                  <td>
                    <select className="select" value={newMove.to} aria-label="A new move to" onChange={(e) => setNewMove({ ...newMove, to: e.target.value })}>
                      <option value="">To…</option>
                      {stages.filter((n) => n.id !== newMove.from).map((n) => (
                        <option key={n.id} value={n.id}>{n.data.name}</option>
                      ))}
                    </select>
                  </td>
                  <td colSpan={2}>
                    <button type="button" className="btn sm" disabled={!newMove.from || !newMove.to || edges.some((e) => e.source === newMove.from && e.target === newMove.to)} onClick={() => (addMove(newMove.from, newMove.to), setNewMove({ from: "", to: "" }))}>
                      <Plus /> Allow this move
                    </button>
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </details>
    </div>
  );
}

export default function WorkflowEditor(props: Props) {
  return (
    <ReactFlowProvider>
      <Editor {...props} />
    </ReactFlowProvider>
  );
}
