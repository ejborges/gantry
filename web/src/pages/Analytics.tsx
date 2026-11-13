// Analytics: dashboards made of tiles, the library of visualizations (built-in, your own
// and the ones others share), and the editor where a visualization is written as a script.
// Every script runs in the sandboxed frame and reads data only through the API, as the
// person looking at it: a shared dashboard shows each viewer what that viewer may see.
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Link, NavLink, Navigate, Route, Routes, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { ChartColumn, Code, Copy, GripVertical, LayoutDashboard, Maximize2, Pencil, Play, Plus, Settings, Shapes, SlidersHorizontal, Trash2, Users } from "lucide-react";
import { ApiError, api, refresh, useGet, type Person } from "../api";
import { useSession } from "../session";
import { Confirm, CopyButton, Dialog, Empty, Field, Loaded, Loading, Menu, Peers, PersonChip, Pick, toast, useAction } from "../ui";
import { relative } from "../lib/format";
import { colorFor, useLiveRefresh, usePresence, useRoom, useSharedDoc } from "../realtime";
import { usePeople } from "../work";
import { LIBRARIES } from "../lib/sandbox";
import { COLUMNS, GAP_PX, ROW_PX, changed, clamp, compact, placeNew, settle } from "../lib/grid";
import Sandbox from "../components/Sandbox";

const CodeEditor = lazy(() => import("../editors/Code"));

interface Template {
  key: string;
  name: string;
  description: string;
  library: string;
  params: { key: string; label: string; default: unknown }[];
  size: { w: number; h: number };
  source?: string;
}
interface Viz {
  id: string;
  name: string;
  description: string;
  library: string;
  params: Record<string, unknown>;
  template: string | null;
  visibility: "private" | "workspace";
  version: number;
  updated_at: string;
  owner: Person;
  mine: boolean;
  can_edit: boolean;
  editors: Person[];
  source?: string;
}
interface Library {
  templates: Template[];
  visualizations: Viz[];
  libraries: { key: string; name: string }[];
  can_share: boolean;
}
interface Tile {
  id: string;
  kind: "template" | "visualization";
  ref: string;
  title: string;
  x: number;
  y: number;
  w: number;
  h: number;
  params: Record<string, unknown>;
  content: null | { name: string; library: string; source: string; params: Record<string, unknown> | null; builtin?: boolean; owner?: Person };
}
interface Dashboard {
  id: string;
  name: string;
  description: string;
  params: Record<string, unknown>;
  visibility: "private" | "workspace";
  version: number;
  updated_at: string;
  owner: Person;
  mine: boolean;
  can_edit: boolean;
  tiles: number;
  editors: Person[];
  layout: Tile[];
}

