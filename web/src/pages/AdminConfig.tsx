// Administration: integrations, work item types / fields / workflows, and add-on packs.
import { lazy, Suspense, useMemo, useState } from "react";
import { ArrowRight, Plug, Plus, Puzzle, Trash2, X } from "lucide-react";
import { api, useGet, type Status, type Transition } from "../api";
import { Confirm, CopyButton, Dialog, Empty, Field, Loaded, Loading, PersonSelect, StatusBadge, TypeIcon, toast, useAction } from "../ui";
import { ICON_NAMES } from "../icons";
import { problems, type WorkflowDef } from "../lib/workflow";

const VisualWorkflow = lazy(() => import("../components/WorkflowEditor"));
import { dateTime, relative, titleCase } from "../lib/format";

function Head({ title, sub, children }: { title: string; sub?: string; children?: React.ReactNode }) {
  return (
    <div className="page-head">
      <div>
        <h1>{title}</h1>
        {sub && <p className="sub">{sub}</p>}
      </div>
      {children && <div className="row">{children}</div>}
    </div>
  );
}

// ───────── integrations ─────────
interface Kind {
  kind: string;
  name: string;
  direction: string;
  description: string;
  config: { key: string; label: string; required: boolean }[];
  secrets: { key: string; label: string; required: boolean }[];
}
const SENDS = ["slack", "teams", "webhook"];

