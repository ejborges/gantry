import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { Link, Route, Routes, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { BookOpen, ChevronRight, Eye, FileText, History, LifeBuoy, Lock, Paperclip, Pencil, Plus, Settings, Trash2, Undo2, Upload, Users } from "lucide-react";
import { api, ApiError, upload, useGet } from "../api";
import { useSession } from "../session";
import { Avatar, Confirm, Dialog, Empty, Field, Loaded, Loading, Markdown, Peers, PersonName, StatusBadge, toast, useAction } from "../ui";
import { usePeople } from "../work";
import { bytes, dateTime, relative, stamp } from "../lib/format";
import { needsAdvancedEditor } from "../lib/markdown";
import SimpleEditor, { seedRich } from "../editors/Simple";
import MarkdownEditor from "../editors/MarkdownEditor";
import { useLiveRefresh, usePresence, useRoom, useSharedDoc } from "../realtime";

import { problems, type WorkflowDef } from "../lib/workflow";

const DiffView = lazy(() => import("../components/DiffView"));
const WorkflowEditor = lazy(() => import("../components/WorkflowEditor"));

interface PageNode {
  id: string;
  parent_id: string | null;
  title: string;
  status: string;
  customer_visible: boolean;
}
interface SpaceDetail {
  space: { id: string; key: string; name: string; description: string; kind: string; access: string; workflow?: any | null };
  permissions: string[];
  pages: PageNode[];
  members: any[];
  project: string | null;
}

function useSpace(key: string | undefined) {
  return useGet<SpaceDetail>(key ? `/api/v1/spaces/${key}` : null);
}

// ───────── spaces ─────────
function Spaces() {
  const { can } = useSession();
  const navigate = useNavigate();
  const spaces = useGet<any[]>("/api/v1/spaces");
  const [creating, setCreating] = useState(false);
  const [busy, run] = useAction();
  const [form, setForm] = useState({ name: "", key: "", kind: "wiki", access: "internal" });
  const keyOk = /^[A-Z][A-Z0-9]{1,9}$/.test(form.key);
  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Docs</h1>
          <p className="sub">Spaces hold pages: plans, procedures, designs, meeting notes and knowledge base articles.</p>
        </div>
        {can("space.create") && (
          <button className="btn primary" onClick={() => setCreating(true)}>
            <Plus /> New space
          </button>
        )}
      </div>
      <Loaded q={spaces}>
        {(list) =>
          list.length === 0 ? (
            <div className="panel">
              <Empty icon={<BookOpen />} title="No spaces yet">
                <p>Create a space for a team or subject. Service desk projects get a knowledge base space automatically.</p>
              </Empty>
            </div>
          ) : (
            <div className="card-grid">
              {list.map((s) => (
                <Link key={s.key} to={`/docs/${s.key}`} className="card">
                  <div className="spread">
                    <span className="row">
                      <span className="key">{s.key}</span>
                      {s.kind === "knowledge_base" && (
                        <span className="badge teal">
                          <LifeBuoy /> Knowledge base
                        </span>
                      )}
                    </span>
                    <span className="muted">{s.access === "private" ? <Lock width={14} height={14} /> : <Users width={14} height={14} />}</span>
                  </div>
                  <h2>{s.name}</h2>
                  {s.description && <p className="desc">{s.description}</p>}
                  <span className="small muted" style={{ marginTop: "auto" }}>
                    {s.pages} {s.pages === 1 ? "page" : "pages"}
                    {s.updated_at && `, updated ${relative(s.updated_at)}`}
                  </span>
                </Link>
              ))}
            </div>
          )
        }
      </Loaded>
      {creating && (
        <Dialog
          title="New space"
          onClose={() => setCreating(false)}
          footer={
            <>
              <button className="btn" onClick={() => setCreating(false)}>
                Cancel
              </button>
              <button className="btn primary" disabled={busy || !form.name.trim() || !keyOk} onClick={() => run(() => api("POST", "/api/v1/spaces", form), () => navigate(`/docs/${form.key}`))}>
                Create space
              </button>
            </>
          }
        >
          <div className="stack">
            <Field label="Name" required>
              <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="Engineering handbook" />
            </Field>
            <div className="form-grid">
              <Field label="Key" required hint="2 to 10 capital letters or digits.">
                <input className="input" value={form.key} maxLength={10} onChange={(e) => setForm({ ...form, key: e.target.value.toUpperCase() })} placeholder="ENG" />
              </Field>
              <Field label="Kind">
                <select className="select" value={form.kind} onChange={(e) => setForm({ ...form, kind: e.target.value })}>
                  <option value="wiki">Documentation</option>
                  <option value="knowledge_base">Knowledge base</option>
                </select>
              </Field>
            </div>
            <Field label="Who can read it">
              <select className="select" value={form.access} onChange={(e) => setForm({ ...form, access: e.target.value })}>
                <option value="internal">Everyone in the workspace</option>
                <option value="private">Only people I add</option>
              </select>
            </Field>
          </div>
        </Dialog>
      )}
    </div>
  );
}