const kindOf = (library: string): "html" | "script" => (library === "html" ? "html" : "script");
const libraryName = (key: string) => LIBRARIES[key]?.name ?? key;
const uid = () => `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

function Tabs() {
  const cls = ({ isActive }: { isActive: boolean }) => (isActive ? "active" : "");
  return (
    <nav className="tabs" aria-label="Analytics" style={{ marginBottom: 16 }}>
      <NavLink to="/analytics" end className={cls}><LayoutDashboard /> Dashboards</NavLink>
      <NavLink to="/analytics/library" className={cls}><Shapes /> Visualizations</NavLink>
    </nav>
  );
}

function Shared({ visibility }: { visibility: string }) {
  return visibility === "workspace" ? <span className="badge outline"><Users /> Shared</span> : null;
}

// ───────────────────────── dashboards ─────────────────────────

function Dashboards() {
  const q = useGet<{ dashboards: Dashboard[]; can_share: boolean }>("/api/v1/dashboards");
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [busy, run] = useAction();
  const nav = useNavigate();
  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Analytics</h1>
          <p className="sub">Dashboards of charts and figures about the work. Pick from the built-in visualizations or write your own.</p>
        </div>
        <button className="btn primary" onClick={() => setCreating(true)}>
          <Plus /> New dashboard
        </button>
      </div>
      <Tabs />
      <Loaded q={q} what="Loading dashboards">
        {(d) =>
          d.dashboards.length === 0 ? (
            <Empty icon={<LayoutDashboard />} title="No dashboards yet">
              <p>Create one and add tiles to it: key figures, work by status, workload, time to resolve, or a chart you write yourself.</p>
            </Empty>
          ) : (
            <div className="card-grid">
              {d.dashboards.map((x) => (
                <Link key={x.id} to={`/analytics/d/${x.id}`} className="card">
                  <span className="row">
                    <LayoutDashboard className="muted" />
                    <strong className="grow truncate">{x.name}</strong>
                    <Shared visibility={x.visibility} />
                  </span>
                  {x.description && <span className="small muted">{x.description}</span>}
                  <span className="small muted">
                    {x.tiles} {x.tiles === 1 ? "tile" : "tiles"} · {x.mine ? "yours" : `by ${x.owner.name}`} · changed {relative(x.updated_at)}
                  </span>
                </Link>
              ))}
            </div>
          )
        }
      </Loaded>
      {creating && (
        <Dialog
          title="New dashboard"
          onClose={() => setCreating(false)}
          footer={
            <>
              <button className="btn" onClick={() => setCreating(false)}>Cancel</button>
              <button className="btn primary" disabled={busy || !name.trim()} onClick={() => run(() => api("POST", "/api/v1/dashboards", { name: name.trim() }), (r: any) => nav(`/analytics/d/${r.id}?edit=1`))}>
                Create
              </button>
            </>
          }
        >
          <Field label="Name" required>
            <input className="input" autoFocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} />
          </Field>
          <p className="small muted" style={{ marginTop: 10 }}>A new dashboard is yours alone until you share it.</p>
        </Dialog>
      )}
    </div>
  );
}

function AddTile({ dashboard, onClose }: { dashboard: Dashboard; onClose: () => void }) {
  const lib = useGet<Library>("/api/v1/visualizations");
  const [busy, run] = useAction();
  const add = (kind: Tile["kind"], ref: string, size: { w: number; h: number }) => {
    const at = placeNew(dashboard.layout, size.w, size.h);
    return run(() => api("PUT", `/api/v1/dashboards/${dashboard.id}/tiles/${uid()}`, { kind, ref, ...at }), "Tile added").then((r) => r !== undefined && onClose());
  };
  return (
    <Dialog title="Add a tile" wide onClose={onClose} footer={<button className="btn" onClick={onClose}>Close</button>}>
      <Loaded q={lib} what="Loading visualizations">
        {(l) => (
          <div className="stack loose">
            {l.visualizations.length > 0 && (
              <section>
                <h3 className="section-title">Saved visualizations</h3>
                <div className="viz-list">
                  {l.visualizations.map((v) => (
                    <button key={v.id} className="card viz-card" disabled={busy} onClick={() => add("visualization", v.id, { w: 6, h: 4 })}>
                      <span className="row">
                        <strong className="grow truncate">{v.name}</strong>
                        <span className="badge outline">{libraryName(v.library)}</span>
                      </span>
                      <span className="small muted">{v.description || (v.mine ? "Yours" : `By ${v.owner.name}`)}</span>
                    </button>
                  ))}
                </div>
              </section>
            )}
            <section>
              <h3 className="section-title">Built-in</h3>
              <div className="viz-list">
                {l.templates.map((t) => (
                  <button key={t.key} className="card viz-card" disabled={busy} onClick={() => add("template", t.key, t.size)}>
                    <span className="row">
                      <strong className="grow truncate">{t.name}</strong>
                      <span className="badge outline">{libraryName(t.library)}</span>
                    </span>
                    <span className="small muted">{t.description}</span>
                  </button>
                ))}
              </div>
            </section>
            <p className="small muted">
              Need something else? <Link to="/analytics/library">Write a visualization of your own</Link>, then add it here.
            </p>
          </div>
        )}
      </Loaded>
    </Dialog>
  );
}

function ParamInput({ name, value, fallback, onChange, projects }: { name: string; value: unknown; fallback: unknown; onChange: (v: unknown) => void; projects: { key: string; name: string }[] }) {
  if (name === "project")
    return (
      <select className="select" value={String(value ?? "")} onChange={(e) => onChange(e.target.value)}>
        <option value="">Everything I can see</option>
        {projects.map((p) => (
          <option key={p.key} value={p.key}>{p.key} · {p.name}</option>
        ))}
      </select>
    );
  if (typeof fallback === "number" || typeof value === "number") return <input className="input" type="number" value={value === null || value === undefined ? "" : String(value)} onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))} />;
  if (typeof fallback === "boolean")
    return (
      <select className="select" value={value === true ? "yes" : "no"} onChange={(e) => onChange(e.target.value === "yes")}>
        <option value="yes">Yes</option>
        <option value="no">No</option>
      </select>
    );
  return <input className="input" value={String(value ?? "")} maxLength={500} onChange={(e) => onChange(e.target.value)} />;
}

function TileSettings({ dashboard, tile, onClose }: { dashboard: Dashboard; tile: Tile; onClose: () => void }) {
  const projects = useGet<{ key: string; name: string }[]>("/api/v1/projects").data ?? [];
  const lib = useGet<Library>("/api/v1/visualizations");
  const [title, setTitle] = useState(tile.title);
  const [params, setParams] = useState<Record<string, unknown>>(tile.params ?? {});
  const [busy, run] = useAction();
  const defaults = tile.content?.params ?? {};
  const labels = tile.kind === "template" ? Object.fromEntries((lib.data?.templates.find((t) => t.key === tile.ref)?.params ?? []).map((p) => [p.key, p.label])) : {};
  const keys = Object.keys(defaults);
  return (
    <Dialog
      title={`Settings for “${tile.title || tile.content?.name || "tile"}”`}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={busy} onClick={() => run(() => api("PUT", `/api/v1/dashboards/${dashboard.id}/tiles/${tile.id}`, { title, params }), "Tile saved").then((r) => r !== undefined && onClose())}>
            Save
          </button>
        </>
      }
    >
      <div className="stack">
        <Field label="Title" hint={`Leave empty to show “${tile.content?.name ?? "the name of the visualization"}”.`}>
          <input className="input" value={title} maxLength={120} onChange={(e) => setTitle(e.target.value)} />
        </Field>
        {keys.map((k) => (
          <Field key={k} label={labels[k] ?? k} hint={k in params ? undefined : `Using the default: ${defaults[k] === "" ? "(empty)" : String(defaults[k])}`}>
            <ParamInput name={k} value={k in params ? params[k] : defaults[k]} fallback={defaults[k]} projects={projects} onChange={(v) => setParams({ ...params, [k]: v })} />
          </Field>
        ))}
        {keys.length === 0 && <p className="small muted">This visualization takes no parameters.</p>}
      </div>
    </Dialog>
  );
}

function DashboardSettings({ dashboard, canShare, onClose }: { dashboard: Dashboard; canShare: boolean; onClose: () => void }) {
  const people = usePeople();
  const [f, setF] = useState({ name: dashboard.name, description: dashboard.description, visibility: dashboard.visibility, editors: dashboard.editors.map((e) => e.id) });
  const [busy, run] = useAction();
  const [remove, setRemove] = useState(false);
  const nav = useNavigate();
  const save = () => {
    const body: Record<string, unknown> = { name: f.name.trim(), description: f.description };
    if (dashboard.mine) {
      if (f.visibility !== dashboard.visibility) body.visibility = f.visibility;
      if (JSON.stringify(f.editors) !== JSON.stringify(dashboard.editors.map((e) => e.id))) body.editors = f.editors;
    }
    return run(() => api("PATCH", `/api/v1/dashboards/${dashboard.id}`, body), "Dashboard saved").then((r) => r !== undefined && onClose());
  };
  return (
    <Dialog
      title="Dashboard settings"
      onClose={onClose}
      footer={
        <>
          {dashboard.mine && (
            <button className="btn danger" onClick={() => setRemove(true)}>
              <Trash2 /> Delete
            </button>
          )}
          <span className="grow" />
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={busy || !f.name.trim()} onClick={save}>Save</button>
        </>
      }
    >
      <div className="stack">
        <Field label="Name" required>
          <input className="input" value={f.name} maxLength={200} onChange={(e) => setF({ ...f, name: e.target.value })} />
        </Field>
        <Field label="Description">
          <textarea className="textarea" rows={2} value={f.description} maxLength={2000} onChange={(e) => setF({ ...f, description: e.target.value })} />
        </Field>
        {dashboard.mine ? (
          <>
            <Field label="Who can see it" hint={canShare ? "Each person who opens a shared dashboard sees figures for the work they themselves have access to." : "You do not have the permission to share analytics with the workspace."}>
              <select className="select" value={f.visibility} disabled={!canShare && f.visibility === "private"} onChange={(e) => setF({ ...f, visibility: e.target.value as Dashboard["visibility"] })}>
                <option value="private">Only me and the editors below</option>
                <option value="workspace">Everyone in the workspace</option>
              </select>
            </Field>
            <Field label="Who can change it with me" hint="Editors can add, move and remove tiles at the same time as you.">
              <Pick label="Editors" options={people.filter((p) => p.id !== dashboard.owner.id && p.kind !== "agent").map((p) => ({ value: p.id, label: p.name }))} value={f.editors} onChange={(v) => setF({ ...f, editors: v as string[] })} />
            </Field>
          </>
        ) : (
          <p className="small muted">Owned by {dashboard.owner.name}, who decides who can see and edit it.</p>
        )}
      </div>
      {remove && (
        <Confirm
          title={`Delete ${dashboard.name}?`}
          action="Delete dashboard"
          danger
          body="The dashboard and its layout are deleted. The visualizations on it are kept."
          onClose={() => setRemove(false)}
          onConfirm={async () => {
            await api("DELETE", `/api/v1/dashboards/${dashboard.id}`);
            toast("Dashboard deleted");
            await refresh();
            nav("/analytics");
          }}
        />
      )}
    </Dialog>
  );
}

function DashboardPage() {
  const { id } = useParams();
  const [params, setParams] = useSearchParams();
  const q = useGet<Dashboard>(`/api/v1/dashboards/${id}`, { placeholderData: (p: any) => p });
  const lib = useGet<Library>("/api/v1/visualizations", { staleTime: 60_000 });
  const projects = useGet<{ key: string; name: string }[]>("/api/v1/projects").data ?? [];
  const room = `dashboard:${id}`;
  const { peers } = useRoom(room);
  const d = q.data;
  const editing = params.get("edit") === "1" && !!d?.can_edit;
  const [active, setActive] = useState<string | null>(null);
  usePresence(room, { tile: active, editing });
  const [tick, setTick] = useState(0);
  const pendingTick = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  // When work changes, the tiles fetch their data again: at most every ten seconds.
  useLiveRefresh([room, "work"], (kind) => {
    if (kind.startsWith("dashboard.") || pendingTick.current) return;
    pendingTick.current = setTimeout(() => ((pendingTick.current = undefined), setTick((n) => n + 1)), 10_000);
  });
  useEffect(() => () => clearTimeout(pendingTick.current), []);

  const [dialog, setDialog] = useState<null | "add" | "settings" | { tile: Tile }>(null);
  const [scope, setScope] = useState<string | null>(null);
  const [said, setSaid] = useState("");
  const [busy, run] = useAction();
  // The grid appears only once there is a tile, so it is watched from the moment it mounts.
  const [grid, setGrid] = useState<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    if (!grid) return;
    // A width of zero (the grid is hidden for a moment) must not take the tiles away:
    // each would start over and fetch its data again.
    const measure = () => setWidth((w) => grid.clientWidth || w);
    const ro = new ResizeObserver(measure);
    ro.observe(grid);
    measure();
    return () => ro.disconnect();
  }, [grid]);
  const narrow = width > 0 && width < 720;
  const col = Math.max(1, (width - GAP_PX * (COLUMNS - 1)) / COLUMNS);

  // While a tile is dragged, and until the server confirms, the layout on screen is this one.
  const [draft, setDraft] = useState<Tile[] | null>(null);
  const [drag, setDrag] = useState<null | { id: string; mode: "move" | "size"; x: number; y: number; from: Tile }>(null);
  const layout = draft ?? d?.layout ?? [];
  useEffect(() => setDraft(null), [d?.version]);

  async function commit(next: Tile[]) {
    if (!d) return;
    const moved = changed(d.layout, next);
    if (moved.length === 0) return setDraft(null);
    setDraft(next);
    try {
      await Promise.all(moved.map((t) => api("PUT", `/api/v1/dashboards/${d.id}/tiles/${t.id}`, { x: t.x, y: t.y, w: t.w, h: t.h })));
    } catch (e) {
      toast(e instanceof ApiError ? e.message : "The layout could not be saved.", "error");
      setDraft(null);
    } finally {
      await refresh();
    }
  }
  const moveBy = (t: Tile, dx: number, dy: number, dw = 0, dh = 0) => settle(d!.layout, clamp({ ...t, x: t.x + dx, y: t.y + dy, w: t.w + dw, h: t.h + dh }));
  function onPointerMove(e: React.PointerEvent) {
    if (!drag || !d) return;
    const dx = Math.round((e.clientX - drag.x) / (col + GAP_PX)), dy = Math.round((e.clientY - drag.y) / (ROW_PX + GAP_PX));
    setDraft(drag.mode === "move" ? moveBy(drag.from, dx, dy) : moveBy(drag.from, 0, 0, dx, dy));
  }
  function endDrag() {
    if (!drag) return;
    setDrag(null);
    setActive(null);
    if (draft) void commit(draft);
  }
  function onKey(e: React.KeyboardEvent, t: Tile) {
    const step: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const by = step[e.key];
    if (!by || !d) return;
    e.preventDefault();
    const next = e.shiftKey ? moveBy(t, 0, 0, by[0], by[1]) : moveBy(t, by[0], by[1]);
    const now = next.find((x) => x.id === t.id)!;
    setSaid(e.shiftKey ? `${now.w} columns wide, ${now.h} rows tall` : `Column ${now.x + 1}, row ${now.y + 1}`);
    void commit(next);
  }

  const effectiveScope = scope ?? String(d?.params?.project ?? "");
  const rows = Math.max(1, ...layout.map((t) => t.y + t.h));
  const ordered = narrow ? [...layout].sort((a, b) => a.y - b.y || a.x - b.x) : layout;

  return (
    <div className="page wide">
      <Loaded q={q} what="Opening the dashboard">
        {(dash) => (
          <>
            <div className="page-head">
              <div style={{ minWidth: 0 }}>
                <div className="crumbs">
                  <Link to="/analytics">Dashboards</Link>
                </div>
                <h1 className="row wrap">
                  {dash.name} <Shared visibility={dash.visibility} />
                </h1>
                <p className="sub row wrap">
                  {dash.description && <span>{dash.description}</span>}
                  <span className="row" style={{ gap: 6 }}>{dash.mine ? "Yours" : <>By <PersonChip person={dash.owner} /></>}</span>
                  <Peers peers={peers} where={(s) => (s.tile ? `at “${dash.layout.find((t) => t.id === s.tile)?.title || dash.layout.find((t) => t.id === s.tile)?.content?.name || "a tile"}”` : s.editing ? "arranging tiles" : null)} />
                </p>
              </div>
              <div className="row wrap">
                {dash.can_edit && (
                  <>
                    {editing && (
                      <>
                        <button className="btn" onClick={() => setDialog("add")}>
                          <Plus /> Add a tile
                        </button>
                        <button className="btn" disabled={busy} title="Close the gaps between tiles" onClick={() => commit(compact(dash.layout))}>
                          Tidy up
                        </button>
                        <button className="btn" onClick={() => setDialog("settings")}>
                          <Settings /> Settings
                        </button>
                      </>
                    )}
                    <button className={`btn ${editing ? "primary" : ""}`} aria-pressed={editing} onClick={() => setParams(editing ? {} : { edit: "1" })}>
                      <Pencil /> {editing ? "Done" : "Edit"}
                    </button>
                  </>
                )}
              </div>
            </div>
            <div className="filters">
              <Field label="Show figures for">
                <select className="select" value={effectiveScope} onChange={(e) => setScope(e.target.value)}>
                  <option value="">What each tile is set to</option>
                  {projects.map((p) => (
                    <option key={p.key} value={p.key}>{p.key} · {p.name}</option>
                  ))}
                </select>
              </Field>
              {dash.can_edit && scope !== null && scope !== String(dash.params?.project ?? "") && (
                <button className="btn sm" disabled={busy} onClick={() => run(() => api("PATCH", `/api/v1/dashboards/${dash.id}`, { params: { ...dash.params, project: scope } }), "Saved as the dashboard's choice").then(() => setScope(null))}>
                  Keep this choice for everyone
                </button>
              )}
              <span className="grow" />
              <button className="btn sm" onClick={() => setTick((n) => n + 1)}>Fetch all figures again</button>
            </div>
            {dash.layout.length === 0 ? (
              <Empty icon={<ChartColumn />} title="This dashboard has no tiles yet">
                {dash.can_edit ? (
                  <button className="btn primary" onClick={() => (setParams({ edit: "1" }), setDialog("add"))}>
                    <Plus /> Add a tile
                  </button>
                ) : (
                  <p>Its owner has not added anything.</p>
                )}
              </Empty>
            ) : (
              <div ref={setGrid} className={`dash-grid ${editing ? "editing" : ""} ${narrow ? "narrow" : ""}`} style={narrow ? undefined : { height: rows * ROW_PX + (rows - 1) * GAP_PX }} onPointerMove={onPointerMove} onPointerUp={endDrag} onPointerCancel={endDrag}>
                {width > 0 &&
                  ordered.map((t) => {
                    const peer = peers.find((p) => p.state.tile === t.id);
                    const name = t.title || t.content?.name || "Tile";
                    const style: React.CSSProperties = narrow
                      ? { height: Math.max(3, t.h) * ROW_PX }
                      : { left: t.x * (col + GAP_PX), top: t.y * (ROW_PX + GAP_PX), width: t.w * col + (t.w - 1) * GAP_PX, height: t.h * ROW_PX + (t.h - 1) * GAP_PX };
                    return (
                      <section key={t.id} className={`dash-tile ${drag?.id === t.id ? "dragging" : ""}`} style={{ ...style, outline: peer ? `2px solid ${colorFor(peer.user.id)}` : undefined }} aria-label={name}>
                        <header>
                          {editing && !narrow && (
                            <button
                              className="tile-grip"
                              aria-label={`Move ${name}. Arrow keys move it, with Shift they change its size.`}
                              onPointerDown={(e) => {
                                e.preventDefault();
                                (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
                                setDrag({ id: t.id, mode: "move", x: e.clientX, y: e.clientY, from: t });
                                setActive(t.id);
                              }}
                              onKeyDown={(e) => onKey(e, t)}
                              onFocus={() => setActive(t.id)}
                              onBlur={() => setActive(null)}
                            >
                              <GripVertical />
                            </button>
                          )}
                          <h2 className="truncate grow">{name}</h2>
                          {peer && <span className="small" style={{ color: "var(--muted)" }}>{peer.user.name}</span>}
                          {t.content && !t.content.builtin && t.kind === "visualization" && (
                            <Link className="btn ghost icon sm" to={`/analytics/v/${t.ref}`} aria-label={`Open the visualization ${t.content.name}`} title="Open the visualization">
                              <Maximize2 />
                            </Link>
                          )}
                          {editing && (
                            <Menu
                              trigger={(open, toggle) => (
                                <button className="btn ghost icon sm" aria-expanded={open} aria-label={`Options for ${name}`} onClick={toggle}>
                                  <SlidersHorizontal />
                                </button>
                              )}
                            >
                              {(close) => (
                                <>
                                  <button role="menuitem" onClick={() => (close(), setDialog({ tile: t }))}>Title and parameters…</button>
                                  {t.kind === "template" && <Link role="menuitem" to={`/analytics/t/${t.ref}`}>See how it is written</Link>}
                                  <button role="menuitem" className="danger" onClick={() => (close(), void run(() => api("DELETE", `/api/v1/dashboards/${dash.id}/tiles/${t.id}`), "Tile removed"))}>
                                    Remove from the dashboard
                                  </button>
                                </>
                              )}
                            </Menu>
                          )}
                        </header>
                        <div className="tile-body">
                          {t.content ? (
                            <Sandbox
                              bare
                              refreshKey={tick}
                              content={{ kind: kindOf(t.content.library), library: t.content.library, source: t.content.source, title: name, mode: "tile", params: { ...(t.content.params ?? {}), ...t.params, ...(effectiveScope ? { project: effectiveScope } : {}) } }}
                            />
                          ) : (
                            <p className="muted small" style={{ padding: 14 }}>This visualization is private to someone else, or was deleted.</p>
                          )}
                        </div>
                        {editing && !narrow && (
                          <span
                            className="tile-size"
                            aria-hidden="true"
                            onPointerDown={(e) => {
                              e.preventDefault();
                              (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
                              setDrag({ id: t.id, mode: "size", x: e.clientX, y: e.clientY, from: t });
                              setActive(t.id);
                            }}
                          />
                        )}
                      </section>
                    );
                  })}
              </div>
            )}
            <span className="sr-only" role="status">{said}</span>
            {editing && !narrow && dash.layout.length > 0 && <p className="small muted" style={{ marginTop: 10 }}>Drag a tile by its handle to move it, or by its lower right corner to change its size. Others editing with you see the change as you make it.</p>}
            {dialog === "add" && <AddTile dashboard={dash} onClose={() => setDialog(null)} />}
            {dialog === "settings" && <DashboardSettings dashboard={dash} canShare={lib.data?.can_share ?? false} onClose={() => setDialog(null)} />}
            {dialog && typeof dialog === "object" && <TileSettings dashboard={dash} tile={dialog.tile} onClose={() => setDialog(null)} />}
          </>
        )}
      </Loaded>
    </div>
  );
}

// ───────────────────────── the library ─────────────────────────

const STARTERS: Record<string, string> = {
  chartjs: `// Chart.js is available as the global \`Chart\`. Data comes from the API through gantry.api.get.
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, group_by: 'type' });
const canvas = gantry.root.appendChild(document.createElement('canvas'));
canvas.setAttribute('role', 'img');
canvas.setAttribute('aria-label', 'Work items by type: ' + data.groups.map(g => g.value + ' ' + g.label).join(', '));
const chart = new Chart(canvas, {
  type: 'bar',
  data: {
    labels: data.groups.map(g => g.label),
    datasets: [{ label: 'Work items', data: data.groups.map(g => g.value), backgroundColor: gantry.colors.series[0], borderRadius: { topLeft: 4, topRight: 4 }, maxBarThickness: 24 }],
  },
  options: {
    maintainAspectRatio: false,
    plugins: { legend: { display: false } },
    scales: {
      x: { ticks: { color: gantry.colors.muted }, grid: { display: false } },
      y: { beginAtZero: true, ticks: { color: gantry.colors.muted, precision: 0 }, grid: { color: gantry.colors.grid } },
    },
  },
});
gantry.cleanup(() => chart.destroy());
`,
  echarts: `// Apache ECharts is available as the global \`echarts\`.
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, group_by: 'type' });
const chart = echarts.init(gantry.root, null, { renderer: 'svg' });
chart.setOption({
  aria: { enabled: true },
  textStyle: { color: gantry.colors.text, fontFamily: 'inherit' },
  grid: { left: 110, right: 32, top: 12, bottom: 28 },
  tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
  xAxis: { type: 'value', minInterval: 1, axisLabel: { color: gantry.colors.muted }, splitLine: { lineStyle: { color: gantry.colors.grid } } },
  yAxis: { type: 'category', inverse: true, data: data.groups.map(g => g.label), axisLabel: { color: gantry.colors.text } },
  series: [{ type: 'bar', name: 'Work items', data: data.groups.map(g => g.value), barMaxWidth: 22, itemStyle: { color: gantry.colors.series[0], borderRadius: [0, 4, 4, 0] }, label: { show: true, position: 'right', color: gantry.colors.text } }],
});
gantry.onResize(() => chart.resize());
gantry.cleanup(() => chart.dispose());
`,
  plotly: `// Plotly is available as the global \`Plotly\`.
