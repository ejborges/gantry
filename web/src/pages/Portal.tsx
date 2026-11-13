// The help desk portal: where anyone raises a request and follows it. Customers see only this.
import { useEffect, useRef, useState } from "react";
import { Link, Route, Routes, useNavigate, useParams } from "react-router-dom";
import { ChevronRight, FileText, LifeBuoy, Paperclip, Upload } from "lucide-react";
import { api, upload, useGet, type FieldDef } from "../api";
import { CustomFieldInput, Dialog, Empty, Field, Loaded, Markdown, PersonChip, StatusBadge, useAction } from "../ui";
import { Discussion } from "../components/Discussion";
import { useLiveRefresh } from "../realtime";
import { NamedIcon } from "../icons";
import { bytes, dateTime, relative } from "../lib/format";

function PortalHome() {
  const desks = useGet<any[]>("/api/v1/portal/desks");
  const mine = useGet<any[]>("/api/v1/portal/requests");
  return (
    <div className="page narrow">
      <div className="page-head">
        <div>
          <h1>Help desk</h1>
          <p className="sub">Ask for help, report a problem, or follow a request you have already raised.</p>
        </div>
      </div>
      <div className="stack loose">
        <Loaded q={desks}>
          {(list) =>
            list.length === 0 ? (
              <div className="panel">
                <Empty icon={<LifeBuoy />} title="No help desk is open to you yet">
                  <p>Ask an administrator to give you access to a service desk.</p>
                </Empty>
              </div>
            ) : (
              <div className="card-grid">
                {list.map((d) => (
                  <Link key={d.key} to={`/portal/${d.key}`} className="card">
                    <h2 className="row">
                      <LifeBuoy width={18} height={18} /> {d.name}
                    </h2>
                    <p className="desc">{d.description || "Raise a request with this team."}</p>
                    <span className="btn primary sm" style={{ alignSelf: "flex-start", marginTop: 4 }}>
                      Raise a request
                    </span>
                  </Link>
                ))}
              </div>
            )
          }
        </Loaded>
        <section className="panel">
          <header>
            <h2>Your requests</h2>
          </header>
          <Loaded q={mine} lines={3}>
            {(list) =>
              list.length === 0 ? (
                <Empty title="You have not raised any requests">
                  <p>Requests you raise appear here with their current status.</p>
                </Empty>
              ) : (
                <div>
                  {list.map((r) => (
                    <Link key={r.key} to={`/portal/requests/${r.key}`} className="list-row">
                      <span className="key">{r.key}</span>
                      <span className="grow truncate">{r.title}</span>
                      <span className="small muted hide-sm">{relative(r.updated_at)}</span>
                      <StatusBadge name={r.status_name ?? r.status.replace(/_/g, " ")} category={r.status_category} />
                    </Link>
                  ))}
                </div>
              )
            }
          </Loaded>
        </section>
      </div>
    </div>
  );
}

function Desk() {
  const { desk: key } = useParams();
  const navigate = useNavigate();
  const q = useGet<{ desk: any; request_types: { id: string; name: string; description: string; icon: string; fields: FieldDef[] }[] }>(`/api/v1/portal/desks/${key}`);
  const [typeId, setTypeId] = useState<string | null>(null);
  const [summary, setSummary] = useState("");
  const [description, setDescription] = useState("");
  const [fields, setFields] = useState<Record<string, any>>({});
  const [debounced, setDebounced] = useState("");
  const [article, setArticle] = useState<string | null>(null);
  const [busy, run] = useAction();
  useEffect(() => {
    const t = setTimeout(() => setDebounced(summary.trim()), 350);
    return () => clearTimeout(t);
  }, [summary]);
  const suggestions = useGet<{ id: string; title: string; snippet: string }[]>(debounced.length >= 4 ? `/api/v1/portal/desks/${key}/suggest?q=${encodeURIComponent(debounced)}` : null);
  const opened = useGet<any>(article ? `/api/v1/portal/desks/${key}/articles/${article}` : null);

  return (
    <div className="page narrow">
      <Loaded q={q}>
        {(d) => {
          const type = d.request_types.find((t) => t.id === typeId);
          return (
            <>
              <div className="crumbs">
                <Link to="/portal">Help desk</Link>
                <ChevronRight />
                <span>{d.desk.name}</span>
              </div>
              <div className="page-head">
                <h1>{type ? type.name : "What do you need help with?"}</h1>
              </div>
              {!type ? (
                <div className="card-grid">
                  {d.request_types.map((t) => (
                    <button key={t.id} className="card" onClick={() => setTypeId(t.id)}>
                      <span className="type-icon blue" style={{ width: 30, height: 30 }}>
                        <NamedIcon name={t.icon} style={{ width: 17, height: 17 }} />
                      </span>
                      <h2>{t.name}</h2>
                      <p className="desc">{t.description}</p>
                    </button>
                  ))}
                  {d.request_types.length === 0 && <Empty title="This desk has no request types yet" />}
                </div>
              ) : (
                <form
                  className="stack"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const clean = Object.fromEntries(Object.entries(fields).filter(([, v]) => v !== null && v !== ""));
                    void run(() => api("POST", `/api/v1/portal/desks/${key}/requests`, { request_type_id: type.id, summary, description, fields: clean }), (r: any) => navigate(`/portal/requests/${r.key}`));
                  }}
                >
                  <Field label="Summary" required hint="One line that says what is wrong or what you need.">
                    <input className="input" autoFocus required maxLength={500} value={summary} onChange={(e) => setSummary(e.target.value)} />
                  </Field>
                  {suggestions.data && suggestions.data.length > 0 && (
                    <div className="notice ok" style={{ flexDirection: "column", gap: 6 }}>
                      <strong>These articles may already answer it</strong>
                      {suggestions.data.map((s) => (
                        <button type="button" key={s.id} className="btn ghost sm" style={{ justifyContent: "flex-start", height: "auto", padding: "4px 6px", whiteSpace: "normal", textAlign: "left" }} onClick={() => setArticle(s.id)}>
                          <FileText /> {s.title}
                        </button>
                      ))}
                    </div>
                  )}
                  <Field label="Details" hint="What happened, what you expected, and anything you have already tried.">
                    <textarea className="textarea" rows={6} value={description} onChange={(e) => setDescription(e.target.value)} />
                  </Field>
                  {type.fields.length > 0 && (
                    <div className="form-grid">
                      {type.fields.map((f) => (
                        <Field key={f.key} label={f.name} required={f.required} hint={f.description}>
                          <CustomFieldInput field={f} value={fields[f.key]} onChange={(v) => setFields((x) => ({ ...x, [f.key]: v }))} people={[]} />
                        </Field>
                      ))}
                    </div>
                  )}
                  <div className="row">
                    <button className="btn primary" disabled={busy || !summary.trim()}>
                      Send request
                    </button>
                    <button type="button" className="btn" onClick={() => setTypeId(null)}>
                      Choose a different type
                    </button>
                  </div>
                </form>
              )}
              {article && (
                <Dialog title={opened.data?.title ?? "Article"} wide onClose={() => setArticle(null)}>
                  <Loaded q={opened}>{(a) => <Markdown source={a.content} />}</Loaded>
                </Dialog>
              )}
            </>
          );
        }}
      </Loaded>
    </div>
  );
}