// ───────── page tree ─────────
function Tree({ space, current }: { space: SpaceDetail; current?: string }) {
  const children = useMemo(() => {
    const m = new Map<string | null, PageNode[]>();
    for (const p of space.pages) m.set(p.parent_id, [...(m.get(p.parent_id) ?? []), p]);
    return m;
  }, [space.pages]);
  const known = new Set(space.pages.map((p) => p.id));
  const roots = space.pages.filter((p) => !p.parent_id || !known.has(p.parent_id));
  const branch = (nodes: PageNode[], depth: number): React.ReactNode =>
    nodes.map((p) => (
      <div key={p.id}>
        <Link to={`/docs/${space.space.key}/${p.id}`} className={p.id === current ? "active" : ""} style={{ paddingLeft: 8 + depth * 14 }} aria-current={p.id === current ? "page" : undefined}>
          <FileText />
          <span className="truncate grow">{p.title}</span>
          {p.status === "draft" && <span className="badge amber">Draft</span>}
        </Link>
        {children.get(p.id) && branch(children.get(p.id)!, depth + 1)}
      </div>
    ));
  return (
    <nav className="tree panel" aria-label={`Pages in ${space.space.name}`}>
      <div className="spread" style={{ padding: "4px 8px 8px" }}>
        <Link to={`/docs/${space.space.key}`} style={{ color: "var(--ink)", fontWeight: 600 }}>
          {space.space.name}
        </Link>
        {space.permissions.includes("page.edit") && (
          <Link className="btn ghost icon sm" to={`/docs/${space.space.key}/new${current ? `?parent=${current}` : ""}`} aria-label="New page" title="New page">
            <Plus />
          </Link>
        )}
      </div>
      {roots.length === 0 ? <p className="small muted" style={{ padding: 8 }}>No pages yet.</p> : branch(roots, 0)}
    </nav>
  );
}

function SpaceFrame({ children, current }: { children: (space: SpaceDetail) => React.ReactNode; current?: string }) {
  const { space: key } = useParams();
  const q = useSpace(key);
  return (
    <div className="page">
      <Loaded q={q}>
        {(space) => (
          <>
            <div className="crumbs">
              <Link to="/docs">Docs</Link>
              <ChevronRight />
              <Link to={`/docs/${space.space.key}`}>{space.space.name}</Link>
            </div>
            <div className="docs">
              <Tree space={space} current={current} />
              <div style={{ minWidth: 0 }}>{children(space)}</div>
            </div>
          </>
        )}
      </Loaded>
    </div>
  );
}

function SpaceHome() {
  return (
    <SpaceFrame>
      {(space) => (
        <div className="stack">
          <div className="page-head" style={{ marginBottom: 0 }}>
            <div>
              <h1>{space.space.name}</h1>
              {space.space.description && <p className="sub">{space.space.description}</p>}
            </div>
            <div className="row">
              {space.permissions.includes("space.admin") && (
                <Link className="btn" to={`/docs/${space.space.key}/settings`}>
                  <Settings /> Space settings
                </Link>
              )}
              {space.permissions.includes("page.edit") && (
                <Link className="btn primary" to={`/docs/${space.space.key}/new`}>
                  <Plus /> New page
                </Link>
              )}
            </div>
          </div>
          {space.space.kind === "knowledge_base" && (
            <div className="notice">
              <LifeBuoy />
              <span>This is a knowledge base. Pages marked “Shown in the help desk” are suggested to people while they write a request{space.project ? <> in <Link to={`/projects/${space.project}`}>{space.project}</Link></> : null}.</span>
            </div>
          )}
          {space.pages.length === 0 ? (
            <div className="panel">
              <Empty icon={<FileText />} title="This space has no pages yet">
                <p>Start from a blank page or a template such as meeting notes, a runbook or a design document.</p>
              </Empty>
            </div>
          ) : (
            <div className="panel">
              {space.pages.slice(0, 40).map((p) => (
                <Link key={p.id} to={`/docs/${space.space.key}/${p.id}`} className="list-row">
                  <FileText width={15} height={15} />
                  <span className="grow truncate">{p.title}</span>
                  {p.customer_visible && <span className="badge teal">Help desk</span>}
                  {p.status === "draft" && <span className="badge amber">Draft</span>}
                </Link>
              ))}
            </div>
          )}
        </div>
      )}
    </SpaceFrame>
  );
}