const t = await gantry.api.get('/api/v1/analytics/timeline', { project: gantry.params.project, interval: 'week', periods: 16 });
const el = gantry.root.appendChild(document.createElement('div'));
el.style.cssText = 'width:100%;height:100%';
Plotly.newPlot(el, [
  { x: t.periods.map(p => p.start), y: t.periods.map(p => p.created), name: 'Created', type: 'scatter', mode: 'lines', line: { color: gantry.colors.series[0], width: 2 } },
  { x: t.periods.map(p => p.start), y: t.periods.map(p => p.resolved), name: 'Resolved', type: 'scatter', mode: 'lines', line: { color: gantry.colors.series[1], width: 2 } },
], {
  margin: { l: 40, r: 12, t: 8, b: 36 }, paper_bgcolor: 'rgba(0,0,0,0)', plot_bgcolor: 'rgba(0,0,0,0)', hovermode: 'x unified',
  font: { color: gantry.colors.text, family: 'inherit' }, legend: { orientation: 'h', y: -0.2 },
  xaxis: { gridcolor: gantry.colors.grid }, yaxis: { rangemode: 'tozero', gridcolor: gantry.colors.grid },
}, { displayModeBar: false, responsive: true });
gantry.onResize(() => Plotly.Plots.resize(el));
gantry.cleanup(() => Plotly.purge(el));
`,
  d3: `// D3 is available as the global \`d3\`. gantry.size is the space the drawing has.
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, group_by: 'priority', open: true });
const { width, height } = gantry.size;
const margin = { top: 8, right: 36, bottom: 8, left: 80 };
const y = d3.scaleBand().domain(data.groups.map(g => g.label)).range([margin.top, height - margin.bottom]).padding(0.35);
const x = d3.scaleLinear().domain([0, d3.max(data.groups, g => g.value) || 1]).range([margin.left, width - margin.right]);
const svg = d3.select(gantry.root).append('svg').attr('viewBox', [0, 0, width, height]).attr('width', '100%').attr('height', '100%')
  .attr('role', 'img').attr('aria-label', 'Open work by priority: ' + data.groups.map(g => g.value + ' ' + g.label).join(', '));