function Request() {
  const { key } = useParams();
  const q = useGet<any>(`/api/v1/portal/requests/${key}`, { refetchInterval: 60_000 });
  const [, run] = useAction();
  const fileRef = useRef<HTMLInputElement>(null);
  // Replies from the team appear as they are sent.
  useLiveRefresh([key ? `item:${key.toUpperCase()}` : null]);
  return (
    <div className="page narrow">
      <Loaded q={q}>
        {(r) => (
          <div className="stack">
            <div className="crumbs">
              <Link to="/portal">Help desk</Link>
              <ChevronRight />
              <span>{r.desk.name}</span>
            </div>
            <div className="page-head" style={{ marginBottom: 0 }}>
              <div>
                <h1>{r.title}</h1>
                <p className="sub row">
                  <span className="key">{r.key}</span> <span title={dateTime(r.created_at)}>raised {relative(r.created_at)}</span>
                </p>
              </div>
              <StatusBadge name={r.status_name} category={r.status_category} />
            </div>
            {r.status === "waiting_customer" && <div className="notice warn">The team is waiting for your reply.</div>}
            {r.description && (
              <section className="panel">
                <div className="body">
                  <Markdown source={r.description} attachments={r.attachments} compact />
                </div>
              </section>
            )}
            {r.assignee && (
              <p className="row small muted">
                Handled by <PersonChip person={r.assignee} />
              </p>
            )}
            <section>
              <div className="section-title">
                <h2>Conversation</h2>
                <input
                  ref={fileRef}
                  type="file"
                  hidden
                  multiple
                  onChange={async (e) => {
                    for (const f of Array.from(e.target.files ?? [])) await run(() => upload(`/api/v1/portal/requests/${key}/attachments`, f), `${f.name} attached`);
                    e.target.value = "";
                  }}
                />
                <button type="button" className="btn sm" onClick={() => fileRef.current?.click()}>
                  <Upload /> Attach files
                </button>
              </div>
              <Discussion
                messages={r.comments}
                postTo={`/api/v1/portal/requests/${key}/comments`}
                canPost
                uploadTo={`/api/v1/portal/requests/${key}/attachments`}
                attachments={r.attachments}
                room={`item:${r.key}`}
                placeholder="Add information or reply to the team"
                desk
              />
            </section>
            {r.attachments.length > 0 && (
              <section className="panel">
                {r.attachments.map((a: any) => (
                  <a key={a.id} className="list-row" href={`/api/v1/attachments/${a.id}`}>
                    <Paperclip width={15} height={15} />
                    <span className="grow truncate">{a.filename}</span>
                    <span className="small muted">{bytes(a.size)}</span>
                  </a>
                ))}
              </section>
            )}
          </div>
        )}
      </Loaded>
    </div>
  );
}

export default function Portal() {
  return (
    <Routes>
      <Route index element={<PortalHome />} />
      <Route path="requests/:key" element={<Request />} />
      <Route path=":desk" element={<Desk />} />
    </Routes>
  );
}
