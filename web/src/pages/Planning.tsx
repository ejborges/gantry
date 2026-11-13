// Planning: the levels above projects (programs, initiatives, or whatever an organization
// calls them), what each holds, reports for their owners, the hierarchy as a live graph,
// and Gantt charts that are edited in place.
import { useEffect, useRef, useState } from "react";
import { Link, NavLink, Navigate, Route, Routes, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { ArrowDown, ArrowUp, ChartGantt, ChevronRight, Copy, FolderTree, Layers, Network, Pencil, Plus, Trash2, Users } from "lucide-react";
import { ApiError, api, qs, refresh, useGet, type Person } from "../api";
import { useSession } from "../session";
import { Avatar, Confirm, Dialog, Empty, Field, Loaded, Markdown, Peers, PersonChip, PersonSelect, toast, useAction } from "../ui";
import { day, relative } from "../lib/format";
import { useLiveRefresh, usePresence, useRoom } from "../realtime";
import { usePeople } from "../work";
import Gantt, { GanttSettingsBar, PALETTE, cssColor, type Annotation, type GanttItem, type GanttSettings } from "../components/Gantt";
import HierarchyGraph from "../components/HierarchyGraph";
import Report from "../components/Report";

interface Level {
  id: string;
  position: number;
  name: string;
  plural: string;
  color: string;
}
interface Container {
  id: string;
  name: string;
  description: string;
  parent_id: string | null;
  status: string;
  start_date: string | null;
  target_date: string | null;
  owner: Person | null;
  level: Level;
  projects: { key: string; name: string; kind: string; open?: number; done?: number; description?: string }[];
  mine?: boolean;
}
export const CONTAINER_STATUS: Record<string, string> = { planned: "Planned", active: "Active", on_hold: "On hold", done: "Done" };
const TONE: Record<string, string> = { planned: "todo", active: "blue", on_hold: "amber", done: "green" };

export function LevelBadge({ level }: { level: Pick<Level, "name" | "color"> }) {
  return (
    <span className="badge outline level" style={{ borderColor: cssColor(level.color), color: cssColor(level.color) }}>
      {level.name}
    </span>
  );
}

function Tabs() {
  const cls = ({ isActive }: { isActive: boolean }) => (isActive ? "active" : "");
  return (
    <nav className="tabs" aria-label="Planning" style={{ marginBottom: 16 }}>
      <NavLink to="/planning" end className={cls}><FolderTree /> Structure</NavLink>
      <NavLink to="/planning/graph" className={cls}><Network /> Graph</NavLink>
      <NavLink to="/planning/gantt" className={cls}><ChartGantt /> Gantt charts</NavLink>
      <NavLink to="/planning/levels" className={cls}><Layers /> Levels</NavLink>
    </nav>
  );
}

function Head({ title, sub, children }: { title: React.ReactNode; sub?: React.ReactNode; children?: React.ReactNode }) {
  return (
    <div className="page-head">
      <div>
        <h1>{title}</h1>
        {sub && <p className="sub">{sub}</p>}
      </div>
      {children && <div className="row wrap">{children}</div>}
    </div>
  );
}

// ───────────────────────── structure ─────────────────────────

function ContainerDialog({ levels, containers, initial, parent, onClose }: { levels: Level[]; containers: Container[]; initial?: Container; parent?: Container; onClose: (id?: string) => void }) {
  const people = usePeople().filter((p) => p.kind !== "agent");
  const [busy, run] = useAction();
  const below = parent ? levels.filter((l) => l.position > parent.level.position) : levels;
  const [f, setF] = useState({
    level_id: initial?.level.id ?? below[0]?.id ?? "",
    name: initial?.name ?? "",
    description: initial?.description ?? "",
    status: initial?.status ?? "planned",
    start_date: initial?.start_date ?? "",
    target_date: initial?.target_date ?? "",
    owner_id: initial?.owner?.id ?? null as string | null,
    parent_id: initial?.parent_id ?? parent?.id ?? null as string | null,
  });
  const level = levels.find((l) => l.id === f.level_id);
  // A parent sits on a higher level, and is never the thing itself.
  const parents = containers.filter((c) => level && c.level.position < level.position && c.id !== initial?.id);
  const wrongOrder = !!f.start_date && !!f.target_date && f.target_date < f.start_date;
  function save() {
    const body: Record<string, unknown> = { name: f.name.trim(), description: f.description, status: f.status, start_date: f.start_date || null, target_date: f.target_date || null };
    if (!initial) return run(() => api("POST", "/api/v1/containers", { ...body, level_id: f.level_id, parent_id: f.parent_id }), (r: any) => (toast(`${level?.name ?? "Item"} created`), onClose(r.id)));
    if (f.owner_id !== (initial.owner?.id ?? null)) body.owner_id = f.owner_id;
    if (f.parent_id !== initial.parent_id) body.parent_id = f.parent_id;
    return run(() => api("PATCH", `/api/v1/containers/${initial.id}`, body), () => (toast("Saved"), onClose(initial.id)));
  }
  return (
    <Dialog
      title={initial ? `Edit ${initial.level.name.toLowerCase()}` : `New ${level?.name.toLowerCase() ?? "level"}`}
      onClose={() => onClose()}
      footer={
        <>
          <button className="btn" onClick={() => onClose()}>Cancel</button>
          <button className="btn primary" disabled={busy || !f.name.trim() || !f.level_id || wrongOrder} onClick={save}>
            {initial ? "Save" : "Create"}
          </button>
        </>
      }
    >
      <div className="stack">
        {!initial && below.length > 1 && (
          <Field label="Level">
            <select className="select" value={f.level_id} onChange={(e) => setF({ ...f, level_id: e.target.value, parent_id: parent?.id ?? null })}>
              {below.map((l) => (
                <option key={l.id} value={l.id}>{l.name}</option>
              ))}
            </select>
          </Field>
        )}
        <Field label="Name" required>
          <input className="input" autoFocus value={f.name} maxLength={200} onChange={(e) => setF({ ...f, name: e.target.value })} />
        </Field>
        <Field label="What it is for" hint="Markdown is supported.">
          <textarea className="textarea" rows={4} value={f.description} maxLength={20000} onChange={(e) => setF({ ...f, description: e.target.value })} />
        </Field>
        <div className="form-grid">
          <Field label="Status">
            <select className="select" value={f.status} onChange={(e) => setF({ ...f, status: e.target.value })}>
              {Object.entries(CONTAINER_STATUS).map(([k, v]) => (
                <option key={k} value={k}>{v}</option>
              ))}
            </select>
          </Field>
          <Field label="Beneath" hint={parents.length === 0 ? "Nothing on a higher level exists yet." : undefined}>
            <select className="select" value={f.parent_id ?? ""} disabled={!!parent && !initial} onChange={(e) => setF({ ...f, parent_id: e.target.value || null })}>
              <option value="">Nothing (top of the hierarchy)</option>
              {parents.map((c) => (
                <option key={c.id} value={c.id}>{c.level.name}: {c.name}</option>
              ))}
            </select>
          </Field>
          <Field label="Starts">
            <input className="input" type="date" value={f.start_date} onChange={(e) => setF({ ...f, start_date: e.target.value })} />
          </Field>
          <Field label="Target date" hint={wrongOrder ? <span className="bad-text">The target date cannot be before the start.</span> : undefined}>
            <input className="input" type="date" value={f.target_date} min={f.start_date || undefined} onChange={(e) => setF({ ...f, target_date: e.target.value })} />
          </Field>
        </div>
        {initial && (
          <Field label="Owner" hint="The owner sees reports for everything beneath, including projects they are not a member of.">
            <PersonSelect people={people} value={f.owner_id} onChange={(id) => setF({ ...f, owner_id: id })} none="Nobody" />
          </Field>
        )}
      </div>
    </Dialog>
  );
}

/** What the "New …" button offers, in the organization's own words: "program", "program or initiative", "portfolio, program or mission". */
export function newLabel(levels: { name: string }[]): string {
  const names = levels.map((l) => l.name.toLowerCase());
  if (names.length === 0 || names.length > 3) return "entry";
  return names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} or ${names[names.length - 1]}`;
}

/** Open and finished work in the projects of this container and of everything beneath it that the reader can see. */
function workUnder(c: Container, all: Container[], depth = 0): { open: number; done: number } {
  const own = { open: c.projects.reduce((n, p) => n + (p.open ?? 0), 0), done: c.projects.reduce((n, p) => n + (p.done ?? 0), 0) };
  if (depth > 12) return own;
  return all.filter((x) => x.parent_id === c.id).reduce((t, kid) => {
    const k = workUnder(kid, all, depth + 1);
    return { open: t.open + k.open, done: t.done + k.done };
  }, own);
}

function Branch({ c, all, depth }: { c: Container; all: Container[]; depth: number }) {
  const kids = all.filter((x) => x.parent_id === c.id);
  const { open, done } = workUnder(c, all);
  const share = open + done > 0 ? Math.round((100 * done) / (open + done)) : 0;
  return (
    <li>
      <div className="plan-row" style={{ paddingLeft: 12 + depth * 22 }}>
        <LevelBadge level={c.level} />
        <Link to={`/planning/${c.id}`} className="grow truncate plan-name">{c.name}</Link>
        <span className={`badge ${TONE[c.status] ?? ""}`}>{CONTAINER_STATUS[c.status] ?? c.status}</span>
        {c.target_date && <span className="small muted nowrap hide-sm">Target {day(c.target_date)}</span>}
        {open + done > 0 && (
          <span className="plan-progress hide-sm" title={`${share}% of the work beneath this is done`}>
            <span className="small muted nowrap">{open} open · {done} done</span>
            <span className="meter" aria-hidden="true"><span style={{ width: `${share}%` }} /></span>
          </span>
        )}
        <Avatar person={c.owner} size="sm" />
      </div>
      {(kids.length > 0 || c.projects.length > 0) && (
        <ul>
          {kids.map((k) => (
            <Branch key={k.id} c={k} all={all} depth={depth + 1} />
          ))}
          {c.projects.map((p) => (
            <li key={p.key}>
              <div className="plan-row project" style={{ paddingLeft: 12 + (depth + 1) * 22 }}>
                <span className="badge outline">Project</span>
                <Link to={`/projects/${p.key}`} className="grow truncate">{p.name}</Link>
                <span className="key">{p.key}</span>
                <span className="small muted nowrap hide-sm">{p.open} open · {p.done} done</span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}

function Structure() {
  const q = useGet<{ containers: Container[]; unplaced_projects: { key: string; name: string; kind: string }[]; levels: Level[]; can_manage: boolean }>("/api/v1/containers");
  const [creating, setCreating] = useState(false);
  const nav = useNavigate();
  useLiveRefresh(["planning"]);
  return (
    <div className="page">
      <Head title="Planning" sub="How programs, initiatives and projects fit together, and how each is doing.">
        {q.data?.can_manage && q.data.levels.length > 0 && (
          <button className="btn primary" onClick={() => setCreating(true)}>
            <Plus /> New {newLabel(q.data.levels)}
          </button>
        )}
      </Head>
      <Tabs />
      <Loaded q={q} what="Loading the hierarchy">
        {(d) => {
          const ids = new Set(d.containers.map((c) => c.id));
          const roots = d.containers.filter((c) => !c.parent_id || !ids.has(c.parent_id));
          return (
            <div className="stack loose">
              {d.levels.length === 0 ? (
                <Empty icon={<Layers />} title="No levels above projects yet">
                  <p>Define levels such as Program and Initiative to group projects. {d.can_manage ? <Link to="/planning/levels">Define the levels</Link> : "Someone who manages planning can define them."}</p>
                </Empty>
              ) : roots.length === 0 ? (
                <Empty icon={<FolderTree />} title={`No ${d.levels[0].plural.toLowerCase()} yet`}>
                  <p>{d.can_manage ? `Create a ${d.levels[0].name.toLowerCase()} and place projects in it.` : "Nothing has been set up above the projects."}</p>
                </Empty>
              ) : (
                <section className="panel">
                  <ul className="plan-tree">
                    {roots.map((c) => (
                      <Branch key={c.id} c={c} all={d.containers} depth={0} />
                    ))}
                  </ul>
                </section>
              )}
              {d.unplaced_projects.length > 0 && d.levels.length > 0 && (
                <section className="panel">
                  <header>
                    <h2>Projects not placed anywhere</h2>
                    <span className="small muted">A project's administrator places it from the project's settings.</span>
                  </header>
                  {d.unplaced_projects.map((p) => (
                    <Link key={p.key} to={`/projects/${p.key}`} className="list-row">
                      <span className="key">{p.key}</span>
                      <span className="grow truncate">{p.name}</span>
                      <ChevronRight className="muted" />
                    </Link>
                  ))}
                </section>
              )}
              {creating && <ContainerDialog levels={d.levels} containers={d.containers} onClose={(id) => (setCreating(false), id && nav(`/planning/${id}`))} />}
            </div>
          );
        }}
      </Loaded>
    </div>
  );
}

function ContainerPage() {
  const { id } = useParams();
  const [params, setParams] = useSearchParams();
  const view = params.get("view") ?? "overview";
  const q = useGet<Container & { ancestors: { id: string; name: string; level: string }[]; children: Container[]; can_edit: boolean; can_manage: boolean; archived: boolean }>(`/api/v1/containers/${id}`);
  const all = useGet<{ containers: Container[]; unplaced_projects: { key: string; name: string }[]; levels: Level[] }>("/api/v1/containers");
  const [dialog, setDialog] = useState<"edit" | "child" | "archive" | null>(null);
  const [place, setPlace] = useState("");
  const [busy, run] = useAction();
  const nav = useNavigate();
  useLiveRefresh(["planning"]);
  return (
    <div className="page">
      <Loaded q={q} what="Loading">
        {(c) => {
          const lower = (all.data?.levels ?? []).filter((l) => l.position > c.level.position);
          return (
            <>
              <div className="page-head">
                <div>
                  <div className="crumbs">
                    <Link to="/planning">Planning</Link>
                    {c.ancestors.map((a) => (
                      <span key={a.id} className="row" style={{ gap: 6 }}>
                        <ChevronRight aria-hidden="true" />
                        <Link to={`/planning/${a.id}`}>{a.name}</Link>
                      </span>
                    ))}
                  </div>
                  <h1 className="row wrap">
                    <LevelBadge level={c.level} /> {c.name}
                    <span className={`badge ${TONE[c.status] ?? ""}`}>{CONTAINER_STATUS[c.status] ?? c.status}</span>
                    {c.archived && <span className="badge">Archived</span>}
                  </h1>
                  <p className="sub row wrap">
                    <span className="row" style={{ gap: 6 }}>Owner: <PersonChip person={c.owner} fallback="Nobody" /></span>
                    {(c.start_date || c.target_date) && (
                      <span>
                        {c.start_date ? day(c.start_date) : "No start"} to {c.target_date ? day(c.target_date) : "no target date"}
                      </span>
                    )}
                  </p>
                </div>
                {c.can_edit && (
                  <div className="row wrap">
                    {lower.length > 0 && (
                      <button className="btn" onClick={() => setDialog("child")}>
                        <Plus /> Add {lower.length === 1 ? lower[0].name.toLowerCase() : "beneath"}
                      </button>
                    )}
                    <button className="btn" onClick={() => setDialog("edit")}>
                      <Pencil /> Edit
                    </button>
                  </div>
                )}
              </div>
              <div className="tabs" role="tablist" style={{ marginBottom: 16 }}>
                {[
                  ["overview", "Overview"],
                  ["report", "Report"],
                  ["graph", "Graph"],
                  ["gantt", "Gantt"],
                ].map(([k, label]) => (
                  <button key={k} role="tab" aria-selected={view === k} onClick={() => setParams(k === "overview" ? {} : { view: k })}>
                    {label}
                  </button>
                ))}
              </div>
              {view === "overview" && (
                <div className="stack loose">
                  {c.description && (
                    <section className="panel">
                      <div className="body">
                        <Markdown source={c.description} />
                      </div>
                    </section>
                  )}
                  {c.children.length > 0 && (
                    <section className="panel">
                      <header>
                        <h2>Beneath this {c.level.name.toLowerCase()}</h2>
                      </header>
                      {c.children.map((k) => (
                        <Link key={k.id} to={`/planning/${k.id}`} className="list-row">
                          <LevelBadge level={k.level} />
                          <span className="grow truncate">{k.name}</span>
                          <span className={`badge ${TONE[k.status] ?? ""}`}>{CONTAINER_STATUS[k.status] ?? k.status}</span>
                          <Avatar person={k.owner} size="sm" />
                        </Link>
                      ))}
                    </section>
                  )}
                  <section className="panel">
                    <header>
                      <h2>Projects</h2>
                    </header>
                    {c.projects.length === 0 && <p className="muted" style={{ padding: 14 }}>No projects are placed directly in this {c.level.name.toLowerCase()}.</p>}
                    {c.projects.map((p) => (
                      <div key={p.key} className="list-row">
                        <span className="key">{p.key}</span>
                        <Link to={`/projects/${p.key}`} className="grow truncate">{p.name}</Link>
                        {c.can_edit && (
                          <button className="btn ghost sm" disabled={busy} onClick={() => run(() => api("PUT", `/api/v1/projects/${p.key}/container`, { container_id: null }), `${p.name} taken out`)}>
                            Take out
                          </button>
                        )}
                      </div>
                    ))}
                    {c.can_edit && (all.data?.unplaced_projects.length ?? 0) > 0 && (
                      <form className="list-row wrap" onSubmit={(e) => (e.preventDefault(), place && run(() => api("PUT", `/api/v1/projects/${place}/container`, { container_id: c.id }), "Project placed").then(() => setPlace("")))}>
                        <select className="select" style={{ maxWidth: 320 }} value={place} onChange={(e) => setPlace(e.target.value)} aria-label="A project that is not placed anywhere">
                          <option value="">Place a project here…</option>
                          {all.data!.unplaced_projects.map((p) => (
                            <option key={p.key} value={p.key}>{p.key} · {p.name}</option>
                          ))}
                        </select>
                        <button className="btn sm" disabled={!place || busy}>Place</button>
                        <span className="small muted">You need to administer the project to place it.</span>
                      </form>
                    )}
                  </section>
                  {c.can_edit && !c.archived && (
                    <div>
                      <button className="btn danger sm" onClick={() => setDialog("archive")}>Archive this {c.level.name.toLowerCase()}</button>
                    </div>
                  )}
                </div>
              )}
              {view === "report" && <Report path={`/api/v1/reports/container/${c.id}`} />}
              {view === "graph" && <HierarchyGraph container={c.id} />}
              {view === "gantt" && <GanttPanel scope={{ container: c.id }} name={c.name} />}
              {dialog === "edit" && all.data && <ContainerDialog levels={all.data.levels} containers={all.data.containers} initial={c} onClose={() => setDialog(null)} />}
              {dialog === "child" && all.data && <ContainerDialog levels={all.data.levels} containers={all.data.containers} parent={c} onClose={(nid) => (setDialog(null), nid && nav(`/planning/${nid}`))} />}
              {dialog === "archive" && (
                <Confirm
                  title={`Archive ${c.name}?`}
                  action="Archive"
                  danger
                  body="It disappears from planning. The projects in it are not archived or changed; they simply are no longer placed anywhere."
                  onClose={() => setDialog(null)}
                  onConfirm={async () => {
                    await api("PATCH", `/api/v1/containers/${c.id}`, { archived: true });
                    toast("Archived");
                    await refresh();
                    nav("/planning");
                  }}
                />
              )}
            </>
          );
        }}
      </Loaded>
    </div>
  );
}

// ───────────────────────── Gantt ─────────────────────────

interface GanttData {
  items: GanttItem[];
  cycles: { id: string; name: string; project: string; start_date: string | null; end_date: string | null }[];
  editable: Record<string, boolean>;
  chart?: { id: string; name: string; scope: any; annotations: Annotation[]; settings: GanttSettings; shared: boolean; mine: boolean; owner: Person; updated_at: string };
}

async function saveDates(item: GanttItem, start: string | null, due: string | null) {
  try {
    await api("PATCH", `/api/v1/items/${item.key}`, { start_date: start, due_date: due });
  } catch (e) {
    toast(e instanceof ApiError ? e.message : "The dates could not be saved. Check your connection and try again.", "error");
  } finally {
    await refresh();
  }
}

function remembered(key: string): GanttSettings {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "{}") ?? {};
  } catch {
    return {};
  }
}

/** A chart of a project or a program that is not saved: for a quick look and quick edits. */
export function GanttPanel({ scope, name }: { scope: { project?: string; container?: string }; name: string }) {
  const store = `gantry.gantt.${scope.project ?? scope.container}`;
  const [settings, setSettings] = useState<GanttSettings>(() => remembered(store));
  const q = useGet<GanttData>(`/api/v1/gantt/data${qs(scope)}`, { placeholderData: (p: any) => p });
  const [busy, run] = useAction();
  const nav = useNavigate();
  useLiveRefresh(["work", scope.project ? `project:${scope.project}` : null]);
  const change = (s: GanttSettings) => {
    setSettings(s);
    try {
      localStorage.setItem(store, JSON.stringify(s));
    } catch {
      /* the choice simply is not remembered */
    }
  };
  return (
    <div className="stack">
      <div className="row wrap">
        <GanttSettingsBar value={settings} onChange={change} />
        <span className="grow" />
        <button className="btn sm" disabled={busy} title="A saved chart can carry marker lines, shaded spans and groups, and can be shared" onClick={() => run(() => api("POST", "/api/v1/gantt", { name: `${name} plan`, scope, settings }), (r: any) => nav(`/planning/gantt/${r.id}`))}>
          <Plus /> Save as a chart
        </button>
      </div>
      <Loaded q={q} what="Laying out the chart" lines={8}>
        {(d) => <Gantt items={d.items} cycles={d.cycles} editable={d.editable} settings={settings} annotations={[]} onDates={saveDates} />}
      </Loaded>
      <p className="small muted hide-sm">Drag a bar to move it, or pull an end to change one date. With the keyboard: focus a bar, then use the arrow keys (Shift for the due date, Alt for the start).</p>
    </div>
  );
}

function NewChartDialog({ onClose }: { onClose: () => void }) {
  const projects = useGet<any[]>("/api/v1/projects");
  const containers = useGet<{ containers: Container[] }>("/api/v1/containers");
  const [name, setName] = useState("");
  const [scope, setScope] = useState("");
  const [busy, run] = useAction();
  const nav = useNavigate();
  return (
    <Dialog
      title="New Gantt chart"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button
            className="btn primary"
            disabled={busy || !name.trim() || !scope}
            onClick={() => run(() => api("POST", "/api/v1/gantt", { name: name.trim(), scope: scope.startsWith("p:") ? { project: scope.slice(2) } : { container: scope.slice(2) } }), (r: any) => nav(`/planning/gantt/${r.id}`))}
          >
            Create
          </button>
        </>
      }
    >
      <div className="stack">
        <Field label="Name" required>
          <input className="input" autoFocus value={name} maxLength={200} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field label="What it shows" required hint="A chart shows the work of one project, or of everything beneath a program or initiative.">
          <select className="select" value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="">Choose…</option>
            {(containers.data?.containers.length ?? 0) > 0 && (
              <optgroup label="Programs and initiatives">
                {containers.data!.containers.map((c) => (
                  <option key={c.id} value={`c:${c.id}`}>{c.level.name}: {c.name}</option>
                ))}
              </optgroup>
            )}
            <optgroup label="Projects">
              {(projects.data ?? []).map((p) => (
                <option key={p.key} value={`p:${p.key}`}>{p.key} · {p.name}</option>
              ))}
            </optgroup>
          </select>
        </Field>
      </div>
    </Dialog>
  );
}

function GanttList() {
  const q = useGet<{ id: string; name: string; scope: any; shared: boolean; updated_at: string; owner: Person; mine: boolean }[]>("/api/v1/gantt");
  const [creating, setCreating] = useState(false);
  return (
    <div className="page">
      <Head title="Planning" sub="Gantt charts you have saved, and the ones others share.">
        <button className="btn primary" onClick={() => setCreating(true)}>
          <Plus /> New chart
        </button>
      </Head>
      <Tabs />
      <Loaded q={q} what="Loading charts">
        {(list) =>
          list.length === 0 ? (
            <Empty icon={<ChartGantt />} title="No saved charts yet">
              <p>Every project and program has a Gantt view. Save one as a chart to add deadline lines, shaded spans and groups, and to share it.</p>
            </Empty>
          ) : (
            <section className="panel">
              {list.map((g) => (
                <Link key={g.id} to={`/planning/gantt/${g.id}`} className="list-row">
                  <ChartGantt className="muted" />
                  <span className="grow truncate">{g.name}</span>
                  {g.shared && <span className="badge outline"><Users /> Shared</span>}
                  <span className="small muted nowrap hide-sm">{g.mine ? "Yours" : `By ${g.owner.name}`} · changed {relative(g.updated_at)}</span>
                </Link>
              ))}
            </section>
          )
        }
      </Loaded>
      {creating && <NewChartDialog onClose={() => setCreating(false)} />}
    </div>
  );
}

function GanttChartPage() {
  const { id } = useParams();
  const q = useGet<GanttData>(`/api/v1/gantt/${id}`, { placeholderData: (p: any) => p });
  const room = `gantt:${id}`;
  const { peers } = useRoom(room);
  const [focus, setFocus] = useState<string | null>(null);
  usePresence(room, { focus: focus ? `bar:${focus}` : null });
  useLiveRefresh(["work", room]);
  const chart = q.data?.chart;
  const [draft, setDraft] = useState<{ annotations: Annotation[]; settings: GanttSettings; name: string } | null>(null);
  const dirty = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [confirm, setConfirm] = useState(false);
  const [busy, run] = useAction();
  const nav = useNavigate();
  // What is on screen follows the saved chart, except while the owner's own change is on its way.
  useEffect(() => {
    if (chart && !dirty.current) setDraft({ annotations: chart.annotations, settings: chart.settings, name: chart.name });
  }, [chart?.updated_at, chart?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => clearTimeout(timer.current), []);

  function change(patch: Partial<{ annotations: Annotation[]; settings: GanttSettings; name: string }>) {
    if (!draft || !chart) return;
    const next = { ...draft, ...patch };
    setDraft(next);
    if (!chart.mine) return; // someone else's chart: the change is only for this visit
    dirty.current = true;
    clearTimeout(timer.current);
    timer.current = setTimeout(async () => {
      try {
        await api("PATCH", `/api/v1/gantt/${chart.id}`, { annotations: next.annotations, settings: next.settings, ...(next.name.trim() ? { name: next.name.trim() } : {}) });
      } catch (e) {
        toast(e instanceof ApiError ? e.message : "The chart could not be saved.", "error");
      } finally {
        dirty.current = false;
        void refresh();
      }
    }, 600);
  }

  return (
    <div className="page">
      <Loaded q={q} what="Laying out the chart" lines={8}>
        {(d) =>
          !d.chart || !draft ? null : (
            <>
              <div className="page-head">
                <div style={{ minWidth: 0 }}>
                  <div className="crumbs">
                    <Link to="/planning/gantt">Gantt charts</Link>
                  </div>
                  {d.chart.mine ? (
                    <input className="input title-input" value={draft.name} maxLength={200} aria-label="Name of the chart" onChange={(e) => change({ name: e.target.value })} />
                  ) : (
                    <h1>{d.chart.name}</h1>
                  )}
                  <p className="sub row wrap">
                    <span className="row" style={{ gap: 6 }}>{d.chart.mine ? "Your chart" : <>By <PersonChip person={d.chart.owner} /></>}</span>
                    <span>{d.chart.scope.project ? <Link to={`/projects/${d.chart.scope.project}`}>Project {d.chart.scope.project}</Link> : <Link to={`/planning/${d.chart.scope.container}`}>Everything beneath a program or initiative</Link>}</span>
                    <Peers peers={peers} where={(s) => (typeof s.focus === "string" && s.focus.startsWith("bar:") ? s.focus.slice(4) : null)} />
                  </p>
                </div>
                <div className="row wrap">
                  {d.chart.mine ? (
                    <>
                      <label className="check" title="Everyone in the workspace can open a shared chart. They see only the work they have access to.">
                        <input type="checkbox" checked={d.chart.shared} disabled={busy} onChange={(e) => run(() => api("PATCH", `/api/v1/gantt/${d.chart!.id}`, { shared: e.target.checked }), e.target.checked ? "Shared with the workspace" : "No longer shared")} /> Shared
                      </label>
                      <button className="btn danger sm" onClick={() => setConfirm(true)}>
                        <Trash2 /> Delete
                      </button>
                    </>
                  ) : (
                    <button className="btn" disabled={busy} onClick={() => run(() => api("POST", "/api/v1/gantt", { name: `Copy of ${d.chart!.name}`.slice(0, 200), scope: d.chart!.scope, annotations: draft.annotations, settings: draft.settings }), (r: any) => nav(`/planning/gantt/${r.id}`))}>
                      <Copy /> Save a copy of my own
                    </button>
                  )}
                </div>
              </div>
              <div className="stack">
                <GanttSettingsBar value={draft.settings} onChange={(settings) => change({ settings })} />
                {!d.chart.mine && <p className="small muted">This is {d.chart.owner.name}'s chart. You can change how you look at it, and move work you are allowed to edit; the lines, spans and groups are theirs.</p>}
                <Gantt items={d.items} cycles={d.cycles} editable={d.editable} settings={draft.settings} annotations={draft.annotations} canAnnotate={d.chart.mine} onAnnotations={(annotations) => change({ annotations })} onDates={saveDates} peers={peers} onFocusItem={setFocus} />
                <p className="small muted hide-sm">Drag a bar to move it, or pull an end to change one date. With the keyboard: focus a bar, then use the arrow keys (Shift for the due date, Alt for the start). Every change is saved to the work item and recorded in its history.</p>
              </div>
              {confirm && (
                <Confirm
                  title={`Delete ${d.chart.name}?`}
                  action="Delete chart"
                  danger
                  body="The chart, its lines, spans and groups are deleted. The work items and their dates are not touched."
                  onClose={() => setConfirm(false)}
                  onConfirm={async () => {
                    await api("DELETE", `/api/v1/gantt/${d.chart!.id}`);
                    toast("Chart deleted");
                    await refresh();
                    nav("/planning/gantt");
                  }}
                />
              )}
            </>
          )
        }
      </Loaded>
    </div>
  );
}

// ───────────────────────── levels ─────────────────────────

type Draft = { id?: string; name: string; plural: string; color: string };

function LevelList({ kind, levels, canManage, lead, footer }: { kind: "container" | "item"; levels: Level[]; canManage: boolean; lead: string; footer: string }) {
  const [rows, setRows] = useState<Draft[]>(levels.map((l) => ({ id: l.id, name: l.name, plural: l.plural, color: l.color })));
  const saved = JSON.stringify(levels.map((l) => ({ id: l.id, name: l.name, plural: l.plural, color: l.color })));
  useEffect(() => setRows(JSON.parse(saved)), [saved]);
  const [busy, run] = useAction();
  const move = (i: number, by: number) => setRows((r) => {
    const next = [...r];
    [next[i], next[i + by]] = [next[i + by], next[i]];
    return next;
  });
  const set = (i: number, patch: Partial<Draft>) => setRows((r) => r.map((x, n) => (n === i ? { ...x, ...patch } : x)));
  const changed = JSON.stringify(rows) !== saved;
  const valid = rows.every((r) => r.name.trim()) && new Set(rows.map((r) => r.name.trim().toLowerCase())).size === rows.length;
  return (
    <section className="panel">
      <header>
        <h2>{kind === "container" ? "Above projects" : "Within projects"}</h2>
        <span className="small muted">{lead}</span>
      </header>
      <ol className="levels">
        {rows.map((r, i) => (
          <li key={r.id ?? `new${i}`} className="list-row wrap">
            <span className="num muted" aria-hidden="true">{i + 1}</span>
            <input className="input" style={{ maxWidth: 200 }} value={r.name} maxLength={60} disabled={!canManage} aria-label={`Name of level ${i + 1}`} placeholder="Name, such as Program" onChange={(e) => set(i, { name: e.target.value })} />
            <input className="input" style={{ maxWidth: 200 }} value={r.plural} maxLength={60} disabled={!canManage} aria-label={`Plural of level ${i + 1}`} placeholder="Plural, such as Programs" onChange={(e) => set(i, { plural: e.target.value })} />
            <span className="row" role="radiogroup" aria-label={`Color of level ${i + 1}`} style={{ gap: 3 }}>
              {PALETTE.map((c) => (
                <button key={c} type="button" role="radio" aria-checked={r.color === c} aria-label={c} title={c} disabled={!canManage} className="swatch-btn" style={{ background: cssColor(c) }} onClick={() => set(i, { color: c })} />
              ))}
            </span>
            <span className="grow" />
            {canManage && (
              <>
                <button className="btn ghost icon sm" aria-label={`Move ${r.name || "level"} up`} disabled={i === 0} onClick={() => move(i, -1)}><ArrowUp /></button>
                <button className="btn ghost icon sm" aria-label={`Move ${r.name || "level"} down`} disabled={i === rows.length - 1} onClick={() => move(i, 1)}><ArrowDown /></button>
                <button className="btn ghost icon sm" aria-label={`Remove ${r.name || "level"}`} onClick={() => setRows((x) => x.filter((_, n) => n !== i))}><Trash2 /></button>
              </>
            )}
          </li>
        ))}
        {rows.length === 0 && <li className="muted" style={{ padding: 14 }}>No levels.</li>}
        <li className="list-row small muted">{footer}</li>
      </ol>
      {canManage && (
        <div className="body row wrap">
          <button className="btn sm" disabled={rows.length >= 12} onClick={() => setRows((r) => [...r, { name: "", plural: "", color: "neutral" }])}>
            <Plus /> Add a level
          </button>
          <span className="grow" />
          {changed && <button className="btn sm" onClick={() => setRows(JSON.parse(saved))}>Discard</button>}
          <button className="btn primary sm" disabled={busy || !changed || !valid} onClick={() => run(() => api("PUT", "/api/v1/hierarchy/levels", { kind, levels: rows.map((r) => ({ id: r.id, name: r.name.trim(), plural: r.plural.trim() || undefined, color: r.color })) }), "Levels saved")}>
            Save levels
          </button>
        </div>
      )}
    </section>
  );
}

function Levels() {
  const q = useGet<{ container_levels: Level[]; item_levels: Level[]; item_types: { key: string; name: string; level_id: string | null }[]; enforce_item_levels: boolean; can_manage: boolean }>("/api/v1/hierarchy");
  const [busy, run] = useAction();
  useLiveRefresh(["planning"]);
  return (
    <div className="page">
      <Head title="Planning" sub="Name the levels your organization plans on. Work then nests the same way everywhere: in the graph, in reports and on Gantt charts." />
      <Tabs />
      <Loaded q={q} what="Loading the levels">
        {(h) => (
          <div className="stack loose" style={{ maxWidth: 900 }}>
            {!h.can_manage && <div className="notice">You can see how the hierarchy is defined. Changing it needs the permission to manage planning.</div>}
            <LevelList kind="container" levels={h.container_levels} canManage={h.can_manage} lead="From the top. For example: Portfolio, Program, Initiative." footer="Projects come next, beneath the lowest level here." />
            <LevelList kind="item" levels={h.item_levels} canManage={h.can_manage} lead="From the top. For example: Epic, Story, Subtask." footer="A work item can have a parent on any higher level." />
            <section className="panel">
              <header>
                <h2>Which level each type of work is on</h2>
              </header>
              <div className="table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <th scope="col">Type</th>
                      <th scope="col">Level</th>
                    </tr>
                  </thead>
                  <tbody>
                    {h.item_types.map((t) => (
                      <tr key={t.key}>
                        <th scope="row" style={{ fontWeight: 400 }}>{t.name}</th>
                        <td>
                          <select className="select" style={{ maxWidth: 260 }} value={t.level_id ?? ""} disabled={!h.can_manage || busy} aria-label={`Level of ${t.name}`} onChange={(e) => run(() => api("PUT", `/api/v1/hierarchy/item-types/${t.key}`, { level_id: e.target.value || null }), `${t.name} moved`)}>
                            <option value="">No level (can sit anywhere)</option>
                            {h.item_levels.map((l) => (
                              <option key={l.id} value={l.id}>{l.name}</option>
                            ))}
                          </select>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {h.can_manage && (
                <div className="body">
                  <label className="check">
                    <input type="checkbox" checked={h.enforce_item_levels === true} disabled={busy} onChange={(e) => run(() => api("PUT", "/api/v1/admin/settings/hierarchy", { enforce_item_levels: e.target.checked }), e.target.checked ? "Levels are now enforced" : "Levels are no longer enforced")} /> Refuse a parent that is not on a higher level than its child
                  </label>
                  <p className="small muted" style={{ marginTop: 4 }}>When this is off, the levels are a guide: any work item can be the parent of any other.</p>
                </div>
              )}
            </section>
          </div>
        )}
      </Loaded>
    </div>
  );
}

function GraphPage() {
  return (
    <div className="page wide">
      <Head title="Planning" sub="Every program, initiative, project and work item you can see, and how they hang together. It updates as work changes." />
      <Tabs />
      <HierarchyGraph tall />
    </div>
  );
}

export default function PlanningRoutes() {
  const { can } = useSession();
  if (!can("workspace.access")) return <Navigate to="/portal" replace />;
  return (
    <Routes>
      <Route index element={<Structure />} />
      <Route path="graph" element={<GraphPage />} />
      <Route path="gantt" element={<GanttList />} />
      <Route path="gantt/:id" element={<GanttChartPage />} />
      <Route path="levels" element={<Levels />} />
      <Route path=":id" element={<ContainerPage />} />
    </Routes>
  );
}