const row = svg.selectAll('g').data(data.groups).join('g').attr('transform', d => \`translate(0,\${y(d.label)})\`);
row.append('rect').attr('x', margin.left).attr('width', d => x(d.value) - margin.left).attr('height', Math.min(22, y.bandwidth())).attr('rx', 3).attr('fill', gantry.colors.series[0]);
row.append('text').attr('x', margin.left - 8).attr('y', Math.min(22, y.bandwidth()) / 2).attr('dy', '0.35em').attr('text-anchor', 'end').attr('fill', gantry.colors.text).style('font-size', '13px').text(d => d.label);
row.append('text').attr('x', d => x(d.value) + 6).attr('y', Math.min(22, y.bandwidth()) / 2).attr('dy', '0.35em').attr('fill', gantry.colors.text).style('font-size', '12px').text(d => d.value);
row.append('title').text(d => d.label + ': ' + d.value);
`,
  html: `<!-- Plain HTML. Each <script> runs in order with \`gantry\` in scope and may use await. -->
<style>
  .box { height: 100%; display: grid; place-content: center; text-align: center; }
  .box b { font-size: 44px; line-height: 1.1; }
  .box span { color: var(--muted); }
</style>
<div class="box"><b id="n">…</b><span>open work items</span></div>
<script>
  const s = await gantry.api.get('/api/v1/analytics/summary', { project: gantry.params.project });
  document.getElementById('n').textContent = gantry.formatNumber(s.open);
</script>
`,
};