// ───────── page view ─────────
function PageView() {
  const { space: spaceKey, page: id } = useParams();
  const q = useGet<any>(`/api/v1/pages/${id}`);
  const navigate = useNavigate();
  const [busy, run] = useAction();
  const [archive, setArchive] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const room = id ? `page:${id}` : null;
  const { peers } = useRoom(room);
  useLiveRefresh([room, spaceKey ? `space:${spaceKey}` : null]);
  return (
    <SpaceFrame current={id}>
      {(space) => (
        <Loaded q={q}>
          {(page) => {
            const canEdit = page.permissions.includes("page.edit");
            const patch = (c: Record<string, unknown>, done: string) => run(() => api("PATCH", `/api/v1/pages/${id}`, c), done);
            return (
              <article className="stack">
                <div className="page-head" style={{ marginBottom: 0 }}>
                  <div className="grow">
                    {page.breadcrumbs.length > 0 && (
                      <div className="crumbs">
                        {page.breadcrumbs.map((b: any, i: number) => (
                          <span key={b.id} className="row" style={{ gap: 6 }}>
                            {i > 0 && <ChevronRight />}
                            <Link to={`/docs/${spaceKey}/${b.id}`}>{b.title}</Link>
                          </span>
                        ))}
                      </div>
                    )}
                    <h1 style={{ fontSize: 26 }}>{page.title}</h1>
                    <p className="sub row wrap small">
                      <span title={stamp(page.updated_at)} className="row" style={{ gap: 5 }}>
                        Updated {relative(page.updated_at)} by <PersonName person={page.updated_by_user} fallback={page.updated_by ?? "a former account"} />
                      </span>
                      <span>Version {page.version}</span>
                      {page.stage && <StatusBadge name={page.stage.name} category={page.status === "published" ? "done" : page.stage.key === page.stage.stages?.[0]?.key ? "todo" : "in_progress"} />}
                      {page.status === "draft" && <span className="badge amber">{page.stage ? "Not published yet: only editors can see it" : "Draft: only editors can see it"}</span>}
                      <Peers peers={peers} where={(st) => (st.focus === "editing" ? "editing" : null)} />
                      {page.customer_visible && (
                        <span className="badge teal">
                          <Eye /> Shown in the help desk
                        </span>
                      )}
                    </p>
                  </div>
                  <div className="row no-print">
                    <Link className="btn" to={`/docs/${spaceKey}/${id}/history`}>
                      <History /> History
                    </Link>
                    {canEdit && (
                      <Link className="btn primary" to={`/docs/${spaceKey}/${id}/edit`}>
                        <Pencil /> Edit
                      </Link>
                    )}
                  </div>
                </div>
                {canEdit && (
                  <div className="row wrap no-print">
                    {page.stage ? (
                      // The space defines the stages a page goes through; these are the steps open from here.
                      page.stage.next.map((n: any, i: number) => (
                        <button key={n.to} className={`btn sm ${i === 0 ? "primary" : ""}`} disabled={busy} onClick={() => patch({ stage: n.to }, `Moved to ${n.to_name ?? n.to}`)}>
                          {n.name}
                        </button>
                      ))
                    ) : (
                      <button className="btn sm" disabled={busy} onClick={() => patch({ status: page.status === "draft" ? "published" : "draft" }, page.status === "draft" ? "Published" : "Moved back to draft")}>
                        {page.status === "draft" ? "Publish" : "Move back to draft"}
                      </button>
                    )}
                    {space.space.kind === "knowledge_base" && (
                      <label className="check small">
                        <input type="checkbox" checked={page.customer_visible} onChange={(e) => patch({ customer_visible: e.target.checked }, e.target.checked ? "Now shown in the help desk" : "Hidden from the help desk")} /> Show in the help desk portal
                      </label>
                    )}
                    <span className="grow" />
                    {page.permissions.includes("page.delete") && (
                      <button className="btn sm danger" onClick={() => setArchive(true)}>
                        <Trash2 /> Archive
                      </button>
                    )}
                  </div>
                )}
                <Markdown source={page.content} refs={page.refs} attachments={page.attachments} empty={canEdit ? "This page is empty. Choose Edit to start writing." : "This page is empty."} />
                {(page.attachments.length > 0 || canEdit) && (
                  <section className="no-print">
                    <div className="section-title">
                      <h2>Files</h2>
                      {canEdit && (
                        <>
                          <input
                            ref={fileRef}
                            type="file"
                            hidden
                            multiple
                            onChange={async (e) => {
                              for (const f of Array.from(e.target.files ?? [])) await run(() => upload(`/api/v1/pages/${id}/attachments`, f), `${f.name} attached`);
                              e.target.value = "";
                            }}
                          />
                          <button className="btn sm" onClick={() => fileRef.current?.click()}>
                            <Upload /> Attach files
                          </button>
                        </>
                      )}
                    </div>
                    {page.attachments.map((a: any) => (
                      <a key={a.id} className="list-row" href={`/api/v1/attachments/${a.id}`}>
                        <Paperclip width={15} height={15} />
                        <span className="grow truncate">{a.filename}</span>
                        <span className="small muted">{bytes(a.size)}</span>
                      </a>
                    ))}
                  </section>
                )}
                {page.backlinks.length > 0 && (
                  <section className="no-print">
                    <div className="section-title">
                      <h2>Work items that link here</h2>
                    </div>
                    <div className="panel">
                      {page.backlinks.map((b: any) => (
                        <Link key={b.key + b.rel} to={`/i/${b.key}`} className="list-row">
                          <span className={`status-dot ${b.status_category}`} />
                          <span className="key">{b.key}</span>
                          <span className="grow truncate">{b.title}</span>
                          <span className="badge outline">{b.rel.replace(/_/g, " ")}</span>
                        </Link>
                      ))}
                    </div>
                  </section>
                )}
                {archive && (
                  <Confirm
                    title={`Archive “${page.title}”?`}
                    action="Archive page"
                    danger
                    body="The page and any pages beneath it disappear from the space and from search. Their history is kept."
                    onClose={() => setArchive(false)}
                    onConfirm={async () => {
                      await api("DELETE", `/api/v1/pages/${id}`);
                      toast("Page archived");
                      navigate(`/docs/${spaceKey}`);
                    }}
                  />
                )}
              </article>
            );
          }}
        </Loaded>
      )}
    </SpaceFrame>
  );
}