export function Integrations() {
  const q = useGet<{ integrations: any[]; kinds: Kind[]; events: { key: string; description: string }[] }>("/api/v1/admin/integrations");
  const projects = useGet<any[]>("/api/v1/projects");
  const people = useGet<any[]>("/api/v1/users").data ?? [];
  const [busy, run] = useAction();
  const [kind, setKind] = useState<Kind | null>(null);
  const [picking, setPicking] = useState(false);
  const [form, setForm] = useState<{ name: string; config: Record<string, string>; secrets: Record<string, string>; events: string[]; project: string }>({ name: "", config: {}, secrets: {}, events: [], project: "" });
  const [secret, setSecret] = useState<any | null>(null);
  const [log, setLog] = useState<any | null>(null);
  const deliveries = useGet<any[]>(log ? `/api/v1/admin/integrations/${log.id}/deliveries` : null);
  const [jira, setJira] = useState<any | null>(null);
  const [imp, setImp] = useState({ project: "", jql: "", max: "1000" });
  const [remove, setRemove] = useState<any | null>(null);
  const start = (k: Kind) => (setPicking(false), setForm({ name: k.name, config: {}, secrets: {}, events: SENDS.includes(k.kind) ? ["item.created", "item.transitioned", "comment.created"] : [], project: "" }), setKind(k));

  return (
    <>
      <Head title="Integrations" sub="Connect chat, code hosting and other trackers. Secrets are stored encrypted and never shown again.">
        <button className="btn primary" onClick={() => setPicking(true)}>
          <Plus /> Add integration
        </button>
      </Head>
      <Loaded q={q}>
        {(d) => (
          <div className="stack">
            {d.integrations.length === 0 && (
              <div className="panel">
                <Empty icon={<Plug />} title="Nothing is connected yet">
                  <p>Send updates to Slack or Microsoft Teams, link commits and merge requests from GitHub or GitLab, import from Jira, or post signed events to any system with a webhook.</p>
                </Empty>
              </div>
            )}
            {d.integrations.map((i) => {
              const k = d.kinds.find((x) => x.kind === i.kind);
              const canSend = SENDS.includes(i.kind) && (i.kind !== "slack" || i.secrets_set.includes("webhook_url"));
              return (
                <section key={i.id} className="panel">
                  <header>
                    <h2 className="row wrap">
                      {i.name} <span className="badge outline">{k?.name ?? i.kind}</span> {i.enabled ? <span className="badge green">On</span> : <span className="badge">Off</span>}
                      {i.project && <span className="badge blue">Only {i.project}</span>}
                    </h2>
                    <span className="row wrap">
                      {canSend && (
                        <button className="btn sm" disabled={busy} onClick={() => run(() => api("POST", `/api/v1/admin/integrations/${i.id}/test`), (r: any) => toast(r.ok ? "Test message delivered" : `Test failed: ${r.error}`, r.ok ? "info" : "error"))}>
                          Send test
                        </button>
                      )}
                      {i.kind === "jira" && (
                        <button className="btn sm" onClick={() => (setImp({ project: "", jql: "", max: "1000" }), setJira(i))}>
                          Import issues
                        </button>
                      )}
                      <button className="btn sm" onClick={() => setLog(i)}>
                        Activity
                      </button>
                      <button className="btn sm" disabled={busy} onClick={() => run(() => api("PATCH", `/api/v1/admin/integrations/${i.id}`, { enabled: !i.enabled }), i.enabled ? "Switched off" : "Switched on")}>
                        {i.enabled ? "Turn off" : "Turn on"}
                      </button>
                      <button className="btn ghost icon sm" aria-label={`Delete ${i.name}`} onClick={() => setRemove(i)}>
                        <Trash2 />
                      </button>
                    </span>
                  </header>
                  <div className="body stack tight">
                    <span className="muted">{k?.description}</span>
                    {i.events.length > 0 && <span className="small">Sends: {i.events.includes("*") ? "every event" : i.events.join(", ")}</span>}
                    {i.inbound_url && (
                      <span className="row small">
                        <span className="muted nowrap">{i.kind === "slack" ? "Slash command URL" : "Webhook URL to enter there"}:</span> <code className="truncate">{i.inbound_url}</code> <CopyButton text={i.inbound_url} />
                      </span>
                    )}
                    {i.last_delivery && (
                      <span className="small muted">
                        Last activity {relative(i.last_delivery.at)}: {i.last_delivery.event}, {i.last_delivery.outcome === "ok" ? "delivered" : i.last_delivery.outcome}
                      </span>
                    )}
                  </div>
                </section>
              );
            })}
            {picking && (
              <Dialog title="Add an integration" wide onClose={() => setPicking(false)}>
                <div className="card-grid" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))" }}>
                  {d.kinds.map((k) => (
                    <button key={k.kind} className="card" onClick={() => start(k)}>
                      <strong>{k.name}</strong>
                      <span className="desc small">{k.description}</span>
                    </button>
                  ))}
                </div>
              </Dialog>
            )}
            {kind && (
              <Dialog
                title={`Connect ${kind.name}`}
                wide
                onClose={() => setKind(null)}
                footer={
                  <>
                    <button className="btn" onClick={() => setKind(null)}>
                      Cancel
                    </button>
                    <button
                      className="btn primary"
                      disabled={busy || !form.name.trim()}
                      onClick={() =>
                        run(
                          () => api("POST", "/api/v1/admin/integrations", { kind: kind.kind, name: form.name, config: Object.fromEntries(Object.entries(form.config).filter(([, v]) => v)), secrets: Object.fromEntries(Object.entries(form.secrets).filter(([, v]) => v)), events: form.events, project: form.project || undefined }),
                          (r: any) => (setKind(null), r.generated_signing_secret || r.inbound_url ? setSecret(r) : toast("Integration added")),
                        )
                      }
                    >
                      Add integration
                    </button>
                  </>
                }
              >
                <div className="stack">
                  <Field label="Name" required>
                    <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
                  </Field>
                  <div className="form-grid">
                    {kind.config.map((c) => (
                      <Field key={c.key} label={c.label} required={c.required}>
                        {c.key === "acting_user_id" ? (
                          <PersonSelect people={people} value={form.config[c.key] ?? null} onChange={(v) => setForm({ ...form, config: { ...form.config, [c.key]: v ?? "" } })} none="Choose an account" />
                        ) : c.key === "deployment" ? (
                          <select className="select" value={form.config[c.key] ?? ""} onChange={(e) => setForm({ ...form, config: { ...form.config, [c.key]: e.target.value } })}>
                            <option value="">Choose</option>
                            <option value="cloud">Jira Cloud</option>
                            <option value="datacenter">Jira Data Center</option>
                          </select>
                        ) : c.key === "auto_transition" ? (
                          <select className="select" value={form.config[c.key] ?? ""} onChange={(e) => setForm({ ...form, config: { ...form.config, [c.key]: e.target.value } })}>
                            <option value="">No</option>
                            <option value="true">Yes</option>
                          </select>
                        ) : (
                          <input className="input" value={form.config[c.key] ?? ""} onChange={(e) => setForm({ ...form, config: { ...form.config, [c.key]: e.target.value } })} />
                        )}
                      </Field>
                    ))}
                    {kind.secrets.map((c) => (
                      <Field key={c.key} label={c.label} required={c.required}>
                        <input className="input" type="password" autoComplete="off" value={form.secrets[c.key] ?? ""} onChange={(e) => setForm({ ...form, secrets: { ...form.secrets, [c.key]: e.target.value } })} />
                      </Field>
                    ))}
                  </div>
                  {SENDS.includes(kind.kind) && (
                    <div className="stack tight">
                      <span className="label">Send a message when</span>
                      <div className="perm-grid">
                        {d.events.map((e) => (
                          <label key={e.key} className="check">
                            <input type="checkbox" checked={form.events.includes(e.key)} onChange={(ev) => setForm({ ...form, events: ev.target.checked ? [...form.events, e.key] : form.events.filter((x) => x !== e.key) })} />
                            <span className="small">{e.description}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  )}
                  <Field label="Limit to one project" hint="Leave on “All projects” to cover everything. Internal service desk notes are never sent anywhere.">
                    <select className="select" value={form.project} onChange={(e) => setForm({ ...form, project: e.target.value })}>
                      <option value="">All projects</option>
                      {(projects.data ?? []).map((p) => (
                        <option key={p.key} value={p.key}>
                          {p.name}
                        </option>
                      ))}
                    </select>
                  </Field>
                </div>
              </Dialog>
            )}
          </div>
        )}
      </Loaded>
      {secret && (
        <Dialog title="Finish the connection on the other side" onClose={() => setSecret(null)} footer={<button className="btn primary" onClick={() => setSecret(null)}>Done</button>}>
          <div className="stack">
            {secret.inbound_url && (
              <Field label="URL to enter in the other system">
                <div className="secret">{secret.inbound_url}</div>
              </Field>
            )}
            {secret.generated_signing_secret && (
              <Field label="Signing secret" hint="Shown once. The receiver uses it to check that messages really come from Gantry.">
                <div className="secret">{secret.generated_signing_secret}</div>
              </Field>
            )}
          </div>
        </Dialog>
      )}
      {log && (
        <Dialog title={`Recent activity: ${log.name}`} wide onClose={() => setLog(null)}>
          <Loaded q={deliveries}>
            {(list) =>
              list.length === 0 ? (
                <Empty title="No activity yet" />
              ) : (
                <div className="table-wrap">
                  <table className="table">
                    <thead>
                      <tr>
                        <th>When</th>
                        <th>Direction</th>
                        <th>Event</th>
                        <th>Result</th>
                      </tr>
                    </thead>
                    <tbody>
                      {list.map((x, i) => (
                        <tr key={i}>
                          <td title={dateTime(x.at)} className="nowrap">{relative(x.at)}</td>
                          <td>{x.direction === "out" ? "Sent" : "Received"}</td>
                          <td>{x.event}</td>
                          <td>
                            <span className={`badge ${x.outcome === "ok" ? "green" : "red"}`}>{x.outcome === "ok" ? "OK" : titleCase(x.outcome)}</span> <span className="small muted">{x.detail}</span>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )
            }
          </Loaded>
        </Dialog>
      )}
      {jira && (
        <Dialog
          title={`Import from ${jira.name}`}
          onClose={() => setJira(null)}
          footer={
            <>
              <button className="btn" onClick={() => setJira(null)}>
                Cancel
              </button>
              <button className="btn primary" disabled={busy || !imp.project || !imp.jql.trim()} onClick={() => run(() => api("POST", `/api/v1/admin/integrations/${jira.id}/jira-import`, { project: imp.project, jql: imp.jql, max: Number(imp.max) || 1000 }), "Import queued. Follow it under Background jobs.").then((r) => r && setJira(null))}>
                Start import
              </button>
            </>
          }
        >
          <div className="stack">
            <Field label="Into project" required>
              <select className="select" value={imp.project} onChange={(e) => setImp({ ...imp, project: e.target.value })}>
                <option value="">Choose a project</option>
                {(projects.data ?? []).map((p) => (
                  <option key={p.key} value={p.key}>
                    {p.name}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="Which issues (JQL)" required hint="For example: project = PUMP ORDER BY created">
              <input className="input" value={imp.jql} onChange={(e) => setImp({ ...imp, jql: e.target.value })} />
            </Field>
            <Field label="Stop after" hint="issues">
              <input className="input num" type="number" min={1} max={20000} value={imp.max} onChange={(e) => setImp({ ...imp, max: e.target.value })} style={{ maxWidth: 140 }} />
            </Field>
            <p className="small muted">Types, statuses, priority, labels, due dates, parents and comments are mapped. People are matched by email. Running the same import again updates the items it created instead of duplicating them.</p>
          </div>
        </Dialog>
      )}
      {remove && <Confirm title={`Delete ${remove.name}?`} action="Delete integration" danger body="Messages stop immediately and inbound webhooks to its URL are refused." onClose={() => setRemove(null)} onConfirm={async () => void (await run(() => api("DELETE", `/api/v1/admin/integrations/${remove.id}`), "Integration deleted"))} />}
    </>
  );
}

// ───────── schemes ─────────
const FIELD_TYPES = [
  ["text", "Short text"], ["textarea", "Long text"], ["number", "Number"], ["date", "Date"], ["select", "One choice"], ["multiselect", "Several choices"], ["user", "Person"], ["url", "Link"], ["checkbox", "Yes / no"],
];
const COLORS = ["neutral", "blue", "green", "amber", "red", "purple", "teal"];
const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "").replace(/^([0-9])/, "f_$1");

function WorkflowEditor({ wf, fields, onClose }: { wf: any; fields: any[]; onClose: () => void }) {
  const [busy, run] = useAction();
  const [name, setName] = useState<string>(wf.name ?? "");
  const [key, setKey] = useState<string>(wf.key ?? "");
  const isNew = !wf.key;
  const start = useMemo<WorkflowDef>(
    () =>
      wf.definition
        ? { initial: wf.definition.initial, statuses: wf.definition.statuses, transitions: wf.definition.transitions, layout: wf.definition.layout ?? null }
        : { initial: "todo", statuses: [{ key: "todo", name: "To do", category: "todo" }, { key: "in_progress", name: "In progress", category: "in_progress" }, { key: "done", name: "Done", category: "done" }], transitions: [{ name: "Start", from: ["todo"], to: "in_progress" }, { name: "Complete", from: ["*"], to: "done" }, { name: "Reopen", from: ["done"], to: "todo" }] },
    [wf],
  );
  const [def, setDef] = useState<WorkflowDef>(start);
  const issues = problems(def);
  return (
    <Dialog
      title={isNew ? "New workflow" : `Edit workflow: ${wf.name}`}
      wide="canvas"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" disabled={busy || !name.trim() || !key || issues.length > 0} onClick={() => run(() => api("PUT", `/api/v1/admin/workflows/${key}`, { name, definition: def }), "Workflow saved").then((r) => r !== undefined && onClose())}>
            Save workflow
          </button>
        </>
      }
    >
      <div className="stack">
        <div className="form-grid">
          <Field label="Name" required>
            <input className="input" value={name} maxLength={80} onChange={(e) => (setName(e.target.value), isNew && setKey(slug(e.target.value)))} />
          </Field>
          <Field label="Key" hint={isNew ? "Used in the API and in add-on packs. It cannot be changed later." : "Cannot be changed."}>
            <input className="input" value={key} disabled={!isNew} maxLength={40} onChange={(e) => setKey(slug(e.target.value))} />
          </Field>
        </div>
        <p className="small muted" style={{ margin: 0 }}>
          Every work item type that uses this workflow follows it in every project, unless a project has designed its own. A status that still holds work cannot be removed; move the work first.
        </p>
        <Suspense fallback={<Loading what="Opening the workflow editor" />}>
          <VisualWorkflow value={start} onChange={setDef} fields={fields} />
        </Suspense>
      </div>
    </Dialog>
  );
}

export function Schemes() {
  const q = useGet<{ fields: any[]; workflows: any[]; item_types: any[] }>("/api/v1/admin/schemes");
  const [tab, setTab] = useState<"types" | "fields" | "workflows">("types");
  const [busy, run] = useAction();
  const [type, setType] = useState<any | null>(null);
  const [field, setField] = useState<any | null>(null);
  const [wf, setWf] = useState<any | null>(null);
  // Where a definition came from: the core pack, an add-on (shown by its name), or a local edit.
  const packs = useGet<{ installed: { id: string; name: string }[] }>("/api/v1/admin/packs");
  const origin = (source: string) => (source === "custom" ? "Customized" : source === "core" ? "Core" : source.length <= 4 ? source.toUpperCase() : titleCase(source));
  const originTitle = (source: string) => (source === "custom" ? "Changed by an administrator; upgrades leave it alone" : `Installed by: ${packs.data?.installed.find((p) => p.id === source)?.name ?? source}`);
  return (
    <>
      <Head title="Types and workflows" sub="Define the kinds of work your organization tracks, the information each carries, and the steps it moves through.">
        {tab === "types" && <button className="btn primary" onClick={() => setType({ isNew: true, key: "", name: "", icon: "circle", color: "blue", description: "", workflow: "simple", fields: [] })}><Plus /> New type</button>}
        {tab === "fields" && <button className="btn primary" onClick={() => setField({ isNew: true, key: "", name: "", type: "text", options: [], description: "" })}><Plus /> New field</button>}
        {tab === "workflows" && <button className="btn primary" onClick={() => setWf({})}><Plus /> New workflow</button>}
      </Head>
      <div className="tabs" role="tablist" style={{ marginBottom: 14 }}>
        {(["types", "fields", "workflows"] as const).map((t) => (
          <button key={t} role="tab" aria-selected={tab === t} onClick={() => setTab(t)}>
            {t === "types" ? "Work item types" : t === "fields" ? "Fields" : "Workflows"}
          </button>
        ))}
      </div>
      <Loaded q={q}>
        {(d) => (
          <>
            {tab === "types" && (
              <div className="panel table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <th>Type</th>
                      <th>Workflow</th>
                      <th className="hide-sm">Fields</th>
                      <th className="hide-sm">Used in</th>
                      <th>From</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {d.item_types.map((t) => (
                      <tr key={t.key}>
                        <td>
                          <span className="row">
                            <TypeIcon type={t} title={false} /> <strong>{t.name}</strong>
                          </span>
                          <div className="small muted">{t.description}</div>
                        </td>
                        <td>{d.workflows.find((w) => w.key === t.workflow)?.name ?? t.workflow}</td>
                        <td className="hide-sm num">{t.fields.length}</td>
                        <td className="hide-sm num nowrap">{t.projects} project{t.projects === 1 ? "" : "s"}</td>
                        <td>
                          <span className={`badge ${t.source === "custom" ? "blue" : "outline"}`} title={originTitle(t.source)}>{origin(t.source)}</span>
                        </td>
                        <td>
                          <button className="btn sm" onClick={() => setType({ ...t })}>
                            Edit
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
            {tab === "fields" && (
              <div className="panel table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <th>Field</th>
                      <th>Kind</th>
                      <th className="hide-sm">Choices</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {d.fields.map((f) => (
                      <tr key={f.key}>
                        <td>
                          <strong>{f.name}</strong> <code className="small muted">{f.key}</code>
                        </td>
                        <td>{FIELD_TYPES.find((x) => x[0] === f.type)?.[1]}</td>
                        <td className="hide-sm small muted truncate" style={{ maxWidth: 320 }}>{f.options.join(", ")}</td>
                        <td>
                          <button className="btn sm" onClick={() => setField({ ...f })}>
                            Edit
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
            {tab === "workflows" && (
              <div className="stack">
                {d.workflows.map((w) => (
                  <section key={w.key} className="panel">
                    <header>
                      <h2 className="row">
                        {w.name} <span className={`badge ${w.source === "custom" ? "blue" : "outline"}`} title={originTitle(w.source)}>{origin(w.source)}</span>
                        <span className="small muted">used by {w.used_by} type{w.used_by === 1 ? "" : "s"}</span>
                      </h2>
                      <button className="btn sm" onClick={() => setWf(w)}>
                        Edit
                      </button>
                    </header>
                    <div className="body stack tight">
                      <div className="flow">
                        {w.definition.statuses.map((s: Status, i: number) => (
                          <span key={s.key} className="row" style={{ gap: 6 }}>
                            {i > 0 && <ArrowRight width={13} height={13} className="muted" />}
                            <StatusBadge name={s.name} category={s.category} />
                          </span>
                        ))}
                      </div>
                      <span className="small muted">Moves: {w.definition.transitions.map((t: Transition) => t.name).join(", ")}</span>
                    </div>
                  </section>
                ))}
              </div>
            )}
            {type && (
              <Dialog
                title={type.isNew ? "New work item type" : `Edit ${type.name}`}
                wide
                onClose={() => setType(null)}
                footer={
                  <>
                    <button className="btn" onClick={() => setType(null)}>
                      Cancel
                    </button>
                    <button className="btn primary" disabled={busy || !type.name.trim() || !type.key} onClick={() => run(() => api("PUT", `/api/v1/admin/item-types/${type.key}`, { name: type.name, icon: type.icon, color: type.color, description: type.description, workflow: type.workflow, fields: type.fields, active: true }), "Type saved").then((r) => r !== undefined && setType(null))}>
                      Save type
                    </button>
                  </>
                }
              >
                <div className="stack">
                  <div className="form-grid">
                    <Field label="Name" required>
                      <input className="input" value={type.name} onChange={(e) => setType({ ...type, name: e.target.value, key: type.isNew ? slug(e.target.value) : type.key })} placeholder="Calibration" />
                    </Field>
                    <Field label="Workflow">
                      <select className="select" value={type.workflow} onChange={(e) => setType({ ...type, workflow: e.target.value })}>
                        {d.workflows.map((w) => (
                          <option key={w.key} value={w.key}>
                            {w.name}
                          </option>
                        ))}
                      </select>
                    </Field>
                    <Field label="Icon">
                      <select className="select" value={type.icon} onChange={(e) => setType({ ...type, icon: e.target.value })}>
                        {ICON_NAMES.map((n) => (
                          <option key={n} value={n}>
                            {titleCase(n.replace(/-/g, " "))}
                          </option>
                        ))}
                      </select>
                    </Field>
                    <Field label="Color">
                      <select className="select" value={type.color} onChange={(e) => setType({ ...type, color: e.target.value })}>
                        {COLORS.map((c) => (
                          <option key={c} value={c}>
                            {titleCase(c)}
                          </option>
                        ))}
                      </select>
                    </Field>
                  </div>
                  <Field label="Description">
                    <input className="input" value={type.description} onChange={(e) => setType({ ...type, description: e.target.value })} />
                  </Field>
                  <div className="stack tight">
                    <span className="label">Fields on this type</span>
                    {type.fields.map((tf: any, i: number) => (
                      <div key={tf.key} className="row">
                        <span className="grow">{d.fields.find((f) => f.key === tf.key)?.name ?? tf.key}</span>
                        <label className="check small">
                          <input type="checkbox" checked={!!tf.required} onChange={(e) => setType({ ...type, fields: type.fields.map((x: any, j: number) => (j === i ? { ...x, required: e.target.checked } : x)) })} /> Required when creating
                        </label>
                        <button className="btn ghost icon sm" aria-label="Remove field" onClick={() => setType({ ...type, fields: type.fields.filter((_: any, j: number) => j !== i) })}>
                          <X />
                        </button>
                      </div>
                    ))}
                    <select className="select" style={{ maxWidth: 280 }} value="" aria-label="Add a field" onChange={(e) => e.target.value && setType({ ...type, fields: [...type.fields, { key: e.target.value, required: false }] })}>
                      <option value="">Add a field…</option>
                      {d.fields.filter((f) => !type.fields.some((x: any) => x.key === f.key)).map((f) => (
                        <option key={f.key} value={f.key}>
                          {f.name}
                        </option>
                      ))}
                    </select>
                  </div>
                  <p className="small muted">After saving, switch the type on in each project that should use it (Project → Settings → Work item types).</p>
                </div>
              </Dialog>
            )}
            {field && (
              <Dialog
                title={field.isNew ? "New field" : `Edit ${field.name}`}
                onClose={() => setField(null)}
                footer={
                  <>
                    <button className="btn" onClick={() => setField(null)}>
                      Cancel
                    </button>
                    <button className="btn primary" disabled={busy || !field.name.trim() || !field.key} onClick={() => run(() => api("PUT", `/api/v1/admin/fields/${field.key}`, { name: field.name, type: field.type, options: field.options, description: field.description }), "Field saved").then((r) => r !== undefined && setField(null))}>
                      Save field
                    </button>
                  </>
                }
              >
                <div className="stack">
                  <Field label="Name" required>
                    <input className="input" value={field.name} onChange={(e) => setField({ ...field, name: e.target.value, key: field.isNew ? slug(e.target.value) : field.key })} placeholder="Batch number" />
                  </Field>
                  <Field label="Kind" hint={field.isNew ? undefined : "The kind cannot change once items hold values for this field."}>
                    <select className="select" value={field.type} onChange={(e) => setField({ ...field, type: e.target.value })}>
                      {FIELD_TYPES.map(([k, l]) => (
                        <option key={k} value={k}>
                          {l}
                        </option>
                      ))}
                    </select>
                  </Field>
                  {["select", "multiselect"].includes(field.type) && (
                    <Field label="Choices" hint="One per line.">
                      <textarea className="textarea" value={field.options.join("\n")} onChange={(e) => setField({ ...field, options: e.target.value.split("\n") })} />
                    </Field>
                  )}
                  <Field label="Help text">
                    <input className="input" value={field.description} onChange={(e) => setField({ ...field, description: e.target.value })} />
                  </Field>
                </div>
              </Dialog>
            )}
            {wf && <WorkflowEditor wf={wf} fields={d.fields} onClose={() => setWf(null)} />}
          </>
        )}
      </Loaded>
    </>
  );
}

// ───────── add-ons ─────────
export function Addons() {
  const q = useGet<{ installed: any[]; available: any[]; addons_dir: string }>("/api/v1/admin/packs");
  const [busy, run] = useAction();
  return (
    <>
      <Head title="Add-ons" sub="Packs add work item types, workflows, project templates and page templates for a discipline." />
      <Loaded q={q}>
        {(d) => (
          <div className="stack loose">
            <section className="panel">
              <header>
                <h2>Installed</h2>
              </header>
              {d.installed.map((p) => (
                <div key={p.id} className="list-row" style={{ alignItems: "flex-start" }}>
                  <Puzzle width={17} height={17} style={{ marginTop: 2 }} />
                  <div className="grow stack tight">
                    <span>
                      <strong>{p.name}</strong> <span className="muted small">version {p.version}, installed {relative(p.installed_at)}</span>
                    </span>
                    <span className="small muted">{p.description}</span>
                    <span className="small">{Object.entries(p.contents).map(([k, v]) => `${v} ${k.replace(/_/g, " ")}`).join(", ")}</span>
                  </div>
                </div>
              ))}
            </section>
            <section className="panel">
              <header>
                <h2>Available on this server</h2>
                <span className="small muted">Packs found in {d.addons_dir}</span>
              </header>
              {d.available.length === 0 ? (
                <Empty icon={<Puzzle />} title="No add-on packages are installed on the server">
                  <p>A system administrator installs them as RPM packages, for example <code>dnf install gantry-addon-nasa</code>. They then appear here.</p>
                </Empty>
              ) : (
                d.available.map((p) => {
                  const inst = d.installed.find((i) => i.id === p.id);
                  return (
                    <div key={p.id} className="list-row" style={{ alignItems: "flex-start" }}>
                      <div className="grow stack tight">
                        <span>
                          <strong>{p.name ?? p.id}</strong> {p.valid && <span className="muted small">version {p.version}</span>}
                        </span>
                        {p.valid ? <span className="small muted">{p.description}</span> : <span className="error-text">This pack cannot be read: {p.error}</span>}
                      </div>
                      {p.valid && (
                        <button className={`btn sm ${inst ? "" : "primary"}`} disabled={busy || (inst && inst.version === p.version)} onClick={() => run(() => api("POST", `/api/v1/admin/packs/${p.id}/install`), `${p.name} ${inst ? "updated" : "installed"}. Its templates are now offered when creating projects.`)}>
                          {!inst ? "Install" : inst.version === p.version ? "Installed" : `Update to ${p.version}`}
                        </button>
                      )}
                    </div>
                  );
                })
              )}
            </section>
          </div>
        )}
      </Loaded>
    </>
  );
}