function NewViz({ onClose, canCreate }: { onClose: () => void; canCreate: boolean }) {
  const [name, setName] = useState("");
  const [library, setLibrary] = useState("chartjs");
  const [busy, run] = useAction();
  const nav = useNavigate();
  return (
    <Dialog
      title="New visualization"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={busy || !name.trim() || !canCreate} onClick={() => run(() => api("POST", "/api/v1/visualizations", { name: name.trim(), library, source: STARTERS[library], params: { project: "" } }), (r: any) => nav(`/analytics/v/${r.id}`))}>
            Create
          </button>
        </>
      }
    >
      <div className="stack">
        <Field label="Name" required>
          <input className="input" autoFocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="Written with" hint="Chart.js is the quickest for common charts. Plotly and Apache ECharts offer more chart types and are still easy to learn. D3 gives you full control of the drawing.">
          <select className="select" value={library} onChange={(e) => setLibrary(e.target.value)}>
            {Object.entries(LIBRARIES).map(([k, l]) => (
              <option key={k} value={k}>{l.name}</option>
            ))}
          </select>
        </Field>
        <p className="small muted">You start from a small working example and change it. To start from one of the built-in visualizations instead, open it and make a copy.</p>
      </div>
    </Dialog>
  );
}

function LibraryPage() {
  const q = useGet<Library>("/api/v1/visualizations");
  const [creating, setCreating] = useState(false);
  const { can } = useSession();
  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Analytics</h1>
          <p className="sub">Visualizations to put on dashboards and in documents: the built-in ones, your own, and the ones others share.</p>
        </div>
        <button className="btn primary" onClick={() => setCreating(true)}>
          <Plus /> New visualization
        </button>
      </div>
      <Tabs />
      <Loaded q={q} what="Loading visualizations">
        {(l) => {
          const mine = l.visualizations.filter((v) => v.mine), others = l.visualizations.filter((v) => !v.mine);
          const list = (title: string, items: Viz[], empty: string) => (
            <section className="panel">
              <header>
                <h2>{title}</h2>
              </header>
              {items.length === 0 && <p className="muted" style={{ padding: 14 }}>{empty}</p>}
              {items.map((v) => (
                <Link key={v.id} to={`/analytics/v/${v.id}`} className="list-row">
                  <Code className="muted" />
                  <span className="grow stack tight" style={{ minWidth: 0 }}>
                    <span className="truncate">{v.name}</span>
                    {v.description && <span className="small muted truncate">{v.description}</span>}
                  </span>
                  <span className="badge outline hide-sm">{libraryName(v.library)}</span>
                  <Shared visibility={v.visibility} />
                  <span className="small muted nowrap hide-sm">{v.mine ? "" : `${v.owner.name} · `}changed {relative(v.updated_at)}</span>
                </Link>
              ))}
            </section>
          );
          return (
            <div className="stack loose">
              {list("Yours", mine, "You have not written a visualization yet. Create one, or open a built-in one and make a copy to change.")}
              {others.length > 0 && list("Shared with you", others, "")}
              <section>
                <h2 className="section-title">Built-in</h2>
                <div className="card-grid">
                  {l.templates.map((t) => (
                    <Link key={t.key} to={`/analytics/t/${t.key}`} className="card">
                      <span className="row">
                        <strong className="grow truncate">{t.name}</strong>
                        <span className="badge outline">{libraryName(t.library)}</span>
                      </span>
                      <span className="small muted">{t.description}</span>
                    </Link>
                  ))}
                </div>
              </section>
            </div>
          );
        }}
      </Loaded>
      {creating && <NewViz onClose={() => setCreating(false)} canCreate={can("workspace.access")} />}
    </div>
  );
}