// ───────── page editor ─────────
function Editor({ space, page }: { space: SpaceDetail; page: any | null }) {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const templates = useGet<any[]>(page ? null : "/api/v1/page-templates");
  const [title, setTitle] = useState(page?.title ?? "");
  const [content, setContent] = useState<string>(page?.content ?? "");
  const [message, setMessage] = useState("");
  const blocker = needsAdvancedEditor(content);
  const [mode, setMode] = useState<"simple" | "advanced">(page?.editor === "advanced" || needsAdvancedEditor(page?.content ?? "") ? "advanced" : "simple");
  const [busy, setBusy] = useState(false);
  const [conflict, setConflict] = useState("");
  const [editorKey, setEditorKey] = useState(0);
  const [review, setReview] = useState(false);
  const { me } = useSession();
  const people = usePeople();
  // An existing page is edited together with whoever else has it open, in the editor the
  // page was last saved with. Changing editors continues alone until the page is saved.
  const startMode = useRef(mode).current;
  const [together, setTogether] = useState(!!page);
  const room = page ? `page:${page.id}` : "";
  const collab = useSharedDoc(room, "content", together && !!page, me.user, startMode === "simple" ? "rich" : "text", (d) => (startMode === "simple" ? seedRich(d, page?.content ?? "") : d.getText("t").insert(0, page?.content ?? "")), (mine) => {
    if (!mine) toast("The unsaved changes to this page were discarded, or the page was saved from somewhere else. Reload to continue from the saved version.", "error");
    setTogether(false);
  });
  const [discarding, setDiscarding] = useState(false);
  const { peers } = useRoom(page ? room : null);
  usePresence(page ? room : null, { focus: "editing" });
  const others = peers.filter((p) => p.state.focus === "editing");
  const flowing = !!space.space.workflow;
  const dirty = title !== (page?.title ?? "") || content.trim() !== (page?.content ?? "").trim();
  useEffect(() => {
    if (!dirty) return;
    const warn = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [dirty]);

  async function applyTemplate(key: string) {
    if (!key) return;
    const t = await api("GET", `/api/v1/page-templates/${key}`);
    setContent(t.body);
    if (!title.trim() && key !== "blank") setTitle(t.name);
    if (needsAdvancedEditor(t.body)) setMode("advanced");
    setEditorKey((k) => k + 1);
  }
  async function save(status?: string) {
    setBusy(true);
    setConflict("");
    try {
      const saved = page
        ? await api("PATCH", `/api/v1/pages/${page.id}`, { title, content, base_version: page.version, message, editor: mode, ...(collab.shared ? { collab: true } : {}) })
        : await api("POST", `/api/v1/spaces/${space.space.key}/pages`, { title, content, parent_id: params.get("parent") ?? undefined, editor: mode, status });
      toast(page ? "Page saved" : status === "draft" ? "Draft saved" : "Page published");
      // Leave without the unsaved-changes prompt: the content is now stored.
      window.onbeforeunload = null;
      navigate(`/docs/${space.space.key}/${saved.id}`, { replace: true });
      // Refresh in the background so the tree and view show the new version.
      void import("../api").then((m) => m.refresh());
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) setConflict(e.message);
      else toast(e instanceof Error ? e.message : "The page could not be saved.", "error");
      // Back to the editor, where the explanation is and the text still stands.
      setReview(false);
      setBusy(false);
    }
  }
  const byCategory = useMemo(() => {
    const m = new Map<string, any[]>();
    for (const t of templates.data ?? []) m.set(t.category, [...(m.get(t.category) ?? []), t]);
    return [...m.entries()];
  }, [templates.data]);

  return (
    <div className="stack">
      <div className="spread wrap" style={{ flexWrap: "wrap" }}>
        <h1>{page ? "Edit page" : "New page"}</h1>
        <div className="row wrap">
          <div className="mode-switch" role="group" aria-label="Editor">
            <button type="button" aria-pressed={mode === "simple"} disabled={!!blocker || (others.length > 0 && mode !== "simple")} title={blocker ? `This page contains ${blocker}, which only the advanced editor can keep.` : others.length > 0 && mode !== "simple" ? "Others are editing this page in the advanced editor right now." : "Rich text"} onClick={() => switchMode("simple")}>
              Simple
            </button>
            <button type="button" aria-pressed={mode === "advanced"} disabled={others.length > 0 && mode !== "advanced"} onClick={() => switchMode("advanced")} title={others.length > 0 && mode !== "advanced" ? "Others are editing this page in the simple editor right now." : "Markdown, with the result beside it"}>
              Advanced
            </button>
          </div>
          <Link className="btn" to={page ? `/docs/${space.space.key}/${page.id}` : `/docs/${space.space.key}`}>
            Cancel
          </Link>
          <Peers peers={others} where={() => "editing"} />
          {!page && (
            <button className={`btn ${flowing ? "primary" : ""}`} disabled={busy || !title.trim()} onClick={() => save("draft")}>
              {flowing ? "Create page" : "Save as draft"}
            </button>
          )}
          {page ? (
            <button className="btn primary" disabled={busy || !title.trim() || !dirty || !collab.ready} onClick={() => setReview(true)}>
              Review and save
            </button>
          ) : (
            !flowing && (
              <button className="btn primary" disabled={busy || !title.trim()} onClick={() => save("published")}>
                Publish page
              </button>
            )
          )}
        </div>
      </div>
      {conflict && (
        <div className="notice danger" role="alert">
          <span>
            {conflict} Your text is still here. <a href={`/docs/${space.space.key}/${page?.id}`} target="_blank" rel="noreferrer">Open the latest version in a new tab</a>, copy over what you need, then reload this editor.
          </span>
        </div>
      )}
      {blocker && mode === "advanced" && <p className="small muted">This page contains {blocker}. The simple editor is turned off for it so nothing is lost.</p>}
      <div className="form-grid" style={{ gridTemplateColumns: page ? "minmax(0, 2fr) minmax(0, 1fr)" : "minmax(0, 2fr) minmax(0, 1fr)" }}>
        <Field label="Title" required>
          <input className="input" value={title} maxLength={300} onChange={(e) => setTitle(e.target.value)} autoFocus={!page} style={{ fontSize: 16, fontWeight: 500 }} />
        </Field>
        {page ? (
          <Field label="What changed" hint="Shown in the page history.">
            <input className="input" value={message} maxLength={300} onChange={(e) => setMessage(e.target.value)} placeholder="Optional" />
          </Field>
        ) : (
          <Field label="Template">
            <select className="select" defaultValue="" onChange={(e) => void applyTemplate(e.target.value)}>
              <option value="">Blank page</option>
              {byCategory.map(([cat, list]) => (
                <optgroup key={cat} label={cat}>
                  {list.filter((t) => t.key !== "blank").map((t) => (
                    <option key={t.key} value={t.key}>
                      {t.name}
                    </option>
                  ))}
                </optgroup>
              ))}
            </select>
          </Field>
        )}
      </div>
      {page && collab.shared && <p className="small muted">Everyone with this page open in the editor sees the same text as it is typed. Nothing is published until someone saves.</p>}
      {page && collab.shared && dirty && (
        <p className="small muted row wrap" style={{ gap: 8 }}>
          <span>There are changes that are not saved. They stay here for whoever opens the editor next, until someone saves or discards them.</span>
          <button type="button" className="btn sm" disabled={others.length > 0} title={others.length > 0 ? "Others are editing this page right now." : `Return to version ${page.version}, the saved one`} onClick={() => setDiscarding(true)}>
            <Undo2 /> Discard changes
          </button>
        </p>
      )}
      {discarding && page && (
        <Confirm
          title="Discard the unsaved changes?"
          body={
            <p>
              Everything typed since version {page.version} was saved is removed, whoever typed it. The saved page is not affected. This is recorded in the audit log.
            </p>
          }
          action="Discard changes"
          danger
          onClose={() => setDiscarding(false)}
          onConfirm={() => {
            collab.shared?.discard();
            window.onbeforeunload = null;
            toast("Unsaved changes discarded");
            navigate(`/docs/${space.space.key}/${page.id}`, { replace: true });
          }}
        />
      )}
      {page && !together && <p className="small muted">You are editing on your own. Others will see your changes when you save.</p>}
      {page && collab.alone && together && <p className="small muted">The live connection is not available, so you are editing on your own. Saving works as usual.</p>}
      {flowing && !page && <p className="small muted">This space sends pages through stages. A new page starts at the first stage and is published when it reaches the last.</p>}
      {!collab.ready && page ? (
        <Loading lines={6} what="Joining the others who have this page open" />
      ) : mode === "simple" ? (
        <SimpleEditor key={`s${editorKey}${collab.shared ? "t" : ""}`} value={content} onChange={setContent} shared={collab.shared} me={me.user} />
      ) : (
        <MarkdownEditor key={`a${editorKey}${collab.shared ? "t" : ""}`} value={content} onChange={setContent} label="Page content" shared={collab.shared} storageKey="page" uploadTo={page ? `/api/v1/pages/${page.id}/attachments` : undefined} attachments={page?.attachments} people={people} />
      )}
      {review && page && (
        <Dialog
          title="Review your changes"
          wide
          onClose={() => setReview(false)}
          footer={
            <>
              <button className="btn" onClick={() => setReview(false)}>
                Keep editing
              </button>
              <button className="btn primary" disabled={busy} onClick={() => void save()}>
                {page.status === "published" ? "Save and publish this version" : "Save page"}
              </button>
            </>
          }
        >
          <div className="stack">
            {title !== page.title && (
              <p>
                Title: <del>{page.title}</del> → <ins>{title}</ins>
              </p>
            )}
            <Field label="What changed" hint="Shown in the page history.">
              <input className="input" value={message} maxLength={300} onChange={(e) => setMessage(e.target.value)} placeholder="Optional" />
            </Field>
            <Suspense fallback={<Loading lines={4} what="Comparing the versions" />}>
              <DiffView before={page.content ?? ""} after={content} beforeLabel={`Version ${page.version} (saved)`} afterLabel="Your version" />
            </Suspense>
            {page.status === "published" && <p className="small muted">This page is published: readers see the new version as soon as it is saved.</p>}
          </div>
        </Dialog>
      )}
    </div>
  );
  function switchMode(next: "simple" | "advanced") {
    if (next === mode) return;
    // The two editors keep different shared documents, so a switch leaves the shared session.
    if (page) setTogether(false);
    setMode(next);
    setEditorKey((k) => k + 1);
  }
}

