import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { Bell, BellOff, ChevronRight, ExternalLink, FileText, History, Link2, MessageSquare, Network, Paperclip, Pencil, Plus, Trash2, Upload, X } from "lucide-react";
import { api, upload, useGet, type Item } from "../api";
import { Avatar, Confirm, CustomFieldInput, Empty, Field, Loaded, Loading, Markdown, Peers, PersonChip, PersonSelect, SlaChip, StatusBadge, TypeIcon, fieldText, toast, useAction } from "../ui";
import { NewItemDialog, statusName, usePeople, useProject, type ProjectDetail } from "../work";
import { bytes, dateTime, PRIORITIES, relative, stamp, titleCase } from "../lib/format";
import { Discussion, ItemHistory } from "../components/Discussion";
import { useLiveRefresh, usePresence, useRoom, useSharedDoc } from "../realtime";
import { useSession } from "../session";

const MarkdownEditor = lazy(() => import("../editors/MarkdownEditor"));

/** What a colleague is doing on this item, from the presence they share. */
const WHERE: Record<string, string> = { description: "editing the description", title: "editing the title", assignee: "choosing the assignee", priority: "setting the priority", start_date: "setting the start date", due_date: "setting the due date", cycle: "choosing the cycle", labels: "editing the labels", details: "editing the details" };
const whereText = (state: Record<string, any>) => (state.writing ? "writing a message" : state.focus ? (WHERE[state.focus] ?? null) : null);

const RELATIONS = ["relates_to", "blocks", "duplicates", "derives_from", "verifies", "mitigates", "threatens", "raised_at", "waives", "changes", "documented_in"];

/** How a link reads from the far end: "A blocks B" appears on B as "blocked by A". */
const INVERSE: Record<string, string> = {
  blocks: "blocked by", relates_to: "relates to", duplicates: "duplicated by", derives_from: "source of", verifies: "verified by", mitigates: "mitigated by",
  threatens: "threatened by", raised_at: "raised here", waives: "waived by", changes: "changed by", documented_in: "documents",
};
const relLabel = (l: { rel: string; direction: string }) => (l.direction === "out" ? l.rel.replace(/_/g, " ") : (INVERSE[l.rel] ?? `${l.rel.replace(/_/g, " ")} (incoming)`));

interface GraphData {
  root: { id: string };
  nodes: any[];
  edges: { id: string; rel: string; src: { type: string; id: string }; dst: { type: string; id: string } }[];
}

/** Links around an item, laid out in rings by distance from it. */
function Graph({ itemKey }: { itemKey: string }) {
  const q = useGet<GraphData>(`/api/v1/items/${itemKey}/graph?depth=2`);
  const navigate = useNavigate();
  return (
    <Loaded q={q} lines={2}>
      {(g) => {
        if (g.edges.length === 0) return <p className="muted small">No links to draw yet.</p>;
        const level = new Map<string, number>([[g.root.id, 0]]);
        for (let d = 0; d < 3; d++)
          for (const e of g.edges) {
            const [a, b] = [e.src.id, e.dst.id];
            if (level.get(a) === d && !level.has(b)) level.set(b, d + 1);
            if (level.get(b) === d && !level.has(a)) level.set(a, d + 1);
          }
        const W = 760, H = 420, cx = W / 2, cy = H / 2;
        const pos = new Map<string, { x: number; y: number }>([[g.root.id, { x: cx, y: cy }]]);
        for (const lv of [1, 2]) {
          const ring = g.nodes.filter((n) => level.get(n.id) === lv);
          ring.forEach((n, i) => {
            const a = (2 * Math.PI * i) / Math.max(ring.length, 1) + (lv === 2 ? 0.4 : -Math.PI / 2);
            pos.set(n.id, { x: cx + Math.cos(a) * (lv === 1 ? 210 : 320) * 0.95, y: cy + Math.sin(a) * (lv === 1 ? 120 : 185) });
          });
        }
        const label = (n: any) => (n.key ? `${n.key} ${n.title}` : n.title ?? n.id).slice(0, 26);
        return (
          <svg className="graph" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`Links around ${itemKey}: ${g.edges.length} connections`}>
            {g.edges.map((e) => {
              const a = pos.get(e.src.id), b = pos.get(e.dst.id);
              if (!a || !b) return null;
              return (
                <g key={e.id}>
                  <line className="edge" x1={a.x} y1={a.y} x2={b.x} y2={b.y} />
                  <text className="edge-label" x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 4} textAnchor="middle">
                    {e.rel.replace(/_/g, " ")}
                  </text>
                </g>
              );
            })}
            {g.nodes.map((n) => {
              const p = pos.get(n.id);
              if (!p) return null;
              const internal = typeof n.url === "string" && n.url.startsWith("/");
              return (
                <g key={n.id} className={`node ${n.id === g.root.id ? "root" : ""}`} transform={`translate(${p.x - 88}, ${p.y - 15})`} style={{ cursor: internal ? "pointer" : "default" }} onClick={() => internal && navigate(n.url)}>
                  <rect width="176" height="30" />
                  <text x="88" y="19.5" textAnchor="middle">
                    {label(n)}
                  </text>
                </g>
              );
            })}
          </svg>
        );
      }}
    </Loaded>
  );
}