/** What a script can ask for. Short on purpose; the manual has the full reference. */
function DataHelp() {
  return (
    <details className="viz-help">
      <summary>How a script gets its data, and what it may do</summary>
      <div className="prose compact">
        <p>
          A script runs in a sealed frame. It cannot open network connections, read this page, or see cookies. It gets data by asking: <code>await gantry.api.get(path, query)</code> performs a GET under <code>/api/v1/</code> as the person looking at the chart, so everyone sees figures only for work they have access to. Nothing can be changed from a script.
        </p>
        <table>
          <thead>
            <tr>
              <th scope="col">Path</th>
              <th scope="col">Returns</th>
            </tr>
          </thead>
          <tbody>
            <tr><td><code>/api/v1/analytics/summary</code></td><td>Open, overdue, unassigned, created and resolved in 30 days</td></tr>
            <tr><td><code>/api/v1/analytics/items</code></td><td>Counts or estimates in groups. <code>group_by</code>: up to three of status_category, status, type, priority, project, assignee, reporter, label, cycle, created_week, created_month, resolved_month, due_month. <code>metric</code>: count or estimate. <code>open</code>: true for unfinished work only</td></tr>
            <tr><td><code>/api/v1/analytics/timeline</code></td><td>Created, resolved and open at the end of each period. <code>interval</code>: day, week or month. <code>periods</code>: how many</td></tr>
            <tr><td><code>/api/v1/analytics/workload</code></td><td>Open, overdue and recently finished work per assignee</td></tr>
            <tr><td><code>/api/v1/analytics/cycle-time</code></td><td>Days from creation to resolution for work resolved in the last <code>days</code>, with median and percentiles</td></tr>
            <tr><td><code>/api/v1/items/query</code></td><td>Work items matching criteria: <code>{`{ q: JSON.stringify({ criteria, sort, limit }) }`}</code>, the same search as Find work</td></tr>
          </tbody>
        </table>
        <p>
          The analytics paths take <code>project</code> (a key) or <code>container</code> (the id of a program or initiative); without either they cover everything the viewer can see. Any other GET path in the API reference works too.
        </p>
        <p>
          Also in scope: <code>gantry.root</code> (the element to draw in), <code>gantry.size</code>, <code>gantry.params</code> (the values under Parameters, which each dashboard tile can override), <code>gantry.colors</code> (text, muted, grid, background, accent, todo, progress, done, and <code>series</code>: eight colors to use in order), <code>gantry.theme</code>, <code>gantry.formatDate</code>, <code>gantry.formatNumber</code>, <code>gantry.onResize(fn)</code> and <code>gantry.cleanup(fn)</code>.
        </p>
        <p>
          <a href="/manual/user/analytics.html" target="_blank" rel="noreferrer">The manual has worked examples for each library.</a>
        </p>
      </div>
    </details>
  );
}

function ParamsEditor({ value, onChange, readOnly }: { value: Record<string, unknown>; onChange: (v: Record<string, unknown>) => void; readOnly: boolean }) {
  const [name, setName] = useState("");
  const valid = /^[A-Za-z0-9_]{1,40}$/.test(name) && !(name in value);
  const parse = (text: string): unknown => (text === "true" ? true : text === "false" ? false : text.trim() !== "" && !Number.isNaN(Number(text)) ? Number(text) : text);
  return (
    <div className="stack tight">
      {Object.entries(value).map(([k, v]) => (
        <div key={k} className="row">
          <code style={{ minWidth: 90 }}>{k}</code>
          <input className="input" value={v === null || v === undefined ? "" : String(v)} disabled={readOnly} maxLength={500} aria-label={`Default value of ${k}`} placeholder={k === "project" ? "A project key, or empty for everything" : ""} onChange={(e) => onChange({ ...value, [k]: parse(e.target.value) })} />
          {!readOnly && (
            <button className="btn ghost icon sm" aria-label={`Remove the parameter ${k}`} onClick={() => onChange(Object.fromEntries(Object.entries(value).filter(([x]) => x !== k)))}>
              <Trash2 />
            </button>
          )}
        </div>
      ))}
      {Object.keys(value).length === 0 && <span className="small muted">No parameters.</span>}
      {!readOnly && (
        <form className="row" onSubmit={(e) => (e.preventDefault(), valid && (onChange({ ...value, [name]: "" }), setName("")))}>
          <input className="input" value={name} maxLength={40} placeholder="New parameter, such as weeks" aria-label="Name of a new parameter" onChange={(e) => setName(e.target.value)} />
          <button className="btn sm" disabled={!valid}>
            <Plus /> Add
          </button>
        </form>
      )}
    </div>
  );
}

function AddToDashboard({ viz, onClose }: { viz: Viz; onClose: () => void }) {
  const q = useGet<{ dashboards: Dashboard[] }>("/api/v1/dashboards");
  const [busy, run] = useAction();
  const nav = useNavigate();
  const mine = (q.data?.dashboards ?? []).filter((d) => d.can_edit);
  async function add(id: string) {
    const d = await api<Dashboard>("GET", `/api/v1/dashboards/${id}`);
    await api("PUT", `/api/v1/dashboards/${id}/tiles/${uid()}`, { kind: "visualization", ref: viz.id, ...placeNew(d.layout, 6, 4) });
    return id;
  }
  return (
    <Dialog title={`Add “${viz.name}” to a dashboard`} onClose={onClose} footer={<button className="btn" onClick={onClose}>Close</button>}>
      <Loaded q={q}>
        {() =>
          mine.length === 0 ? (
            <p className="muted">You have no dashboard you can edit. <Link to="/analytics">Create one first.</Link></p>
          ) : (
            <div className="stack tight">
              {mine.map((d) => (
                <button key={d.id} className="btn" style={{ justifyContent: "flex-start" }} disabled={busy} onClick={() => run(() => add(d.id), (id) => nav(`/analytics/d/${id}`))}>
                  <LayoutDashboard /> {d.name}
                </button>
              ))}
            </div>
          )
        }
      </Loaded>
    </Dialog>
  );
}