function PageEdit({ isNew }: { isNew?: boolean }) {
  const { page: id } = useParams();
  const q = useGet<any>(isNew ? null : `/api/v1/pages/${id}`, { staleTime: 0, refetchOnWindowFocus: false });
  return <SpaceFrame current={id}>{(space) => (isNew ? <Editor space={space} page={null} /> : <Loaded q={q}>{(page) => <Editor key={page.id} space={space} page={page} />}</Loaded>)}</SpaceFrame>;
}

// ───────── history ─────────
function PageHistory() {
  const { space: spaceKey, page: id } = useParams();
  const page = useGet<any>(`/api/v1/pages/${id}`);
  const versions = useGet<any[]>(`/api/v1/pages/${id}/versions`);
  const [viewing, setViewing] = useState<number | null>(null);
  const one = useGet<any>(viewing ? `/api/v1/pages/${id}/versions/${viewing}` : null);
  const [compare, setCompare] = useState<number | null>(null);
  const older = useGet<any>(compare ? `/api/v1/pages/${id}/versions/${compare}` : null);
  const newer = useGet<any>(compare ? `/api/v1/pages/${id}/versions/${compare + 1}` : null, { retry: false });
  const [busy, run] = useAction();
  const navigate = useNavigate();
  return (
    <SpaceFrame current={id}>
      {() => (
        <div className="stack">
          <div className="spread">
            <h1>History of “{page.data?.title ?? "…"}”</h1>
            <Link className="btn" to={`/docs/${spaceKey}/${id}`}>
              Back to page
            </Link>
          </div>
          <div className="panel">
            <Loaded q={versions}>
              {(list) => (
                <div className="table-wrap">
                  <table className="table">
                    <thead>
                      <tr>
                        <th>Version</th>
                        <th>Saved</th>
                        <th>By</th>
                        <th className="hide-sm">Note</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {list.map((v, i) => (
                        <tr key={v.version}>
                          <td className="num">
                            {v.version} {i === 0 && <span className="badge green">Current</span>}
                          </td>
                          <td title={relative(v.created_at)}>{stamp(v.created_at)}</td>
                          <td>
                            <PersonName person={v.author_user} fallback={v.author ?? "Former account"} />
                          </td>
                          <td className="hide-sm muted">{v.message}</td>
                          <td style={{ textAlign: "right" }} className="nowrap">
                            <button className="btn sm" onClick={() => setViewing(v.version)}>
                              View
                            </button>{" "}
                            {v.version > 1 && (
                              <button className="btn sm" onClick={() => setCompare(v.version - 1)} title={`What version ${v.version} changed`}>
                                Changes
                              </button>
                            )}{" "}
                            {i > 0 && page.data?.permissions.includes("page.edit") && (
                              <button className="btn sm" disabled={busy} onClick={() => run(() => api("POST", `/api/v1/pages/${id}/versions/${v.version}/restore`), () => (toast(`Version ${v.version} restored as the newest version`), navigate(`/docs/${spaceKey}/${id}`)))}>
                                Restore
                              </button>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </Loaded>
          </div>
          {compare && (
            <Dialog title={`What version ${compare + 1} changed`} wide onClose={() => setCompare(null)}>
              {older.data && (newer.data || newer.error) ? (
                <Suspense fallback={<Loading lines={4} what="Comparing the versions" />}>
                  <DiffView before={older.data.content ?? ""} after={(newer.data ?? page.data)?.content ?? ""} beforeLabel={`Version ${compare}`} afterLabel={`Version ${compare + 1}`} />
                </Suspense>
              ) : (
                <Loading lines={4} what="Loading the two versions" />
              )}
            </Dialog>
          )}
          {viewing && (
            <Dialog title={`Version ${viewing}`} wide onClose={() => setViewing(null)}>
              <Loaded q={one}>
                {(v) => (
                  <div className="stack">
                    <h2>{v.title}</h2>
                    <Markdown source={v.content} empty="This version was empty." />
                  </div>
                )}
              </Loaded>
            </Dialog>
          )}
        </div>
      )}
    </SpaceFrame>
  );
}

// ───────── space settings ─────────
const REVIEW_FLOW: WorkflowDef = {
  initial: "draft",
  statuses: [
    { key: "draft", name: "Draft", category: "todo" },
    { key: "in_review", name: "In review", category: "in_progress" },
    { key: "published", name: "Published", category: "done" },
  ],
  transitions: [
    { name: "Send for review", from: ["draft"], to: "in_review" },
    { name: "Send back", from: ["in_review"], to: "draft" },
    { name: "Publish", from: ["in_review"], to: "published" },
    { name: "Revise", from: ["published"], to: "draft" },
  ],
};

/** The stages a page goes through in this space, drawn with the same editor as work item workflows. */
function PageWorkflow({ spaceKey, workflow }: { spaceKey: string; workflow: any | null }) {
  const [editing, setEditing] = useState(false);
  const [busy, run] = useAction();
  const start = useMemo<WorkflowDef>(() => (workflow ? { initial: workflow.initial, statuses: workflow.statuses, transitions: workflow.transitions, layout: workflow.layout ?? null } : REVIEW_FLOW), [workflow]);
  const [def, setDef] = useState<WorkflowDef>(start);
  const issues = problems(def);
  const save = (value: WorkflowDef | null, done: string) => run(() => api("PATCH", `/api/v1/spaces/${spaceKey}`, { workflow: value }), done).then((r) => r !== undefined && setEditing(false));
  return (
    <section className="panel">
      <header>
        <h2>Stages a page goes through</h2>
        <span className="small muted">{workflow ? workflow.statuses.map((s: any) => s.name).join(" → ") : "Draft → Published"}</span>
      </header>
      <div className="body stack">
        <p className="small muted" style={{ margin: 0 }}>
          {workflow
            ? "Pages in this space move through these stages. A page is visible to readers once it is in a stage that counts as done; before that only editors see it."
            : "Pages in this space are either a draft or published. Define stages to add steps such as a review before publishing."}
        </p>
        <div className="row wrap">
          <button className="btn sm" onClick={() => (setDef(start), setEditing(true))}>
            <Pencil /> {workflow ? "Edit the stages" : "Define stages"}
          </button>
          {workflow && (
            <button className="btn ghost sm" disabled={busy} onClick={() => save(null, "Pages are drafts or published again")}>
              Go back to draft and published
            </button>
          )}
        </div>
      </div>
      {editing && (
        <Dialog
          wide="canvas"
          title="Stages a page goes through"
          onClose={() => setEditing(false)}
          footer={
            <>
              <button className="btn" onClick={() => setEditing(false)}>Cancel</button>
              <button className="btn primary" disabled={busy || issues.length > 0} onClick={() => save(def, "Stages saved")}>
                Save stages
              </button>
            </>
          }
        >
          <div className="stack">
            <p className="small muted" style={{ margin: 0 }}>
              Stages that count as done are the published ones. If you remove a stage, published pages in it move to the first published stage and the others to the start.
            </p>
            <Suspense fallback={<Loading what="Opening the workflow editor" />}>
              <WorkflowEditor value={start} onChange={setDef} noun="stage" />
            </Suspense>
          </div>
        </Dialog>
      )}
    </section>
  );
}

function SpaceSettings() {
  const people = usePeople();
  const cat = useGet<{ roles: any[]; groups: any[] }>("/api/v1/catalog");
  const [busy, run] = useAction();
  const [who, setWho] = useState("");
  const [role, setRole] = useState("space_editor");
  return (
    <SpaceFrame>
      {(space) => {
        const base = `/api/v1/spaces/${space.space.key}`;
        if (!space.permissions.includes("space.admin")) return <Empty title="Only space administrators can change these settings" />;
        return (
          <div className="stack loose" style={{ maxWidth: 760 }}>
            <h1>Space settings</h1>
            <section className="panel">
              <div className="body form-grid">
                <Field label="Who can read it">
                  <select className="select" value={space.space.access} onChange={(e) => run(() => api("PATCH", base, { access: e.target.value }), "Saved")}>
                    <option value="internal">Everyone in the workspace</option>
                    <option value="private">Only members listed below</option>
                  </select>
                </Field>
                <Field label="Kind">
                  <select className="select" value={space.space.kind} onChange={(e) => run(() => api("PATCH", base, { kind: e.target.value }), "Saved")}>
                    <option value="wiki">Documentation</option>
                    <option value="knowledge_base">Knowledge base</option>
                  </select>
                </Field>
              </div>
            </section>
            <PageWorkflow spaceKey={space.space.key} workflow={space.space.workflow ?? null} />
            <section className="panel">
              <header>
                <h2>Members</h2>
              </header>
              {space.members.map((m) => (
                <div key={m.binding_id} className="list-row">
                  {m.user ? <Avatar person={m.user} /> : <span className="badge outline">Group</span>}
                  <span className="grow">{m.user?.name ?? m.group?.name}</span>
                  <span className="muted">{m.role_name}</span>
                  <button className="btn ghost icon sm" aria-label={`Remove ${m.user?.name ?? m.group?.name}`} onClick={() => run(() => api("DELETE", `${base}/members/${m.binding_id}`), "Member removed")}>
                    <Trash2 />
                  </button>
                </div>
              ))}
              <div className="body row wrap" style={{ borderTop: "1px solid var(--line)" }}>
                <select className="select" style={{ width: "auto", minWidth: 200 }} value={who} onChange={(e) => setWho(e.target.value)} aria-label="Person or group">
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
                  {(cat.data?.roles ?? []).filter((r) => r.scope === "space").map((r) => (
                    <option key={r.key} value={r.key}>
                      {r.name}
                    </option>
                  ))}
                </select>
                <button className="btn" disabled={!who || busy} onClick={() => run(() => api("POST", `${base}/members`, { [who.startsWith("u:") ? "user_id" : "group_id"]: who.slice(2), role }), "Member added").then(() => setWho(""))}>
                  <Plus /> Add member
                </button>
              </div>
            </section>
          </div>
        );
      }}
    </SpaceFrame>
  );
}

export default function Docs() {
  return (
    <Routes>
      <Route index element={<Spaces />} />
      <Route path=":space" element={<SpaceHome />} />
      <Route path=":space/new" element={<PageEdit isNew />} />
      <Route path=":space/settings" element={<SpaceSettings />} />
      <Route path=":space/:page" element={<PageView />} />
      <Route path=":space/:page/edit" element={<PageEdit />} />
      <Route path=":space/:page/history" element={<PageHistory />} />
    </Routes>
  );
}
