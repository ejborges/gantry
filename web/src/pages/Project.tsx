import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { BookOpen, ChartColumn, ChartGantt, ChevronRight, Inbox, LayoutGrid, LifeBuoy, List, Network, Plus, Settings, Trash2 } from "lucide-react";
import { api, qs, useGet, type Item, type ItemType, type Person } from "../api";
import { Avatar, Confirm, Empty, Field, Loaded, Loading, Peers, PersonChip, PersonSelect, Priority, TypeIcon, toast, useAction } from "../ui";
import { DueDate, ItemTable, NewItemDialog, statusName, typeOf, usePeople, useProject, type ProjectDetail } from "../work";
import { PRIORITIES } from "../lib/format";
import { useLiveRefresh, usePresence, useRoom } from "../realtime";
import { ProjectPlacement, ProjectWorkflows } from "./ProjectMore";

// The planning views bring a charting library with them; they load when first opened.
const GanttPanel = lazy(() => import("./Planning").then((m) => ({ default: m.GanttPanel })));
const HierarchyGraph = lazy(() => import("../components/HierarchyGraph"));
const Report = lazy(() => import("../components/Report"));
const VIEW_WORDS: Record<string, string> = { board: "on the board", list: "in the list", queues: "in the queues", gantt: "on the Gantt chart", graph: "in the graph", report: "reading the report", settings: "in the settings" };

interface ItemList {
  items: Item[];
  total: number;
}