function VizEditor({ viz, canShare }: { viz: Viz; canShare: boolean }) {
  const { me } = useSession();
  const people = usePeople();
  const nav = useNavigate();
  const room = `viz:${viz.id}`;
  const { peers } = useRoom(room);
  usePresence(room, { focus: "source" });
  useLiveRefresh([room]);
  const [f, setF] = useState({ name: viz.name, description: viz.description, library: viz.library, params: viz.params ?? {}, visibility: viz.visibility, editors: viz.editors.map((e) => e.id) });
  const [source, setSource] = useState(viz.source ?? "");
  const [shown, setShown] = useState(viz.source ?? "");
  const [auto, setAuto] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<null | "delete" | "dashboard">(null);
  const [busy, run] = useAction();
  const savedSource = useRef(viz.source ?? "");
  // Several people can write the same script: the text is shared while it is open.
  const { shared, ready, alone } = useSharedDoc(room, "source", viz.can_edit, me.user, "text", (doc) => doc.getText("t").insert(0, viz.source ?? ""), () => window.location.reload());
  const others = peers.filter((p) => p.user.id !== me.user.id);
  // Someone else saved: take over what is not being edited here.
  useEffect(() => {
    savedSource.current = viz.source ?? "";
    setF((cur) => ({ ...cur, visibility: viz.visibility, editors: viz.editors.map((e) => e.id) }));
    if (!viz.can_edit) (setSource(viz.source ?? ""), setShown(viz.source ?? ""));
  }, [viz.version]); // eslint-disable-line react-hooks/exhaustive-deps
  // The preview follows the text a moment after typing stops.
  useEffect(() => {
    if (!auto || source === shown) return;
    const t = setTimeout(() => setShown(source), 900);
    return () => clearTimeout(t);
  }, [source, auto, shown]);

  const dirty = source !== savedSource.current || f.name !== viz.name || f.description !== viz.description || f.library !== viz.library || JSON.stringify(f.params) !== JSON.stringify(viz.params ?? {});
  const sharingChanged = f.visibility !== viz.visibility || JSON.stringify(f.editors) !== JSON.stringify(viz.editors.map((e) => e.id));
  function save() {
    const body: Record<string, unknown> = { name: f.name.trim(), description: f.description, library: f.library, params: f.params };
    if (source !== savedSource.current) (body.source = source), (body.collab = !!shared);
    if (viz.mine && sharingChanged) (body.visibility = f.visibility), (body.editors = f.editors);
    return run(() => api("PATCH", `/api/v1/visualizations/${viz.id}`, body), "Saved");
  }
  const copy = () => run(() => api("POST", "/api/v1/visualizations", { name: `Copy of ${viz.name}`.slice(0, 200), description: viz.description, library: f.library, source, params: f.params }), (r: any) => nav(`/analytics/v/${r.id}`));
  const embed = `![[viz:${viz.id}|320]]`;

  return (
    <>
      <div className="page-head">
        <div style={{ minWidth: 0 }}>
          <div className="crumbs">
            <Link to="/analytics/library">Visualizations</Link>
          </div>
          {viz.can_edit ? <input className="input title-input" value={f.name} maxLength={200} aria-label="Name" onChange={(e) => setF({ ...f, name: e.target.value })} /> : <h1>{viz.name}</h1>}
          <p className="sub row wrap">
            <span className="row" style={{ gap: 6 }}>{viz.mine ? "Yours" : <>By <PersonChip person={viz.owner} /></>}</span>
            <span className="badge outline">{libraryName(f.library)}</span>
            <Shared visibility={viz.visibility} />
            <Peers peers={peers} where={() => "has this open"} />
            {viz.can_edit && alone && <span className="badge amber">Live editing is unavailable: you are editing alone</span>}
          </p>
        </div>
        <div className="row wrap">
          <button className="btn" onClick={() => setDialog("dashboard")}>
            <LayoutDashboard /> Add to a dashboard
          </button>
          <button className="btn" disabled={busy} onClick={copy}>
            <Copy /> Save a copy
          </button>
          {viz.can_edit && shared && source !== savedSource.current && (
            <button className="btn" disabled={busy || others.length > 0} title={others.length > 0 ? "Others have this script open right now." : "Remove what was typed and not saved, and return to the saved script"} onClick={() => shared.discard()}>
              Discard changes
            </button>
          )}
          {viz.can_edit && (
            <button className="btn primary" disabled={busy || !f.name.trim() || (!dirty && !sharingChanged)} onClick={save}>
              Save
            </button>
          )}
        </div>
      </div>
      {!viz.can_edit && <div className="notice">You can look at how this is written and try changes in the preview. To keep changes, save a copy of your own.</div>}
      <div className="viz-editor">
        <section className="panel viz-code">
          <header>
            <h2>{f.library === "html" ? "HTML" : "Script"}</h2>
            <span className="row">
              <label className="check small">
                <input type="checkbox" checked={auto} onChange={(e) => setAuto(e.target.checked)} /> Run as I type
              </label>
              <button className="btn sm" onClick={() => setShown(source + (source === shown ? " " : ""))} title="Ctrl+Enter">
                <Play /> Run
              </button>
            </span>
          </header>
          <Suspense fallback={<Loading what="Opening the editor" />}>
            {viz.can_edit && !ready ? (
              <Loading what="Joining the others who have this open" lines={6} />
            ) : (
              <CodeEditor value={source} onChange={setSource} language={f.library === "html" ? "html" : "javascript"} label={`${f.library === "html" ? "HTML" : "Script"} of ${viz.name}`} shared={shared} onRun={() => setShown(source + (source === shown ? " " : ""))} />
            )}
          </Suspense>
        </section>
        <section className="panel viz-preview">
          <header>
            <h2>Preview</h2>
            <span className="small muted">With your own access, in the current theme</span>
          </header>
          <div className="viz-stage">
            <Sandbox content={{ kind: kindOf(f.library), library: f.library, source: shown, params: f.params, title: f.name, mode: "tile" }} onError={setError} />
          </div>
          {error && (
            <div className="notice danger" role="alert" style={{ margin: 10 }}>
              {error}
            </div>
          )}
        </section>
      </div>
      <div className="two-col" style={{ marginTop: 16 }}>
        <div className="stack loose">
          <section className="panel">
            <header>
              <h2>About it</h2>
            </header>
            <div className="body stack">
              <Field label="Description" hint="Shown where people choose a visualization.">
                <textarea className="textarea" rows={2} value={f.description} maxLength={2000} disabled={!viz.can_edit} onChange={(e) => setF({ ...f, description: e.target.value })} />
              </Field>
              <Field label="Written with" hint="Decides which library is loaded before the script runs.">
                <select className="select" value={f.library} disabled={!viz.can_edit} onChange={(e) => setF({ ...f, library: e.target.value })}>
                  {Object.entries(LIBRARIES).map(([k, l]) => (
                    <option key={k} value={k}>{l.name}</option>
                  ))}
                </select>
              </Field>
              <Field label="Parameters" hint="Values the script reads from gantry.params. Each dashboard tile can set its own.">
                <ParamsEditor value={f.params} readOnly={!viz.can_edit} onChange={(params) => setF({ ...f, params })} />
              </Field>
            </div>
          </section>
          <DataHelp />
        </div>
        <div className="stack loose">
          <section className="panel">
            <header>
              <h2>Sharing</h2>
            </header>
            <div className="body stack">
              {viz.mine ? (
                <>
                  <Field label="Who can use it" hint={canShare ? "Others can put a shared visualization on their dashboards and in documents. It shows each viewer only what that viewer may see." : "You do not have the permission to share analytics with the workspace."}>
                    <select className="select" value={f.visibility} disabled={!canShare && f.visibility === "private"} onChange={(e) => setF({ ...f, visibility: e.target.value as Viz["visibility"] })}>
                      <option value="private">Only me and the editors below</option>
                      <option value="workspace">Everyone in the workspace</option>
                    </select>
                  </Field>
                  <Field label="Who can change it with me">
                    <Pick label="Editors" options={people.filter((p) => p.id !== viz.owner.id && p.kind !== "agent").map((p) => ({ value: p.id, label: p.name }))} value={f.editors} onChange={(v) => setF({ ...f, editors: v as string[] })} />
                  </Field>
                </>
              ) : (
                <p className="small muted" style={{ margin: 0 }}>
                  {viz.owner.name} decides who can use and change this.
                  {viz.editors.length > 0 && <> Editors: {viz.editors.map((e) => e.name).join(", ")}.</>}
                </p>
              )}
              <Field label="In a document, a comment or a ticket" hint="Paste this line where the chart should appear. The number is its height in pixels.">
                <span className="row">
                  <code className="grow truncate">{embed}</code>
                  <CopyButton text={embed} />
                </span>
              </Field>
            </div>
          </section>
          {viz.mine && (
            <div>
              <button className="btn danger sm" onClick={() => setDialog("delete")}>
                <Trash2 /> Delete this visualization
              </button>
            </div>
          )}
        </div>
      </div>
      {dialog === "dashboard" && <AddToDashboard viz={viz} onClose={() => setDialog(null)} />}
      {dialog === "delete" && (
        <Confirm
          title={`Delete ${viz.name}?`}
          action="Delete visualization"
          danger
          body="Dashboards and documents that show it will show a note in its place."
          onClose={() => setDialog(null)}
          onConfirm={async () => {
            await api("DELETE", `/api/v1/visualizations/${viz.id}`);
            toast("Visualization deleted");
            await refresh();
            nav("/analytics/library");
          }}
        />
      )}
    </>
  );
}