function Details({ item, detail }: { item: Item; detail: ProjectDetail }) {
  const people = usePeople();
  const type = detail.types.find((t) => t.key === item.type);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Record<string, any>>({});
  const [busy, run] = useAction();
  const canEdit = item.permissions.includes("item.edit");
  if (!type || type.fields.length === 0) return null;
  const filled = type.fields.filter((f) => fieldText(f, item.custom?.[f.key], people));
  return (
    <section>
      <div className="section-title">
        <h2>{type.name} details</h2>
        {canEdit && !editing && (
          <button className="btn sm" onClick={() => (setDraft({ ...item.custom }), setEditing(true))}>
            <Pencil /> Edit details
          </button>
        )}
      </div>
      {editing ? (
        <div className="stack">
          <div className="form-grid">
            {type.fields.map((f) => (
              <Field key={f.key} label={f.name} hint={f.description} required={f.required}>
                <CustomFieldInput field={f} value={draft[f.key]} onChange={(v) => setDraft((d) => ({ ...d, [f.key]: v }))} people={people} />
              </Field>
            ))}
          </div>
          <div className="row">
            <button
              className="btn primary"
              disabled={busy}
              onClick={() => {
                const changes = Object.fromEntries(type.fields.map((f) => [f.key, draft[f.key] ?? null]));
                void run(() => api("PATCH", `/api/v1/items/${item.key}`, { custom: changes }), "Details saved").then((r) => r && setEditing(false));
              }}
            >
              Save details
            </button>
            <button className="btn" onClick={() => setEditing(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : filled.length === 0 ? (
        <p className="muted">Nothing filled in yet.</p>
      ) : (
        <dl className="props" style={{ gridTemplateColumns: "minmax(120px, 200px) minmax(0, 1fr)", alignItems: "start" }}>
          {filled.map((f) => (
            <div key={f.key} style={{ display: "contents" }}>
              <dt>{f.name}</dt>
              <dd style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
                {f.type === "url" ? (
                  <a href={/^https?:\/\//.test(item.custom[f.key]) ? item.custom[f.key] : "#"} target="_blank" rel="noopener noreferrer">
                    {item.custom[f.key]}
                  </a>
                ) : (
                  fieldText(f, item.custom[f.key], people)
                )}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  );
}

function ItemView({ item, detail }: { item: Item; detail: ProjectDetail }) {
  const people = usePeople();
  const navigate = useNavigate();
  const [busy, run] = useAction();
  const base = `/api/v1/items/${item.key}`;
  const can = (p: string) => item.permissions.includes(p);
  const type = detail.types.find((t) => t.key === item.type);
  const desk = detail.project.kind === "service_desk";
  const [title, setTitle] = useState(item.title);
  const [editDesc, setEditDesc] = useState(false);
  const [desc, setDesc] = useState(item.description ?? "");
  const [tab, setTab] = useState<"comments" | "history">("comments");
  const { me } = useSession();
  const room = `item:${item.key}`;
  const { peers } = useRoom(room);
  const [focus, setFocus] = useState<string | null>(null);
  useLiveRefresh([room]);
  const together = useSharedDoc(room, "description", editDesc, me.user, "text", (d) => d.getText("t").insert(0, item.description ?? ""), (mine) => {
    if (!mine) toast("The unsaved changes to the description were discarded, or it was saved from somewhere else. Open it again to continue from the saved text.");
    setEditDesc(false);
  });
  usePresence(room, editDesc ? { focus: "description" } : focus ? { focus } : null);
  const on = (field: string) => peers.filter((p) => p.state.focus === field);
  const [rel, setRel] = useState("relates_to");
  const [target, setTarget] = useState("");
  const [showGraph, setShowGraph] = useState(false);
  const [labels, setLabels] = useState(item.labels.join(", "));
  const [child, setChild] = useState(false);
  const [archive, setArchive] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const titleRef = useRef<HTMLTextAreaElement>(null);
  // The title wraps instead of being cut off; grow the field to fit it.
  useEffect(() => {
    const el = titleRef.current;
    if (el) {
      el.style.height = "auto";
      el.style.height = `${el.scrollHeight}px`;
    }
  }, [title]);
  useEffect(() => {
    setTitle(item.title);
    setLabels(item.labels.join(", "));
  }, [item.title, item.labels, item.key]);
  const patch = (changes: Record<string, unknown>, done?: string) => run(() => api("PATCH", base, changes), done);

  return (
    <>
      <div className="crumbs">
        <Link to="/projects">Projects</Link>
        <ChevronRight />
        <Link to={`/projects/${detail.project.key}`}>{detail.project.name}</Link>
        {item.parent && (
          <>
            <ChevronRight />
            <Link to={`/i/${item.parent.key}`}>{item.parent.key}</Link>
          </>
        )}
      </div>
      <header style={{ marginBottom: 4 }}>
        <div className="row" style={{ marginBottom: 6 }}>
          <TypeIcon type={type} />
          <span className="key">{item.key}</span>
          <span className="muted small">{type?.name}</span>
          <StatusBadge name={statusName(detail.types, item)} category={item.status_category} />
          {item.channel !== "web" && <span className="badge outline">via {item.channel}</span>}
          <span className="grow" />
          <Peers peers={peers} where={whereText} />
          {item.assignee && (
            <span className="row small muted only-sm" style={{ gap: 6 }}>
              <Avatar person={item.assignee} />
            </span>
          )}
        </div>
        {can("item.edit") ? (
          <>
            {/* The page keeps a heading for people who move through it by headings; the box below is the same title, editable. */}
            <h1 className="sr-only" data-title={`${item.key} ${item.title}`}>{item.title}</h1>
            <textarea
              ref={titleRef}
              className="title-edit"
              aria-label="Title"
              rows={1}
              value={title}
              maxLength={500}
              onChange={(e) => setTitle(e.target.value.replace(/\n/g, " "))}
              onBlur={() => title.trim() && title !== item.title && patch({ title })}
              onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), (e.target as HTMLTextAreaElement).blur())}
            />
          </>
        ) : (
          <h1 data-title={`${item.key} ${item.title}`}>{item.title}</h1>
        )}
        {can("item.transition") && item.transitions.length > 0 && (
          <div className="row wrap" style={{ marginTop: 10 }} role="group" aria-label="Change status">
            {item.transitions.map((t: any, i: number) => (
              <button key={t.to} className={`btn sm ${i === 0 && t.category !== "todo" ? "primary" : ""}`} disabled={busy} onClick={() => run(() => api("POST", `${base}/transition`, { to: t.to }), `Moved to ${t.to_name}`)} title={t.requires?.length ? `Needs: ${t.requires.map(titleCase).join(", ")}` : undefined}>
                {t.name}
              </button>
            ))}
          </div>
        )}
      </header>
      <div className="detail">
        <div className="stack" style={{ gap: 0 }}>
          <section>
            <div className="section-title">
              <h2>Description</h2>
              <Peers peers={on("description")} where={() => "editing"} />
              {can("item.edit") && !editDesc && (
                <button className="btn sm" onClick={() => (setDesc(item.description ?? ""), setEditDesc(true))}>
                  <Pencil /> Edit
                </button>
              )}
            </div>
            {editDesc ? (
              <div className="stack">
                {together.ready ? (
                  <Suspense fallback={<Loading lines={4} what="Opening the editor" />}>
                    <MarkdownEditor value={desc} onChange={setDesc} label="Description" shared={together.shared} storageKey="description" uploadTo={`${base}/attachments`} attachments={item.attachments} people={people} autoFocus />
                  </Suspense>
                ) : (
                  <Loading lines={4} what="Joining the others who have this open" />
                )}
                <div className="row wrap">
                  <button className="btn primary" disabled={busy || !together.ready} onClick={() => patch({ description: desc, ...(together.shared ? { collab: true } : {}) }, "Description saved").then((r) => r && setEditDesc(false))}>
                    Save description
                  </button>
                  <button className="btn" onClick={() => setEditDesc(false)} title="What you typed stays here for whoever opens the description next">
                    Close without saving
                  </button>
                  {together.shared && desc.trim() !== (item.description ?? "").trim() && (
                    <button
                      className="btn"
                      disabled={on("description").length > 0}
                      title={on("description").length > 0 ? "Others are editing the description right now." : "Remove what was typed and not saved, and return to the saved description"}
                      onClick={() => {
                        together.shared?.discard();
                        setEditDesc(false);
                        toast("Unsaved changes discarded");
                      }}
                    >
                      Discard changes
                    </button>
                  )}
                  <span className="small muted">{together.alone ? "Editing on your own: the live connection is not available." : "Everyone with this open sees the same text as it is typed. Saving publishes it."}</span>
                </div>
              </div>
            ) : (
              <Markdown source={item.description ?? ""} refs={item.description_refs} attachments={item.attachments} compact empty="No description." />
            )}
          </section>

          <Details item={item} detail={detail} />

          {(item.children.length > 0 || can("item.create")) && (
            <section>
              <div className="section-title">
                <h2>Sub-items</h2>
                {can("item.create") && (
                  <button className="btn sm" onClick={() => setChild(true)}>
                    <Plus /> Add sub-item
                  </button>
                )}
              </div>
              {item.children.length > 0 && (
                <div className="panel">
                  {item.children.map((c: any) => (
                    <Link key={c.key} to={`/i/${c.key}`} className="list-row">
                      <span className={`status-dot ${c.status_category}`} />
                      <span className="key">{c.key}</span>
                      <span className="grow truncate">{c.title}</span>
                      <StatusBadge name={statusName(detail.types, c)} category={c.status_category} />
                    </Link>
                  ))}
                </div>
              )}
            </section>
          )}

          <section>
            <div className="section-title">
              <h2>Links</h2>
              {item.links.length > 0 && (
                <button className="btn sm" onClick={() => setShowGraph((s) => !s)} aria-pressed={showGraph}>
                  <Network /> {showGraph ? "Hide graph" : "Show graph"}
                </button>
              )}
            </div>
            {showGraph && <Graph itemKey={item.key} />}
            {item.links.length > 0 && (
              <div className="panel" style={{ marginTop: showGraph ? 10 : 0 }}>
                {item.links.map((l: any) => {
                  const o = l.other;
                  const text = o.key ? `${o.key} ${o.title}` : o.title;
                  return (
                    <div key={l.id} className="list-row">
                      <span className="badge outline">{relLabel(l)}</span>
                      {o.type === "page" ? <FileText width={15} height={15} /> : o.external ? <ExternalLink width={15} height={15} /> : <span className={`status-dot ${o.status_category}`} />}
                      {o.external ? (
                        o.url ? <a className="grow truncate" href={o.url} target="_blank" rel="noopener noreferrer">{text}</a> : <span className="grow truncate">{text}</span>
                      ) : (
                        <Link className="grow truncate" to={o.url}>{text}</Link>
                      )}
                      {can("item.link") && l.direction === "out" && (
                        <button className="btn ghost icon sm" aria-label={`Remove link to ${text}`} onClick={() => run(() => api("DELETE", `/api/v1/links/${l.id}`), "Link removed")}>
                          <X />
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
            {can("item.link") && (
              <form
                className="row wrap"
                style={{ marginTop: 10 }}
                onSubmit={(e) => {
                  e.preventDefault();
                  void run(() => api("POST", `${base}/links`, { rel, target: target.trim().toUpperCase() }), "Link added").then((r) => r && setTarget(""));
                }}
              >
                <select className="select" style={{ width: "auto" }} value={rel} onChange={(e) => setRel(e.target.value)} aria-label="Relation">
                  {RELATIONS.map((r) => (
                    <option key={r} value={r}>
                      {r.replace(/_/g, " ")}
                    </option>
                  ))}
                </select>
                <input className="input" style={{ maxWidth: 160 }} placeholder="Item key, e.g. OPS-12" value={target} onChange={(e) => setTarget(e.target.value)} aria-label="Target item key" />
                <button className="btn" disabled={!target.trim() || busy}>
                  <Link2 /> Add link
                </button>
              </form>
            )}
          </section>

          <section>
            <div className="section-title">
              <h2>Files</h2>
              {can("item.comment") && (
                <>
                  <input
                    ref={fileRef}
                    type="file"
                    hidden
                    multiple
                    onChange={async (e) => {
                      for (const f of Array.from(e.target.files ?? [])) await run(() => upload(`${base}/attachments`, f), `${f.name} attached`);
                      e.target.value = "";
                    }}
                  />
                  <button className="btn sm" onClick={() => fileRef.current?.click()} disabled={busy}>
                    <Upload /> Attach files
                  </button>
                </>
              )}
            </div>
            {item.attachments.length === 0 ? (
              <p className="muted">No files attached.</p>
            ) : (
              <div className="panel">
                {item.attachments.map((a: any) => (
                  <div key={a.id} className="list-row file-row">
                    {String(a.content_type).startsWith("image/") ? <img className="thumb" src={`/api/v1/attachments/${a.id}`} alt="" loading="lazy" /> : <Paperclip width={15} height={15} />}
                    <span className="grow stack tight" style={{ minWidth: 0 }}>
                      <a className="truncate" href={`/api/v1/attachments/${a.id}`}>
                        {a.filename}
                      </a>
                      {/^(audio|video)\//.test(String(a.content_type)) && <Markdown source={`![${a.filename}](/api/v1/attachments/${a.id})`} attachments={item.attachments} compact />}
                      <span className="small muted">
                        {bytes(a.size)}
                        {a.uploaded_by ? ` · ${a.uploaded_by.name}${a.uploaded_by.kind === "agent" ? " (AI agent)" : ""}` : ""}
                        {a.created_at ? ` · ${stamp(a.created_at)}` : ""}
                      </span>
                    </span>
                    {can("item.edit") && (
                      <button className="btn ghost icon sm" aria-label={`Delete ${a.filename}`} onClick={() => run(() => api("DELETE", `/api/v1/attachments/${a.id}`), "File deleted")}>
                        <Trash2 />
                      </button>
                    )}
                  </div>
                ))}
              </div>
            )}
          </section>

          <section>
            <div className="tabs" role="tablist" style={{ marginTop: 24 }}>
              <button role="tab" id="tab-discussion" aria-controls="panel-discussion" aria-selected={tab === "comments"} onClick={() => setTab("comments")}>
                <MessageSquare /> Discussion <span className="count num">{item.comments.filter((c: any) => !c.deleted).length}</span>
              </button>
              <button role="tab" id="tab-history" aria-controls="panel-history" aria-selected={tab === "history"} onClick={() => setTab("history")}>
                <History /> History <span className="count num">{item.history_total}</span>
              </button>
            </div>
            {tab === "comments" ? (
              <div role="tabpanel" id="panel-discussion" aria-labelledby="tab-discussion">
                <Discussion
                  messages={item.comments}
                  postTo={`${base}/comments`}
                  canPost={can("item.comment")}
                  canInternal={desk && can("desk.agent")}
                  canModerate={can("comment.moderate")}
                  uploadTo={can("item.comment") ? `${base}/attachments` : undefined}
                  attachments={item.attachments}
                  room={room}
                  people={people}
                  desk={desk}
                />
              </div>
            ) : (
              <div role="tabpanel" id="panel-history" aria-labelledby="tab-history">
                <ItemHistory itemKey={item.key} total={item.history_total} />
              </div>
            )}
          </section>
        </div>

        <aside className="detail-side panel" aria-label="Properties">
          <div className="body">
            <dl className="props" onFocusCapture={(e) => setFocus((e.target as HTMLElement).closest("[data-field]")?.getAttribute("data-field") ?? null)} onBlurCapture={() => setFocus(null)}>
              <dt>Status</dt>
              <dd>
                <StatusBadge name={statusName(detail.types, item)} category={item.status_category} />
              </dd>
              {item.sla && (
                <>
                  <dt>Response</dt>
                  <dd>
                    <SlaChip due={item.sla.response_due} breached={item.sla.response_breached} metAt={item.sla.first_response_at} what="First response" />
                  </dd>
                  <dt>Resolution</dt>
                  <dd>
                    <SlaChip due={item.sla.resolution_due} breached={item.sla.resolution_breached} metAt={item.resolved_at} what="Resolution" />
                  </dd>
                </>
              )}
              <dt>
                Assignee <Peers peers={on("assignee")} />
              </dt>
              <dd data-field="assignee" className="stack tight">
                <PersonChip person={item.assignee} presence />
                {can("item.assign") && <PersonSelect label="Assignee" people={people} value={item.assignee?.id ?? null} onChange={(id) => patch({ assignee_id: id })} />}
              </dd>
              <dt>
                Priority <Peers peers={on("priority")} />
              </dt>
              <dd data-field="priority">
                <select className="select" value={item.priority} disabled={!can("item.edit")} onChange={(e) => patch({ priority: Number(e.target.value) })} aria-label="Priority">
                  {PRIORITIES.map((p) => (
                    <option key={p.value} value={p.value}>
                      {p.label}
                    </option>
                  ))}
                </select>
              </dd>
              <dt>
                Start <Peers peers={on("start_date")} />
              </dt>
              <dd data-field="start_date">
                <input className="input" type="date" value={item.start_date ?? ""} max={item.due_date ?? undefined} disabled={!can("item.edit")} onChange={(e) => patch({ start_date: e.target.value || null })} aria-label="Start date" />
              </dd>
              <dt>
                Due <Peers peers={on("due_date")} />
              </dt>
              <dd data-field="due_date">
                <input className="input" type="date" value={item.due_date ?? ""} min={item.start_date ?? undefined} disabled={!can("item.edit")} onChange={(e) => patch({ due_date: e.target.value || null })} aria-label="Due date" />
              </dd>
              {detail.cycles.length > 0 && (
                <>
                  <dt>
                    Cycle <Peers peers={on("cycle")} />
                  </dt>
                  <dd data-field="cycle">
                    <select className="select" value={item.cycle_id ?? ""} disabled={!can("item.edit")} onChange={(e) => patch({ cycle_id: e.target.value || null })} aria-label="Cycle">
                      <option value="">None</option>
                      {detail.cycles.map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.name}
                        </option>
                      ))}
                    </select>
                  </dd>
                </>
              )}
              <dt>
                Labels <Peers peers={on("labels")} />
              </dt>
              <dd data-field="labels">
                <input
                  className="input"
                  value={labels}
                  disabled={!can("item.edit")}
                  placeholder="Comma separated"
                  onChange={(e) => setLabels(e.target.value)}
                  onBlur={() => labels !== item.labels.join(", ") && patch({ labels: labels.split(",").map((l) => l.trim()).filter(Boolean) })}
                  onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
                  aria-label="Labels"
                />
              </dd>
              <dt>Reporter</dt>
              <dd>
                <PersonChip person={item.reporter} fallback="Unknown" />
              </dd>
              <dt>Created</dt>
              <dd className="small" title={relative(item.created_at)}>{dateTime(item.created_at)}</dd>
              <dt>Updated</dt>
              <dd className="small" title={relative(item.updated_at)}>{dateTime(item.updated_at)}</dd>
              {item.external_refs.map((r: any) => (
                <div key={r.external_id} style={{ display: "contents" }}>
                  <dt>Imported</dt>
                  <dd>
                    <a href={r.url} target="_blank" rel="noopener noreferrer">
                      {r.external_id}
                    </a>
                  </dd>
                </div>
              ))}
            </dl>
            <div className="watchers">
              <div className="row spread">
                <span className="small muted">
                  {Number(item.watchers ?? 0) === 1 ? "1 person follows this" : `${Number(item.watchers ?? 0)} people follow this`}
                </span>
                <button className="btn sm" aria-pressed={!!item.watching} onClick={() => run(() => api(item.watching ? "DELETE" : "POST", `${base}/watch`), item.watching ? "You will no longer be told about changes here" : "You will be told about changes here")}>
                  {item.watching ? <BellOff /> : <Bell />}
                  {item.watching ? "Stop following" : "Follow"}
                </button>
              </div>
            </div>
            {can("item.delete") && (
              <button className="btn danger sm block" style={{ marginTop: 16 }} onClick={() => setArchive(true)}>
                <Trash2 /> Archive item
              </button>
            )}
          </div>
        </aside>
      </div>
      {child && <NewItemDialog detail={detail} defaults={{ parent: item.key }} onClose={() => setChild(false)} />}
      {archive && (
        <Confirm
          title={`Archive ${item.key}?`}
          action="Archive item"
          danger
          body="It disappears from boards, lists and search. Its history is kept."
          onClose={() => setArchive(false)}
          onConfirm={async () => {
            await api("DELETE", base);
            toast(`${item.key} archived`);
            navigate(`/projects/${detail.project.key}`);
          }}
        />
      )}
    </>
  );
}

/** The same view of a work item, for showing beside a list without leaving the list. */
export function ItemPane({ itemKey }: { itemKey: string }) {
  const q = useGet<Item>(`/api/v1/items/${itemKey}`);
  const project = useProject(q.data?.project);
  if (q.error || project.error)
    return (
      <Empty title="This item is not available">
        <p>It may have been archived, or you may no longer have access to its project.</p>
      </Empty>
    );
  if (!q.data || !project.data) return <Loading what={`Opening ${itemKey}`} lines={6} />;
  return <ItemView key={q.data.id} item={q.data} detail={project.data} />;
}

export default function ItemPage() {
  const { key } = useParams();
  const q = useGet<Item>(`/api/v1/items/${key}`);
  const project = useProject(q.data?.project);
  const both = useMemo(() => ({ data: q.data && project.data ? { item: q.data, detail: project.data } : undefined, error: q.error ?? project.error, isPending: q.isPending || project.isPending }), [q.data, q.error, q.isPending, project.data, project.error, project.isPending]);
  return (
    <div className="page">
      <Loaded q={both}>{({ item, detail }) => <ItemView key={item.id} item={item} detail={detail} />}</Loaded>
      {q.error && (
        <Empty title="This item is not available">
          <p>It may have been archived, or you may not have access to its project.</p>
          <Link className="btn" to="/projects">
            Back to projects
          </Link>
        </Empty>
      )}
    </div>
  );
}