function useDebounced<T>(value: T, ms = 250) {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

// ───────── board ─────────
const STAGES = [
  { key: "todo", name: "Not started", category: "todo" as const },
  { key: "in_progress", name: "In progress", category: "in_progress" as const },
  { key: "done", name: "Done", category: "done" as const },
];

function Board({ detail }: { detail: ProjectDetail }) {
  const [, run] = useAction();
  // Types that share a workflow share a detailed board. When a project mixes workflows,
  // "All work" shows everything in three stages so nothing is hidden on another board.
  const workflows = useMemo(() => {
    const by = new Map<string, { wf: ItemType["workflow"]; types: ItemType[] }>();
    for (const t of detail.types) by.set(t.workflow.key, { wf: t.workflow, types: [...(by.get(t.workflow.key)?.types ?? []), t] });
    return [...by.values()];
  }, [detail.types]);
  const [view, setView] = useState(workflows.length > 1 ? "all" : (workflows[0]?.wf.key ?? "all"));
  const [mine, setMine] = useState(false);
  const [text, setText] = useState("");
  const [drag, setDrag] = useState<Item | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const current = workflows.find((w) => w.wf.key === view);
  const columns = current ? current.wf.statuses : STAGES;
  const q = useGet<ItemList>(`/api/v1/items${qs({ project: detail.project.key, type: current ? current.types.map((t) => t.key).join(",") : undefined, sort: "rank", limit: 500, assignee: mine ? "me" : undefined })}`);
  const canMove = detail.permissions.includes("item.transition");
  const wfOf = (item: Item) => detail.types.find((t) => t.key === item.type)?.workflow;
  const column = (item: Item) => (current ? item.status : item.status_category);
  /** The status an item would move to if dropped on `col`, or null when the workflow forbids it. */
  const target = (item: Item | null, col: string): string | null => {
    const wf = item && wfOf(item);
    if (!item || !wf || column(item) === col) return null;
    const open = wf.transitions.filter((t) => t.to !== item.status && (t.from.includes("*") || t.from.includes(item.status)));
    if (current) return open.some((t) => t.to === col) ? col : null;
    return open.find((t) => wf.statuses.find((s) => s.key === t.to)?.category === col)?.to ?? null;
  };
  const move = (item: Item, to: string) => run(() => api("POST", `/api/v1/items/${item.key}/transition`, { to }));
  const visible = (q.data?.items ?? []).filter((i) => !text || i.title.toLowerCase().includes(text.toLowerCase()) || i.key.toLowerCase().includes(text.toLowerCase()));

  if (workflows.length === 0) return <Empty title="This project has no work item types" />;
  return (
    <>
      <div className="filters">
        <input className="input search" placeholder="Filter cards" value={text} onChange={(e) => setText(e.target.value)} aria-label="Filter cards" />
        {workflows.length > 1 && (
          <select className="select" value={view} onChange={(e) => setView(e.target.value)} aria-label="Board">
            <option value="all">All work, by stage</option>
            {workflows.map((w) => (
              <option key={w.wf.key} value={w.wf.key}>
                {w.wf.name} workflow ({w.types.length === 1 ? w.types[0].name.toLowerCase() : `${w.types.length} types`})
              </option>
            ))}
          </select>
        )}
        <label className="check">
          <input type="checkbox" checked={mine} onChange={(e) => setMine(e.target.checked)} /> Only mine
        </label>
        {q.data && q.data.total > q.data.items.length && <span className="small muted">Showing the first {q.data.items.length} of {q.data.total}</span>}
      </div>
      <Loaded q={q}>
        {() => (
          <div className="board" role="list" aria-label="Board">
            {columns.map((s) => {
              const cards = visible.filter((i) => column(i) === s.key);
              const dropTo = target(drag, s.key);
              return (
                <section
                  key={s.key}
                  role="listitem"
                  aria-label={`${s.name}, ${cards.length} ${cards.length === 1 ? "item" : "items"}`}
                  className={`column ${over === s.key && dropTo ? "over" : ""} ${drag && !dropTo && column(drag) !== s.key ? "blocked" : ""}`}
                  onDragOver={(e) => {
                    if (dropTo) {
                      e.preventDefault();
                      setOver(s.key);
                    }
                  }}
                  onDragLeave={() => setOver((o) => (o === s.key ? null : o))}
                  onDrop={(e) => {
                    e.preventDefault();
                    if (drag && dropTo) void move(drag, dropTo);
                    setDrag(null);
                    setOver(null);
                  }}
                >
                  <header>
                    <span className={`status-dot ${s.category}`} />
                    {s.name} <span className="n num">{cards.length}</span>
                  </header>
                  <div className="cards">
                    {cards.map((i) => {
                      const wf = wfOf(i);
                      const next = (wf?.transitions ?? []).filter((t) => t.to !== i.status && (t.from.includes("*") || t.from.includes(i.status)));
                      return (
                        <Link key={i.id} to={`/i/${i.key}`} className={`item-card ${drag?.id === i.id ? "dragging" : ""}`} draggable={canMove} onDragStart={(e) => (e.dataTransfer.setData("text/plain", i.key), (e.dataTransfer.effectAllowed = "move"), setDrag(i))} onDragEnd={() => (setDrag(null), setOver(null))}>
                          <span className="t">{i.title}</span>
                          <span className="row" style={{ gap: 6 }}>
                            <TypeIcon type={typeOf(detail.types, i)} />
                            <span className="key small">{i.key}</span>
                            {i.priority >= 3 && <Priority value={i.priority} label={false} />}
                            <DueDate item={i} />
                            <span className="grow" />
                            {!current && <span className="small muted">{statusName(detail.types, i)}</span>}
                            {i.assignee && <Avatar person={i.assignee} />}
                          </span>
                          {canMove && next.length > 0 && (
                            <select
                              className="select only-sm"
                              value=""
                              aria-label={`Move ${i.key}`}
                              onClick={(e) => e.preventDefault()}
                              onChange={(e) => {
                                e.preventDefault();
                                if (e.target.value) void move(i, e.target.value);
                              }}
                            >
                              <option value="">Move to…</option>
                              {next.map((t) => (
                                <option key={t.to} value={t.to}>
                                  {t.name}
                                </option>
                              ))}
                            </select>
                          )}
                        </Link>
                      );
                    })}
                    {cards.length === 0 && <span className="small muted" style={{ padding: "6px 4px" }}>{drag && dropTo ? "Drop here" : "Nothing here"}</span>}
                  </div>
                </section>
              );
            })}
          </div>
        )}
      </Loaded>
      {canMove && <p className="small muted hide-sm">Drag a card to change its status, or with the keyboard press Tab on a card to reach its <strong>Move to…</strong> list. Columns fade when the item's workflow does not allow the move.</p>}
    </>
  );
}

// ───────── list ─────────
function ItemList({ detail, people }: { detail: ProjectDetail; people: Person[] }) {
  const [text, setText] = useState("");
  const [category, setCategory] = useState("todo,in_progress");
  const [type, setType] = useState("");
  const [assignee, setAssignee] = useState("");
  const [sort, setSort] = useState("updated");
  const [limit, setLimit] = useState(50);
  const dq = useDebounced(text);
  const q = useGet<ItemList>(`/api/v1/items${qs({ project: detail.project.key, q: dq, category, type, assignee, sort, limit })}`, { placeholderData: (p) => p });
  return (
    <>
      <div className="filters">
        <input className="input search" placeholder="Search titles and keys" value={text} onChange={(e) => setText(e.target.value)} aria-label="Search" />
        <select className="select" value={category} onChange={(e) => setCategory(e.target.value)} aria-label="Status">
          <option value="todo,in_progress">Open</option>
          <option value="todo">Not started</option>
          <option value="in_progress">In progress</option>
          <option value="done">Done</option>
          <option value="">All</option>
        </select>
        <select className="select" value={type} onChange={(e) => setType(e.target.value)} aria-label="Type">
          <option value="">Any type</option>
          {detail.types.map((t) => (
            <option key={t.key} value={t.key}>
              {t.name}
            </option>
          ))}
        </select>
        <select className="select" value={assignee} onChange={(e) => setAssignee(e.target.value)} aria-label="Assignee">
          <option value="">Anyone</option>
          <option value="me">Me</option>
          <option value="none">Unassigned</option>
          {people.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <select className="select" value={sort} onChange={(e) => setSort(e.target.value)} aria-label="Sort">
          <option value="updated">Recently updated</option>
          <option value="created">Newest</option>
          <option value="priority">Priority</option>
          <option value="due">Due date</option>
          <option value="key">Key</option>
        </select>
      </div>
      <div className="panel">
        <Loaded q={q}>
          {(d) => (
            <>
              <ItemTable items={d.items} types={detail.types} empty={<Empty title="No items match these filters"><p>Change the filters above, or create an item.</p></Empty>} />
              {d.total > d.items.length && (
                <div className="spread" style={{ padding: "10px 16px", borderTop: "1px solid var(--line)" }}>
                  <span className="small muted">
                    {d.items.length} of {d.total}
                  </span>
                  <button className="btn sm" onClick={() => setLimit((l) => l + 100)}>
                    Show more
                  </button>
                </div>
              )}
            </>
          )}
        </Loaded>
      </div>
    </>
  );
}

// ───────── service desk queues ─────────
function Queues({ detail }: { detail: ProjectDetail }) {
  const [params, setParams] = useSearchParams();
  const queues = useGet<{ key: string; name: string; count: number; filter: Record<string, string> }[]>(`/api/v1/projects/${detail.project.key}/queues`, { refetchInterval: 30_000 });
  const active = queues.data?.find((x) => x.key === (params.get("queue") ?? "unassigned")) ?? queues.data?.[0];
  const items = useGet<ItemList>(active ? `/api/v1/items${qs({ project: detail.project.key, limit: 200, ...active.filter })}` : null, { refetchInterval: 30_000 });
  return (
    <Loaded q={queues}>
      {(list) => (
        <div className="stack">
          <div className="tabs" role="tablist">
            {list.map((x) => (
              <button key={x.key} role="tab" aria-selected={x.key === active?.key} onClick={() => setParams({ view: "queues", queue: x.key })}>
                {x.name}
                <span className={`count num ${x.key === "breached" && x.count > 0 ? "badge red" : ""}`}>{x.count}</span>
              </button>
            ))}
          </div>
          <div className="panel">
            <Loaded q={items}>
              {(d) => <ItemTable items={d.items} types={detail.types} showSla empty={<Empty icon={<Inbox />} title="This queue is empty"><p>New requests from the help desk portal appear here, ordered by their resolution target.</p></Empty>} />}
            </Loaded>
          </div>
        </div>
      )}
    </Loaded>
  );
}

// ───────── settings ─────────
function ProjectSettings({ detail, people }: { detail: ProjectDetail; people: Person[] }) {
  const p = detail.project;
  const base = `/api/v1/projects/${p.key}`;
  const [busy, run] = useAction();
  const cat = useGet<{ roles: any[]; groups: any[]; item_types: any[] }>("/api/v1/catalog");
  const [name, setName] = useState(p.name);
  const [description, setDescription] = useState(p.description);
  const [access, setAccess] = useState(p.access);
  const [who, setWho] = useState<string | null>(null);
  const [role, setRole] = useState("project_member");
  const [cycle, setCycle] = useState("");
  const [archive, setArchive] = useState(false);
  const enabled = new Set(detail.types.map((t) => t.key));
  const roles = (cat.data?.roles ?? []).filter((r) => r.scope === "project");
  const desk = p.kind === "service_desk";
  const sla = useGet<{ policies: { priority: number; response_minutes: number; resolution_minutes: number }[]; calendar: any }>(desk ? `${base}/sla` : null);
  const requestTypes = useGet<any[]>(desk ? `${base}/request-types` : null);
  const [rows, setRows] = useState<Record<number, { r: string; s: string }>>({});
  useEffect(() => {
    if (sla.data) setRows(Object.fromEntries(sla.data.policies.map((x) => [x.priority, { r: String(x.response_minutes), s: String(x.resolution_minutes) }])));
  }, [sla.data]);
  const [rtName, setRtName] = useState("");
  const cal = p.settings?.calendar ?? { mode: "24x7" };

  return (
    <div className="stack loose" style={{ maxWidth: 860 }}>
      <section className="panel">
        <header>
          <h2>Details</h2>
        </header>
        <div className="body stack">
          <div className="form-grid">
            <Field label="Name">
              <input className="input" value={name} onChange={(e) => setName(e.target.value)} />
            </Field>
            <Field label="Who can see it" hint="Workspace-wide visibility lets every member view, create and comment.">
              <select className="select" value={access} onChange={(e) => setAccess(e.target.value)}>
                <option value="private">Only members listed below</option>
                <option value="internal">Everyone in the workspace</option>
              </select>
            </Field>
          </div>
          <Field label="Description">
            <textarea className="textarea" value={description} onChange={(e) => setDescription(e.target.value)} />
          </Field>
          <div className="row">
            <button className="btn primary" disabled={busy} onClick={() => run(() => api("PATCH", base, { name, description, access }), "Project saved")}>
              Save changes
            </button>
            <span className="grow" />
            <button className="btn danger" onClick={() => setArchive(true)}>
              Archive project
            </button>
          </div>
        </div>
      </section>

      <ProjectPlacement detail={detail} people={people} />
      <section className="panel">
        <header>
          <h2>Members</h2>
        </header>
        <div className="table-wrap">
          <table className="table">
            <thead>
              <tr>
                <th>Person or group</th>
                <th>Role</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {detail.members.map((m) => (
                <tr key={m.binding_id}>
                  <td>{m.user ? <span className="row"><Avatar person={m.user} /> {m.user.name}</span> : <span className="row"><span className="badge outline">Group</span> {m.group?.name}</span>}</td>
                  <td>{m.role_name}</td>
                  <td style={{ textAlign: "right" }}>
                    <button className="btn ghost icon sm" aria-label={`Remove ${m.user?.name ?? m.group?.name}`} onClick={() => run(() => api("DELETE", `${base}/members/${m.binding_id}`), "Member removed")}>
                      <Trash2 />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="body row wrap" style={{ borderTop: "1px solid var(--line)" }}>
          <select className="select" style={{ width: "auto", minWidth: 200 }} value={who ?? ""} onChange={(e) => setWho(e.target.value || null)} aria-label="Person or group">
            <option value="">Choose a person or group</option>
            <optgroup label="People">
              {people.map((x) => (
                <option key={x.id} value={`u:${x.id}`}>
                  {x.name}
                  {x.kind === "agent" ? " (AI agent)" : ""}
                </option>
              ))}
            </optgroup>
            <optgroup label="Groups">
              {(cat.data?.groups ?? []).map((g) => (
                <option key={g.id} value={`g:${g.id}`}>
                  {g.name}
                </option>
              ))}
            </optgroup>
          </select>
          <select className="select" style={{ width: "auto" }} value={role} onChange={(e) => setRole(e.target.value)} aria-label="Role">
            {roles.map((r) => (
              <option key={r.key} value={r.key}>
                {r.name}
              </option>
            ))}
          </select>
          <button className="btn" disabled={!who || busy} onClick={() => run(() => api("POST", `${base}/members`, { [who!.startsWith("u:") ? "user_id" : "group_id"]: who!.slice(2), role }), "Member added").then(() => setWho(null))}>
            <Plus /> Add member
          </button>
        </div>
      </section>

      <section className="panel">
        <header>
          <h2>Work item types</h2>
          <span className="small muted">Types in use cannot be turned off.</span>
        </header>
        <div className="body perm-grid">
          {(cat.data?.item_types ?? []).map((t) => (
            <label key={t.key} className="check" title={t.description}>
              <input
                type="checkbox"
                checked={enabled.has(t.key)}
                disabled={t.key === p.default_type}
                onChange={(e) => {
                  const next = new Set(enabled);
                  if (e.target.checked) next.add(t.key);
                  else next.delete(t.key);
                  void run(() => api("PUT", `${base}/types`, { types: [...next] }));
                }}
              />
              <TypeIcon type={t} title={false} /> {t.name}
              {t.key === p.default_type && <span className="badge outline">Default</span>}
            </label>
          ))}
        </div>
      </section>

      <ProjectWorkflows detail={detail} />
      <section className="panel">
        <header>
          <h2>Cycles</h2>
          <span className="small muted">Sprints, iterations or reporting periods.</span>
        </header>
        {detail.cycles.map((c) => (
          <div key={c.id} className="list-row">
            <span className="grow">{c.name}</span>
            <select className="select" style={{ width: "auto" }} value={c.state} aria-label={`State of ${c.name}`} onChange={(e) => run(() => api("PATCH", `${base}/cycles/${c.id}`, { state: e.target.value }))}>
              <option value="planned">Planned</option>
              <option value="active">Active</option>
              <option value="closed">Closed</option>
            </select>
          </div>
        ))}
        <div className="body row" style={{ borderTop: detail.cycles.length ? "1px solid var(--line)" : 0 }}>
          <input className="input" style={{ maxWidth: 260 }} placeholder="Sprint 12" value={cycle} onChange={(e) => setCycle(e.target.value)} aria-label="New cycle name" />
          <button className="btn" disabled={!cycle.trim() || busy} onClick={() => run(() => api("POST", `${base}/cycles`, { name: cycle }), "Cycle added").then(() => setCycle(""))}>
            <Plus /> Add cycle
          </button>
        </div>
      </section>

      {desk && (
        <>
          <section className="panel">
            <header>
              <h2>Help desk portal</h2>
            </header>
            <div className="body stack">
              <label className="check">
                <input type="checkbox" checked={!!p.settings?.portal_open} onChange={(e) => run(() => api("PATCH", base, { settings: { portal_open: e.target.checked } }), e.target.checked ? "Portal opened to all signed-in users" : "Portal limited to granted customers")} />
                Any signed-in user may raise requests here
              </label>
              <p className="small muted">When off, only people granted the Service desk customer role (under Members) can use this desk's portal.</p>
              <Field label="Clock for response and resolution targets">
                <select
                  className="select"
                  style={{ maxWidth: 420 }}
                  value={cal.mode === "business" ? "business" : "24x7"}
                  onChange={(e) => run(() => api("PATCH", base, { settings: { calendar: e.target.value === "business" ? { mode: "business", utc_offset_minutes: -new Date().getTimezoneOffset(), days: [1, 2, 3, 4, 5], start: "08:00", end: "17:00" } : { mode: "24x7" } } }), "Calendar saved")}
                >
                  <option value="24x7">Around the clock</option>
                  <option value="business">Working hours: Monday through Friday, 8:00 AM to 5:00 PM (this device's time zone)</option>
                </select>
              </Field>
            </div>
          </section>
          <section className="panel">
            <header>
              <h2>Response and resolution targets</h2>
              <button
                className="btn sm primary"
                disabled={busy}
                onClick={() => {
                  const policies = Object.entries(rows).filter(([, v]) => v.r && v.s).map(([k, v]) => ({ priority: Number(k), response_minutes: Number(v.r), resolution_minutes: Number(v.s) }));
                  void run(() => api("PUT", `${base}/sla`, { policies }), "Targets saved. They apply to new requests.");
                }}
              >
                Save targets
              </button>
            </header>
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th>Priority</th>
                    <th>First response within (minutes)</th>
                    <th>Resolved within (minutes)</th>
                  </tr>
                </thead>
                <tbody>
                  {PRIORITIES.filter((x) => x.value > 0).map((x) => (
                    <tr key={x.value}>
                      <td>
                        <Priority value={x.value} />
                      </td>
                      {(["r", "s"] as const).map((f) => (
                        <td key={f}>
                          <input className="input num" style={{ maxWidth: 140 }} type="number" min={1} placeholder="No target" aria-label={`${x.label} ${f === "r" ? "response" : "resolution"} minutes`} value={rows[x.value]?.[f] ?? ""} onChange={(e) => setRows((r) => ({ ...r, [x.value]: { r: r[x.value]?.r ?? "", s: r[x.value]?.s ?? "", [f]: e.target.value } }))} />
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
          <section className="panel">
            <header>
              <h2>Request types</h2>
              <span className="small muted">What customers can choose in the portal.</span>
            </header>
            {(requestTypes.data ?? []).map((rt) => (
              <div key={rt.id} className="list-row">
                <span className="grow">
                  <strong>{rt.name}</strong> <span className="muted small">creates {detail.types.find((t) => t.key === rt.type)?.name ?? rt.type}; asks for {rt.fields.length ? rt.fields.map((f: any) => f.name).join(", ") : "summary and description only"}</span>
                </span>
                <label className="check small">
                  <input type="checkbox" checked={rt.active} onChange={(e) => run(() => api("PATCH", `${base}/request-types/${rt.id}`, { active: e.target.checked }))} /> Offered
                </label>
              </div>
            ))}
            <div className="body row" style={{ borderTop: "1px solid var(--line)" }}>
              <input className="input" style={{ maxWidth: 280 }} placeholder="Request a badge" value={rtName} onChange={(e) => setRtName(e.target.value)} aria-label="New request type name" />
              <button className="btn" disabled={!rtName.trim() || busy} onClick={() => run(() => api("POST", `${base}/request-types`, { name: rtName }), "Request type added").then(() => setRtName(""))}>
                <Plus /> Add request type
              </button>
            </div>
          </section>
        </>
      )}
      {archive && (
        <Confirm
          title={`Archive ${p.name}?`}
          action="Archive project"
          danger
          body="The project and its items disappear from lists and search. Nothing is deleted; an administrator can restore it through the API."
          onClose={() => setArchive(false)}
          onConfirm={async () => {
            await api("PATCH", base, { archived: true });
            toast("Project archived");
            window.location.assign("/projects");
          }}
        />
      )}
    </div>
  );
}

export default function Project() {
  const { key } = useParams();
  const [params, setParams] = useSearchParams();
  const q = useProject(key);
  const people = usePeople();
  const [creating, setCreating] = useState(false);
  // The board, the list and the counts follow changes others make, and the header shows
  // who else has the project open and what they are looking at.
  const room = q.data ? `project:${q.data.project.key}` : null;
  const { peers } = useRoom(room);
  const viewNow = params.get("view") ?? "board";
  usePresence(room, { view: viewNow });
  useLiveRefresh([room]);
  return (
    <div className={`page ${viewNow === "graph" || viewNow === "gantt" ? "wide" : ""}`}>
      <Loaded q={q}>
        {(detail) => {
          const desk = detail.project.kind === "service_desk";
          const view = params.get("view") ?? (desk && detail.permissions.includes("desk.agent") ? "queues" : "board");
          const tabs = [
            desk && detail.permissions.includes("desk.agent") && { key: "queues", label: "Queues", icon: Inbox },
            { key: "board", label: "Board", icon: LayoutGrid },
            { key: "list", label: "List", icon: List },
            { key: "gantt", label: "Gantt", icon: ChartGantt },
            { key: "graph", label: "Graph", icon: Network },
            detail.permissions.includes("project.reports") && { key: "report", label: "Report", icon: ChartColumn },
            detail.permissions.includes("project.admin") && { key: "settings", label: "Settings", icon: Settings },
          ].filter(Boolean) as { key: string; label: string; icon: typeof List }[];
          const c = detail.counts;
          return (
            <>
              <div className="page-head">
                <div>
                  <div className="crumbs">
                    <Link to="/projects">Projects</Link>
                    {(detail.container ?? []).map((c) => (
                      <span key={c.id} className="row" style={{ gap: 6 }}>
                        <ChevronRight aria-hidden="true" />
                        <Link to={`/planning/${c.id}`} title={c.level}>{c.name}</Link>
                      </span>
                    ))}
                  </div>
                  <h1 className="row">
                    {detail.project.name}
                    <span className="key" style={{ fontSize: 14 }}>{detail.project.key}</span>
                    {desk && (
                      <span className="badge teal">
                        <LifeBuoy /> Service desk
                      </span>
                    )}
                  </h1>
                  <p className="sub">
                    {c.todo + c.in_progress} open, {c.in_progress} in progress
                    {c.overdue > 0 && <>, <strong style={{ color: "var(--danger)" }}>{c.overdue} overdue</strong></>}
                    {desk && c.sla_breached > 0 && <>, <strong style={{ color: "var(--danger)" }}>{c.sla_breached} past target</strong></>}
                    {c.unassigned > 0 && <>, {c.unassigned} unassigned</>}
                  </p>
                  {(detail.lead || peers.length > 0) && (
                    <p className="sub row wrap" style={{ marginTop: 6 }}>
                      {detail.lead && <span className="row" style={{ gap: 6 }}>Lead: <PersonChip person={detail.lead} /></span>}
                      <Peers peers={peers} where={(st) => VIEW_WORDS[st.view] ?? null} />
                    </p>
                  )}
                </div>
                <div className="row">
                  {detail.space && (
                    <Link className="btn" to={`/docs/${detail.space.key}`}>
                      <BookOpen /> {detail.space.kind === "knowledge_base" ? "Knowledge base" : "Docs"}
                    </Link>
                  )}
                  {detail.permissions.includes("item.create") && (
                    <button className="btn primary" onClick={() => setCreating(true)}>
                      <Plus /> New item
                    </button>
                  )}
                </div>
              </div>
              <div className="tabs" role="tablist" style={{ marginBottom: 16 }}>
                {tabs.map((t) => (
                  <button key={t.key} role="tab" aria-selected={view === t.key} onClick={() => setParams({ view: t.key })}>
                    <t.icon /> {t.label}
                  </button>
                ))}
              </div>
              {view === "board" && <Board detail={detail} />}
              {view === "list" && <ItemList detail={detail} people={people} />}
              {view === "queues" && desk && <Queues detail={detail} />}
              <Suspense fallback={<Loading what="Opening" />}>
                {view === "gantt" && <GanttPanel scope={{ project: detail.project.key }} name={detail.project.name} />}
                {view === "graph" && <HierarchyGraph project={detail.project.key} />}
                {view === "report" && detail.permissions.includes("project.reports") && <Report path={`/api/v1/reports/project/${detail.project.key}`} />}
              </Suspense>
              {view === "settings" && detail.permissions.includes("project.admin") && <ProjectSettings detail={detail} people={people} />}
              {creating && <NewItemDialog detail={detail} onClose={() => setCreating(false)} />}
            </>
          );
        }}
      </Loaded>
    </div>
  );
}