function VizPage() {
  const { id } = useParams();
  const q = useGet<Viz>(`/api/v1/visualizations/${id}`);
  const lib = useGet<Library>("/api/v1/visualizations", { staleTime: 60_000 });
  return (
    <div className="page wide">
      <Loaded q={q} what="Opening the visualization">{(v) => <VizEditor key={v.id} viz={v} canShare={lib.data?.can_share ?? false} />}</Loaded>
    </div>
  );
}

function TemplatePage() {
  const { key } = useParams();
  const q = useGet<Template>(`/api/v1/visualization-templates/${key}`);
  const [busy, run] = useAction();
  const nav = useNavigate();
  const projects = useGet<{ key: string; name: string }[]>("/api/v1/projects").data ?? [];
  const [params, setParams] = useState<Record<string, unknown>>({});
  return (
    <div className="page wide">
      <Loaded q={q} what="Opening the visualization">
        {(t) => {
          const defaults = Object.fromEntries(t.params.map((p) => [p.key, p.default]));
          return (
            <>
              <div className="page-head">
                <div>
                  <div className="crumbs">
                    <Link to="/analytics/library">Visualizations</Link>
                  </div>
                  <h1 className="row wrap">
                    {t.name} <span className="badge outline">Built-in</span> <span className="badge outline">{libraryName(t.library)}</span>
                  </h1>
                  <p className="sub">{t.description}</p>
                </div>
                <button className="btn primary" disabled={busy} onClick={() => run(() => api("POST", "/api/v1/visualizations", { template: t.key, name: `My ${t.name.toLowerCase()}` }), (r: any) => nav(`/analytics/v/${r.id}`))}>
                  <Copy /> Make a copy to change
                </button>
              </div>
              <div className="filters">
                {t.params.map((p) => (
                  <Field key={p.key} label={p.label}>
                    <ParamInput name={p.key} value={p.key in params ? params[p.key] : p.default} fallback={p.default} projects={projects} onChange={(v) => setParams({ ...params, [p.key]: v })} />
                  </Field>
                ))}
              </div>
              <div className="viz-editor">
                <section className="panel viz-preview">
                  <header>
                    <h2>What it shows</h2>
                  </header>
                  <div className="viz-stage">
                    <Sandbox content={{ kind: kindOf(t.library), library: t.library, source: t.source ?? "", params: { ...defaults, ...params }, title: t.name, mode: "tile" }} />
                  </div>
                </section>
                <section className="panel viz-code">
                  <header>
                    <h2>How it is written</h2>
                    <CopyButton text={t.source ?? ""} />
                  </header>
                  <Suspense fallback={<Loading />}>
                    <CodeEditor value={t.source ?? ""} onChange={() => undefined} readOnly language={t.library === "html" ? "html" : "javascript"} label={`Source of ${t.name}`} />
                  </Suspense>
                </section>
              </div>
              <div style={{ marginTop: 16, maxWidth: 900 }}>
                <DataHelp />
              </div>
            </>
          );
        }}
      </Loaded>
    </div>
  );
}

export default function AnalyticsRoutes() {
  const { can } = useSession();
  if (!can("workspace.access")) return <Navigate to="/portal" replace />;
  return (
    <Routes>
      <Route index element={<Dashboards />} />
      <Route path="d/:id" element={<DashboardPage />} />
      <Route path="library" element={<LibraryPage />} />
      <Route path="v/:id" element={<VizPage />} />
      <Route path="t/:key" element={<TemplatePage />} />
    </Routes>
  );
}
