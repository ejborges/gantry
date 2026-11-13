// Administration: site and legal settings, profile rules, notification defaults and email,
// alert rules, and the rules for AI agent accounts.
import { useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Bot, ChevronRight, CircleAlert, CircleCheck, Mail, Plus, Send, ShieldAlert, Trash2 } from "lucide-react";
import { api, useGet } from "../api";
import { useSession } from "../session";
import { AgentBadge, Avatar, Confirm, Dialog, Empty, Field, Loaded, Markdown, toast, useAction } from "../ui";
import { relative, stamp, titleCase } from "../lib/format";

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

// ───────────────────────── site, legal, profiles ─────────────────────────

const PROFILE_FIELDS: [string, string][] = [["name", "Display name"], ["job_title", "Job title"], ["org_name", "Organization"], ["org_code", "Organization code"]];

export function SiteSettings() {
  const q = useGet<any>("/api/v1/admin/settings");
  const { can } = useSession();
  const [busy, run] = useAction();
  const [site, setSite] = useState<any | null>(null);
  const [legal, setLegal] = useState<Record<string, string> | null>(null);
  const [doc, setDoc] = useState("privacy");
  const [profile, setProfile] = useState<any | null>(null);
  const save = (key: string, value: unknown, done: string) => run(() => api("PUT", `/api/v1/admin/settings/${key}`, value), done);
  return (
    <>
      <Head title="Settings" sub="What the installation is called, who to ask for help, the statements people can read, and what they may change about themselves." />
      <Loaded q={q} what="Loading the settings">
        {(d) => {
          const s = site ?? d.stored.site;
          const l = legal ?? d.stored.legal;
          const p = profile ?? d.stored.profile;
          return (
            <div className="stack loose" style={{ maxWidth: 860 }}>
              {d.stored.site && (
                <section className="panel">
                  <header>
                    <h2>This site</h2>
                  </header>
                  <div className="body stack">
                    <div className="form-grid">
                      <Field label="Site name" hint="Replaces “Gantry” in the navigation, on the sign-in page and in email.">
                        <input className="input" maxLength={60} value={s.name} onChange={(e) => setSite({ ...s, name: e.target.value })} />
                      </Field>
                      <Field label="Time zone of the organization" hint="Decides the hour and the day of reminders for people who have not chosen a time zone of their own.">
                        <input className="input" value={s.timezone} onChange={(e) => setSite({ ...s, timezone: e.target.value })} placeholder="America/New_York" list="tz-names" />
                        <datalist id="tz-names">
                          {(() => {
                            try {
                              return ((Intl as any).supportedValuesOf("timeZone") as string[]).map((z) => <option key={z} value={z} />);
                            } catch {
                              return null;
                            }
                          })()}
                        </datalist>
                      </Field>
                    </div>
                    <Field label="Announcement" hint="Shown to everyone in a bar at the top until you clear it.">
                      <input className="input" maxLength={500} value={s.announcement} onChange={(e) => setSite({ ...s, announcement: e.target.value })} placeholder="Maintenance on Saturday from 06:00 to 08:00" />
                    </Field>
                    <Field label="Sign-in notice" hint="For example an acceptable-use statement.">
                      <textarea className="textarea" maxLength={2000} value={s.login_notice} onChange={(e) => setSite({ ...s, login_notice: e.target.value })} />
                    </Field>
                    <h3>Where people get help</h3>
                    <p className="small muted">Shown on error pages, on the sign-in page and under About. Leave empty what does not apply.</p>
                    <div className="form-grid">
                      <Field label="Who to ask">
                        <input className="input" maxLength={100} value={s.support.name} onChange={(e) => setSite({ ...s, support: { ...s.support, name: e.target.value } })} placeholder="IT service desk" />
                      </Field>
                      <Field label="Email">
                        <input className="input" type="email" maxLength={320} value={s.support.email} onChange={(e) => setSite({ ...s, support: { ...s.support, email: e.target.value } })} />
                      </Field>
                      <Field label="Web address">
                        <input className="input" type="url" maxLength={500} value={s.support.url} onChange={(e) => setSite({ ...s, support: { ...s.support, url: e.target.value } })} placeholder="https://" />
                      </Field>
                      <Field label="Anything else" hint="Hours, a phone extension, a chat channel.">
                        <input className="input" maxLength={500} value={s.support.text} onChange={(e) => setSite({ ...s, support: { ...s.support, text: e.target.value } })} />
                      </Field>
                    </div>
                    <div>
                      <button className="btn primary" disabled={busy || !site} onClick={() => save("site", s, "Settings saved").then((r) => r !== undefined && setSite(null))}>
                        Save
                      </button>
                    </div>
                  </div>
                </section>
              )}
              {d.stored.legal && (
                <section className="panel">
                  <header>
                    <h2>Statements people can read</h2>
                    <Link className="small" to="/about">
                      See them as readers do
                    </Link>
                  </header>
                  <div className="body stack">
                    <p className="small muted">Written in Markdown and shown under About to everyone, signed in or not. A statement left empty is not listed. These are your organization's words: Gantry does not supply legal text.</p>
                    <div className="tabs" role="tablist">
                      {d.legal_documents.map((x: any) => (
                        <button key={x.key} role="tab" aria-selected={doc === x.key} onClick={() => setDoc(x.key)}>
                          {x.title} {l[x.key]?.trim() ? "" : <span className="count">empty</span>}
                        </button>
                      ))}
                    </div>
                    <div className="form-grid">
                      <Field label={d.legal_documents.find((x: any) => x.key === doc)?.title ?? doc}>
                        <textarea className="textarea" rows={12} style={{ fontFamily: "var(--mono)", fontSize: 13 }} value={l[doc] ?? ""} onChange={(e) => setLegal({ ...l, [doc]: e.target.value })} />
                      </Field>
                      <div className="preview panel" style={{ padding: 12, maxHeight: 320, overflow: "auto" }} aria-label="How it will read">
                        <Markdown source={l[doc] ?? ""} empty="Nothing written yet." compact />
                      </div>
                    </div>
                    <div>
                      <button className="btn primary" disabled={busy || !legal} onClick={() => save("legal", l, "Statements saved").then((r) => r !== undefined && setLegal(null))}>
                        Save statements
                      </button>
                    </div>
                  </div>
                </section>
              )}
              {d.stored.profile && (
                <section className="panel">
                  <header>
                    <h2>Profiles</h2>
                  </header>
                  <div className="body stack">
                    <fieldset className="stack tight" style={{ border: 0, padding: 0, margin: 0 }}>
                      <legend className="field-label">People may change their own</legend>
                      {PROFILE_FIELDS.map(([k, label]) => (
                        <label key={k} className="check">
                          <input type="checkbox" checked={p.user_editable[k] !== false} onChange={(e) => setProfile({ ...p, user_editable: { ...p.user_editable, [k]: e.target.checked } })} /> {label}
                        </label>
                      ))}
                      <p className="small muted">What is not selected is set by administrators under Users and agents, or by your identity provider.</p>
                    </fieldset>
                    <div className="form-grid">
                      <Field label="Largest profile picture, in KB">
                        <input className="input" type="number" min={16} max={4096} value={p.avatar_max_kb} onChange={(e) => setProfile({ ...p, avatar_max_kb: Number(e.target.value) })} />
                      </Field>
                      <Field label="Largest side, in pixels" hint="People are advised to upload a square of 256 × 256.">
                        <input className="input" type="number" min={64} max={4096} value={p.avatar_max_px} onChange={(e) => setProfile({ ...p, avatar_max_px: Number(e.target.value) })} />
                      </Field>
                    </div>
                    <div>
                      <button className="btn primary" disabled={busy || !profile} onClick={() => save("profile", p, "Profile rules saved").then((r) => r !== undefined && setProfile(null))}>
                        Save profile rules
                      </button>
                    </div>
                  </div>
                </section>
              )}
              {can("admin.settings") && d.config_file && (
                <section className="panel">
                  <header>
                    <h2>Server configuration</h2>
                    <span className="small muted">Read-only here. Change it in /etc/gantry/gantry.toml and restart the service.</span>
                  </header>
                  <div className="body stack">
                    {!d.secret_key_set && <div className="notice danger">No secret key is configured.</div>}
                    <pre className="code" tabIndex={0}>{JSON.stringify(d.config_file, null, 2)}</pre>
                    <p className="small muted">Passwords, keys and tokens are never shown.</p>
                  </div>
                </section>
              )}
            </div>
          );
        }}
      </Loaded>
    </>
  );
}

// ───────────────────────── notifications and email ─────────────────────────

export function NotificationSettings() {
  const q = useGet<any>("/api/v1/admin/settings");
  const email = useGet<any>("/api/v1/admin/email");
  const { can, me } = useSession();
  const [busy, run] = useAction();
  const [draft, setDraft] = useState<any | null>(null);
  const [to, setTo] = useState(me.user.email);
  return (
    <>
      <Head title="Notifications and email" sub="What people are told about unless they choose otherwise, when reminders go out, and whether mail is being delivered." />
      <div className="stack loose" style={{ maxWidth: 900 }}>
        <Loaded q={q} what="Loading the notification settings">
          {(d) => {
            const n = draft ?? d.stored.notifications;
            if (!n) return <Empty icon={<Mail />} title="Your role does not include notification settings" />;
            const cat = (key: string, field: string, value: boolean) => setDraft({ ...n, categories: { ...n.categories, [key]: { ...n.categories[key], [field]: value } } });
            return (
              <section className="panel">
                <header>
                  <h2>Defaults for everyone</h2>
                </header>
                <div className="body stack">
                  <label className="check">
                    <input type="checkbox" checked={n.email_enabled} onChange={(e) => setDraft({ ...n, email_enabled: e.target.checked })} /> Send notifications by email (each person can still turn email off for themselves)
                  </label>
                  <div className="table-wrap">
                    <table className="table">
                      <thead>
                        <tr>
                          <th scope="col">Kind of notification</th>
                          <th scope="col">In the application</th>
                          <th scope="col">By email</th>
                          <th scope="col" title="People cannot change a locked choice">Same for everyone</th>
                        </tr>
                      </thead>
                      <tbody>
                        {d.notification_categories.map((c: any) => (
                          <tr key={c.key}>
                            <th scope="row" style={{ fontWeight: 400 }}>
                              <strong>{c.label}</strong>
                              <div className="small muted">{c.description}</div>
                              {c.permission && <div className="small muted">Only for accounts with the {c.permission} permission.</div>}
                            </th>
                            {["in_app", "email", "locked"].map((f) => (
                              <td key={f}>
                                <input type="checkbox" aria-label={`${c.label}: ${f === "in_app" ? "in the application" : f === "email" ? "by email" : "same for everyone"}`} checked={!!n.categories[c.key]?.[f]} onChange={(e) => cat(c.key, f, e.target.checked)} />
                              </td>
                            ))}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  <h3>Reminders about work</h3>
                  <div className="form-grid">
                    <Field label="Remind this many days before something is due" hint="0 turns the reminder off.">
                      <input className="input" type="number" min={0} max={30} value={n.reminders.due_soon_days} onChange={(e) => setDraft({ ...n, reminders: { ...n.reminders, due_soon_days: Number(e.target.value) } })} />
                    </Field>
                    <Field label="Remind about overdue work every … days" hint="0 turns the reminder off.">
                      <input className="input" type="number" min={0} max={30} value={n.reminders.overdue_every_days} onChange={(e) => setDraft({ ...n, reminders: { ...n.reminders, overdue_every_days: Number(e.target.value) } })} />
                    </Field>
                    <Field label="Hour of the day" hint={`0 to 23, in each person's own time zone. People whose time zone is Automatic get the organization's (${d.stored.site?.timezone ?? "set under Settings"}).`}>
                      <input className="input" type="number" min={0} max={23} value={n.reminders.hour} onChange={(e) => setDraft({ ...n, reminders: { ...n.reminders, hour: Number(e.target.value) } })} />
                    </Field>
                  </div>
                  <div>
                    <button className="btn primary" disabled={busy || !draft} onClick={() => run(() => api("PUT", "/api/v1/admin/settings/notifications", n), "Notification settings saved").then((r) => r !== undefined && setDraft(null))}>
                      Save
                    </button>
                  </div>
                </div>
              </section>
            );
          }}
        </Loaded>
        <Loaded q={email} what="Checking the mail settings">
          {(e) => (
            <section className="panel">
              <header>
                <h2 className="row">
                  Email delivery {e.configured ? <span className="badge green"><CircleCheck /> Set up</span> : <span className="badge amber"><CircleAlert /> Not set up</span>}
                </h2>
              </header>
              <div className="body stack">
                {e.configured ? (
                  <dl className="props" style={{ gridTemplateColumns: "170px minmax(0, 1fr)" }}>
                    <dt>Mail server</dt>
                    <dd>
                      {e.host}:{e.port} ({e.encryption}
                      {e.authenticated ? ", signs in" : ", no sign-in"})
                    </dd>
                    <dt>From</dt>
                    <dd>{e.from}</dd>
                    <dt>Last 24 hours</dt>
                    <dd>
                      {e.last_24h.sent} sent, {e.last_24h.failed} failed, {e.last_24h.skipped} not sent
                    </dd>
                  </dl>
                ) : (
                  <p>
                    No mail server is configured, so nothing is sent by email. A system administrator sets <code>[email]</code> in <code>/etc/gantry/gantry.toml</code> (host, port, from) and restarts the service. The system administrator guide has the details.
                  </p>
                )}
                {e.configured && !e.public_url_set && <div className="notice warn">The server's public address is not set, so links in email cannot be followed. Set <code>public_url</code> in the configuration file.</div>}
                {e.configured && can("admin.settings") && (
                  <form
                    className="row wrap"
                    onSubmit={(ev) => {
                      ev.preventDefault();
                      void run(() => api("POST", "/api/v1/admin/email/test", { to }), (r: any) => toast(r?.detail ?? "The mail server accepted the test message."));
                    }}
                  >
                    <input className="input" style={{ maxWidth: 300 }} type="email" value={to} onChange={(ev) => setTo(ev.target.value)} aria-label="Send a test message to" />
                    <button className="btn" disabled={busy}>
                      <Send /> Send a test message
                    </button>
                  </form>
                )}
                {e.log.length > 0 && (
                  <div className="table-wrap" style={{ maxHeight: 320, overflow: "auto" }}>
                    <table className="table">
                      <thead>
                        <tr>
                          <th>When</th>
                          <th>To</th>
                          <th>Subject</th>
                          <th>Outcome</th>
                        </tr>
                      </thead>
                      <tbody>
                        {e.log.map((m: any, i: number) => (
                          <tr key={i}>
                            <td className="nowrap small" title={stamp(m.at)}>{relative(m.at)}</td>
                            <td className="small">{m.to_addr}</td>
                            <td className="small truncate" style={{ maxWidth: 280 }}>{m.subject}</td>
                            <td>
                              <span className={`badge ${m.outcome === "sent" ? "green" : m.outcome === "failed" ? "red" : ""}`} title={m.detail || undefined}>
                                {m.outcome === "skipped" ? "not sent" : m.outcome}
                              </span>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            </section>
          )}
        </Loaded>
      </div>
    </>
  );
}

// ───────────────────────── alerts ─────────────────────────

const SEVERITY: Record<string, string> = { info: "blue", warning: "amber", critical: "red" };

function RuleDialog({ data, rule, onClose }: { data: any; rule: any; onClose: () => void }) {
  const [r, setR] = useState<any>(rule);
  const [busy, run] = useAction();
  const kinds = data.kinds.filter((k: any) => k.scope === r.scope);
  const kind = data.kinds.find((k: any) => k.kind === r.kind) ?? kinds[0];
  const ch = r.channels ?? {};
  const setCh = (c: object) => setR({ ...r, channels: { ...ch, ...c } });
  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  const body = () => ({
    name: r.name, params: r.params, severity: r.severity, channels: { in_app: ch.in_app !== false, email: ch.email !== false, roles: ch.roles ?? [], addresses: ch.addresses ?? [], integrations: ch.integrations ?? [] },
    enabled: r.enabled, cooldown_minutes: Number(r.cooldown_minutes), notify_resolved: r.notify_resolved,
  });
  return (
    <Dialog
      title={r.id ? `Alert: ${rule.name}` : "New alert"}
      wide
      onClose={onClose}
      footer={
        <>
          {r.id && (
            <button className="btn" style={{ marginRight: "auto" }} disabled={busy} onClick={() => run(() => api("POST", `/api/v1/admin/alerts/${r.id}/test`), (t: any) => toast(`Measured now: ${t?.message ?? t?.value ?? "done"}. A test alert went out through this rule's channels.`))}>
              <Send /> Measure now and send a test
            </button>
          )}
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" disabled={busy || !r.name?.trim()} onClick={() => run(() => (r.id ? api("PATCH", `/api/v1/admin/alerts/${r.id}`, body()) : api("POST", "/api/v1/admin/alerts", { scope: r.scope, kind: kind.kind, ...body() })), "Alert saved").then((x) => x !== undefined && onClose())}>
            Save
          </button>
        </>
      }
    >
      <div className="stack">
        <div className="form-grid">
          <Field label="What to watch">
            <select className="select" value={kind.kind} disabled={!!r.id} onChange={(e) => setR({ ...r, kind: e.target.value, params: {}, name: r.name || data.kinds.find((k: any) => k.kind === e.target.value)?.name })}>
              {kinds.map((k: any) => (
                <option key={k.kind} value={k.kind}>
                  {k.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Name" required>
            <input className="input" maxLength={200} value={r.name ?? ""} onChange={(e) => setR({ ...r, name: e.target.value })} />
          </Field>
        </div>
        <p className="small muted">{kind.description}</p>
        <div className="form-grid">
          {kind.params.map((p: any) => (
            <Field key={p.key} label={p.label}>
              <input className="input" type="number" min={0} max={1000000} value={r.params?.[p.key] ?? p.default} onChange={(e) => setR({ ...r, params: { ...r.params, [p.key]: Number(e.target.value) } })} />
            </Field>
          ))}
          {["overdue_items", "stale_items"].includes(kind.kind) && (
            <Field label="Only in this project" hint="A project key. Empty means everywhere.">
              <input className="input" value={r.params?.project ?? ""} onChange={(e) => setR({ ...r, params: e.target.value.trim() ? { ...r.params, project: e.target.value.trim().toUpperCase() } : Object.fromEntries(Object.entries(r.params ?? {}).filter(([k]) => k !== "project")) })} />
            </Field>
          )}
          <Field label="Severity">
            <select className="select" value={r.severity} onChange={(e) => setR({ ...r, severity: e.target.value })}>
              <option value="info">Information</option>
              <option value="warning">Warning</option>
              <option value="critical">Critical</option>
            </select>
          </Field>
          <Field label="Do not repeat within … minutes">
            <input className="input" type="number" min={1} max={10080} value={r.cooldown_minutes} onChange={(e) => setR({ ...r, cooldown_minutes: e.target.value })} />
          </Field>
        </div>
        <fieldset className="stack tight" style={{ border: 0, padding: 0, margin: 0 }}>
          <legend className="field-label">Who is told</legend>
          <div className="row wrap">
            <span className="small muted">Accounts with the role:</span>
            {data.roles.map((role: any) => (
              <label key={role.key} className="check">
                <input type="checkbox" checked={(ch.roles ?? []).includes(role.key)} onChange={() => setCh({ roles: toggle(ch.roles ?? [], role.key) })} /> {role.name}
              </label>
            ))}
          </div>
          <div className="row wrap">
            <label className="check">
              <input type="checkbox" checked={ch.in_app !== false} onChange={(e) => setCh({ in_app: e.target.checked })} /> in the application
            </label>
            <label className="check">
              <input type="checkbox" checked={ch.email !== false} onChange={(e) => setCh({ email: e.target.checked })} /> by email
            </label>
          </div>
          <Field label="Also send email to" hint="Addresses outside the application, comma separated: an on-call mailbox, a pager gateway.">
            <input className="input" value={(ch.addresses ?? []).join(", ")} onChange={(e) => setCh({ addresses: e.target.value.split(",").map((a: string) => a.trim()).filter(Boolean) })} />
          </Field>
          {data.integrations.length > 0 && (
            <div className="row wrap">
              <span className="small muted">Post to:</span>
              {data.integrations.map((i: any) => (
                <label key={i.id} className="check">
                  <input type="checkbox" checked={(ch.integrations ?? []).includes(i.id)} onChange={() => setCh({ integrations: toggle(ch.integrations ?? [], i.id) })} /> {i.name} <span className="muted">({titleCase(i.kind)})</span>
                </label>
              ))}
            </div>
          )}
        </fieldset>
        <label className="check">
          <input type="checkbox" checked={r.notify_resolved} onChange={(e) => setR({ ...r, notify_resolved: e.target.checked })} /> Also say when it is back to normal
        </label>
        <label className="check">
          <input type="checkbox" checked={r.enabled} onChange={(e) => setR({ ...r, enabled: e.target.checked })} /> This alert is on
        </label>
      </div>
    </Dialog>
  );
}

export function Alerts() {
  const q = useGet<any>("/api/v1/admin/alerts", { refetchInterval: 60_000 });
  const [editing, setEditing] = useState<any | null>(null);
  const [removing, setRemoving] = useState<any | null>(null);
  const [busy, run] = useAction();
  return (
    <>
      <Head title="Alerts" sub="Conditions that are measured every minute. When one is met, the people and channels you choose are told; again when it is back to normal." />
      <Loaded q={q} what="Loading the alert rules">
        {(d) => (
          <div className="stack loose">
            {(["system", "application"] as const).map((scope) => {
              const rules = d.rules.filter((r: any) => r.scope === scope);
              const editable = d.can_edit[scope];
              return (
                <section key={scope}>
                  <div className="section-title">
                    <h2>{scope === "system" ? "System: performance, errors and security" : "Application: work, requests and accounts"}</h2>
                    {editable && (
                      <button className="btn sm" onClick={() => setEditing({ scope, kind: d.kinds.find((k: any) => k.scope === scope).kind, name: "", params: {}, severity: "warning", channels: { roles: [scope === "system" ? "system_admin" : "app_admin"] }, enabled: true, cooldown_minutes: 60, notify_resolved: true })}>
                        <Plus /> New alert
                      </button>
                    )}
                  </div>
                  <div className="panel">
                    {rules.length === 0 && <p className="muted" style={{ padding: 14 }}>No alerts of this kind.</p>}
                    {rules.map((r: any) => (
                      <div key={r.id} className="list-row">
                        <span className={`status-dot ${!r.enabled ? "" : r.state === "firing" ? "blocked" : "done"}`} aria-hidden="true" />
                        <span className="grow stack tight" style={{ minWidth: 0 }}>
                          <span className="row wrap" style={{ gap: 6 }}>
                            <strong>{r.name}</strong>
                            <span className={`badge ${SEVERITY[r.severity] ?? ""}`}>{r.severity}</span>
                            {!r.enabled ? <span className="badge outline">Off</span> : r.state === "firing" ? <span className="badge red"><ShieldAlert /> Condition met now</span> : null}
                          </span>
                          <span className="small muted">
                            {d.kinds.find((k: any) => k.kind === r.kind)?.name}
                            {r.last_checked_at ? ` · measured ${relative(r.last_checked_at)}${r.last_value !== null && r.last_value !== undefined ? `: ${r.last_value}` : ""}` : " · not measured yet"}
                            {r.last_fired_at ? ` · last alert ${relative(r.last_fired_at)}` : ""}
                          </span>
                        </span>
                        {editable && (
                          <>
                            <button className="btn sm" disabled={busy} onClick={() => run(() => api("PATCH", `/api/v1/admin/alerts/${r.id}`, { enabled: !r.enabled }), r.enabled ? "Alert turned off" : "Alert turned on")}>
                              {r.enabled ? "Turn off" : "Turn on"}
                            </button>
                            <button className="btn sm" onClick={() => setEditing(r)}>
                              Edit
                            </button>
                            {!r.builtin && (
                              <button className="btn ghost icon sm" aria-label={`Delete the alert ${r.name}`} onClick={() => setRemoving(r)}>
                                <Trash2 />
                              </button>
                            )}
                          </>
                        )}
                      </div>
                    ))}
                  </div>
                </section>
              );
            })}
            <section>
              <div className="section-title">
                <h2>Recent alerts</h2>
              </div>
              {d.events.length === 0 ? (
                <p className="muted">Nothing has triggered an alert yet.</p>
              ) : (
                <div className="panel table-wrap">
                  <table className="table">
                    <thead>
                      <tr>
                        <th>When</th>
                        <th>Alert</th>
                        <th>What happened</th>
                      </tr>
                    </thead>
                    <tbody>
                      {d.events.map((e: any) => (
                        <tr key={e.id}>
                          <td className="nowrap small" title={relative(e.at)}>{stamp(e.at)}</td>
                          <td>
                            {e.rule} <span className={`badge ${e.state === "firing" ? (SEVERITY[e.severity] ?? "") : "green"}`}>{e.state === "firing" ? e.severity : "back to normal"}</span>
                          </td>
                          <td className="small">{e.message}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
            {editing && <RuleDialog data={d} rule={editing} onClose={() => setEditing(null)} />}
            {removing && (
              <Confirm title={`Delete the alert ${removing.name}?`} action="Delete alert" danger body="Its history of past alerts is deleted with it." onClose={() => setRemoving(null)} onConfirm={async () => void (await run(() => api("DELETE", `/api/v1/admin/alerts/${removing.id}`), "Alert deleted"))} />
            )}
          </div>
        )}
      </Loaded>
    </>
  );
}

// ───────────────────────── AI agent rules ─────────────────────────

const PRESETS: { label: string; hint: string; permissions: string[] }[] = [
  { label: "Nothing", hint: "The agent can sign in to the API and do nothing else.", permissions: [] },
  { label: "Read", hint: "See work and pages where its roles allow, and search.", permissions: ["workspace.access", "project.view", "space.view"] },
  { label: "Read and comment", hint: "Also take part in discussions.", permissions: ["workspace.access", "project.view", "space.view", "item.comment"] },
  { label: "Work on items", hint: "Also create, change, move and be assigned work items.", permissions: ["workspace.access", "project.view", "space.view", "item.comment", "item.create", "item.edit", "item.transition", "item.assign"] },
  { label: "Write pages", hint: "Read everything it may, and create and edit pages.", permissions: ["workspace.access", "project.view", "space.view", "page.edit"] },
];

export function AgentRules() {
  const { id } = useParams();
  const q = useGet<any>(`/api/v1/admin/users/${id}/agent-policy`);
  const who = useGet<any>(`/api/v1/users/${id}`);
  const projects = useGet<any[]>("/api/v1/projects");
  const spaces = useGet<any[]>("/api/v1/spaces");
  const { can } = useSession();
  const [busy, run] = useAction();
  const [draft, setDraft] = useState<any | null>(null);
  const [filter, setFilter] = useState("");
  const agent = who.data;
  return (
    <>
      <div className="crumbs">
        <Link to="/admin/users">Users and agents</Link>
        <ChevronRight />
      </div>
      <Loaded q={q} what="Loading the agent's rules">
        {(d) => {
          const p = draft ?? d.policy;
          const has = (k: string) => p.permissions.includes(k);
          const flip = (k: string) => setDraft({ ...p, permissions: has(k) ? p.permissions.filter((x: string) => x !== k) : [...p.permissions, k] });
          const byScope = (scope: string) => d.catalog.filter((c: any) => c.scope === scope && (!filter || `${c.key} ${c.description}`.toLowerCase().includes(filter.toLowerCase())));
          const roleHas = (k: string) => d.role_global.includes(k) || d.role_global.includes("*") || d.role_bindings.some((b: any) => (b.permissions ?? []).includes(k) || (b.permissions ?? []).includes("*"));
          const unrestricted = p.mode === "unrestricted";
          return (
            <div className="stack loose" style={{ maxWidth: 920 }}>
              <header className="row wrap" style={{ gap: 14 }}>
                <Avatar person={agent} size="lg" />
                <div className="stack tight grow">
                  <h1 className="row wrap" style={{ gap: 8 }}>
                    Rules for {agent?.name ?? "this agent"} <AgentBadge person={agent ?? { id: "", name: "", kind: "agent" }} />
                  </h1>
                  <p className="sub">An AI agent can do something only if one of its roles grants it <em>and</em> these rules allow it. A new agent is allowed nothing.</p>
                </div>
              </header>
              <section className="panel">
                <header>
                  <h2>How much the rules allow</h2>
                </header>
                <div className="body stack">
                  <label className="check">
                    <input type="radio" name="mode" checked={!unrestricted} onChange={() => setDraft({ ...p, mode: "restricted" })} />
                    <span>
                      <strong>Only what is selected below</strong> (recommended)
                    </span>
                  </label>
                  <label className="check">
                    <input type="radio" name="mode" checked={unrestricted} disabled={!can("admin.security")} onChange={() => setDraft({ ...p, mode: "unrestricted" })} />
                    <span>
                      <strong>Everything its roles grant</strong>, like a person with the same roles
                    </span>
                  </label>
                  {unrestricted && (
                    <div className="notice warn">
                      <ShieldAlert />
                      <span>No rules apply. The agent can do whatever its roles allow, in every project and space those roles cover. Use this only for an integration you operate yourself, such as directory provisioning.</span>
                    </div>
                  )}
                  {!can("admin.security") && <p className="small muted">Lifting all rules needs an administrator who manages sign-in security.</p>}
                </div>
              </section>
              {!unrestricted && (
                <>
                  <section className="panel">
                    <header>
                      <h2>What it may do</h2>
                      <input className="input" style={{ maxWidth: 220 }} type="search" placeholder="Filter" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter permissions" />
                    </header>
                    <div className="body stack">
                      <div className="row wrap" role="group" aria-label="Start from">
                        <span className="small muted">Start from:</span>
                        {PRESETS.map((x) => (
                          <button key={x.label} type="button" className="btn sm" title={x.hint} onClick={() => setDraft({ ...p, permissions: x.permissions.filter((k) => d.catalog.some((c: any) => c.key === k)) })}>
                            {x.label}
                          </button>
                        ))}
                      </div>
                      {[["global", "Across the workspace"], ["project", "In projects"], ["space", "In documentation spaces"]].map(([scope, title]) => {
                        const list = byScope(scope);
                        if (list.length === 0) return null;
                        return (
                          <fieldset key={scope} className="perm-group">
                            <legend>{title}</legend>
                            {list.map((c: any) => (
                              <label key={c.key} className={`check perm ${has(c.key) && !roleHas(c.key) ? "unbacked" : ""}`}>
                                <input type="checkbox" checked={has(c.key)} onChange={() => flip(c.key)} />
                                <span>
                                  <code className="small">{c.key}</code> <span className="small muted">{c.description}</span>
                                  {has(c.key) && !roleHas(c.key) && <span className="small warn-text"> Allowed here, but none of its roles grants it, so it has no effect yet.</span>}
                                </span>
                              </label>
                            ))}
                          </fieldset>
                        );
                      })}
                    </div>
                  </section>
                  <section className="panel">
                    <header>
                      <h2>Where</h2>
                    </header>
                    <div className="body stack">
                      <p className="small muted">The agent also needs a role in each project or space, given there like for anyone else. This narrows it further.</p>
                      <div className="form-grid">
                        <div className="stack tight">
                          <label className="check">
                            <input type="checkbox" checked={p.all_projects} onChange={(e) => setDraft({ ...p, all_projects: e.target.checked })} /> Every project it has a role in
                          </label>
                          {!p.all_projects && (
                            <div className="pick-list" role="group" aria-label="Projects">
                              {(projects.data ?? []).map((x: any) => (
                                <label key={x.id} className="check">
                                  <input type="checkbox" checked={p.projects.includes(x.id)} onChange={() => setDraft({ ...p, projects: p.projects.includes(x.id) ? p.projects.filter((y: string) => y !== x.id) : [...p.projects, x.id] })} /> <span className="key">{x.key}</span> {x.name}
                                </label>
                              ))}
                              {(projects.data ?? []).length === 0 && <span className="small muted">No projects yet.</span>}
                            </div>
                          )}
                        </div>
                        <div className="stack tight">
                          <label className="check">
                            <input type="checkbox" checked={p.all_spaces} onChange={(e) => setDraft({ ...p, all_spaces: e.target.checked })} /> Every space it has a role in
                          </label>
                          {!p.all_spaces && (
                            <div className="pick-list" role="group" aria-label="Spaces">
                              {(spaces.data ?? []).map((x: any) => (
                                <label key={x.id} className="check">
                                  <input type="checkbox" checked={p.spaces.includes(x.id)} onChange={() => setDraft({ ...p, spaces: p.spaces.includes(x.id) ? p.spaces.filter((y: string) => y !== x.id) : [...p.spaces, x.id] })} /> <span className="key">{x.key}</span> {x.name}
                                </label>
                              ))}
                              {(spaces.data ?? []).length === 0 && <span className="small muted">No spaces yet.</span>}
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                  </section>
                </>
              )}
              <section className="panel">
                <header>
                  <h2>How fast</h2>
                </header>
                <div className="body">
                  <Field label="Requests per minute" hint="Empty uses the limit for all API clients. A lower number contains an agent stuck in a loop.">
                    <input className="input" style={{ maxWidth: 200 }} type="number" min={1} max={100000} value={p.rate_limit_per_minute ?? ""} onChange={(e) => setDraft({ ...p, rate_limit_per_minute: e.target.value ? Number(e.target.value) : null })} />
                  </Field>
                </div>
              </section>
              <section className="panel">
                <header>
                  <h2>Its roles</h2>
                </header>
                <div className="body stack tight">
                  {d.role_bindings.length === 0 && <p className="muted">This agent has no role anywhere, so it can do nothing whatever the rules say. Give it a role under Users and agents, or in a project or space.</p>}
                  {d.role_bindings.map((b: any, i: number) => (
                    <div key={i} className="row wrap small">
                      <span className="badge">{b.role_name}</span> <span className="muted">in</span> {b.label}
                    </div>
                  ))}
                  <p className="small muted">The agent protocol (MCP) lists to this agent only the tools these rules and roles allow.</p>
                </div>
              </section>
              <div className="row">
                <button className="btn primary" disabled={busy || !draft} onClick={() => run(() => api("PUT", `/api/v1/admin/users/${id}/agent-policy`, { mode: p.mode, permissions: p.permissions, all_projects: p.all_projects, projects: p.projects, all_spaces: p.all_spaces, spaces: p.spaces, rate_limit_per_minute: p.rate_limit_per_minute ?? null }), "Rules saved. They apply from the agent's next request.").then((r) => r !== undefined && setDraft(null))}>
                  Save rules
                </button>
                {draft && (
                  <button className="btn" onClick={() => setDraft(null)}>
                    Discard changes
                  </button>
                )}
                <span className="small muted row">
                  <Bot width={14} height={14} aria-hidden="true" /> Every change to an agent's rules is recorded in the audit log.
                </span>
              </div>
            </div>
          );
        }}
      </Loaded>
    </>
  );
}

// ───────────────────────── SAML ─────────────────────────

interface SamlForm {
  name: string;
  issuer: string;
  sso_url: string;
  certificates: string;
  email_attribute: string;
  name_attribute: string;
  allowed_domains: string;
  default_role: string;
  auto_provision: boolean;
}

/** Add a SAML provider, or change one: `existing` is a provider as the list returns it. */
export function SamlDialog({ onClose, existing }: { onClose: () => void; existing?: any }) {
  const [busy, run] = useAction();
  const [f, setF] = useState<SamlForm>(() => ({
    name: existing?.name ?? "",
    issuer: existing?.issuer ?? "",
    sso_url: existing?.config?.sso_url ?? "",
    certificates: ((existing?.config?.certificates ?? []) as string[]).join("\n"),
    email_attribute: existing?.config?.email_attribute ?? "",
    name_attribute: existing?.config?.name_attribute ?? "",
    allowed_domains: ((existing?.allowed_domains ?? []) as string[]).join(", "),
    default_role: existing?.default_role ?? "member",
    auto_provision: existing?.auto_provision ?? true,
  }));
  const [source, setSource] = useState({ url: "", xml: "" });
  const ready = f.name.trim() && f.issuer.trim() && f.sso_url.trim() && f.certificates.includes("BEGIN CERTIFICATE");
  const certs = useMemo(() => f.certificates.split(/(?=-----BEGIN CERTIFICATE-----)/).map((c) => c.trim()).filter(Boolean), [f.certificates]);
  async function read() {
    const r = await run(() => api("POST", "/api/v1/admin/auth-providers/saml-metadata", source.url.trim() ? { url: source.url.trim() } : { xml: source.xml }), "Metadata read. Check the values below.");
    if (r) setF((x) => ({ ...x, issuer: r.issuer ?? x.issuer, sso_url: r.sso_url ?? x.sso_url, certificates: (r.certificates ?? []).join("\n") || x.certificates }));
  }
  return (
    <Dialog
      title={existing ? `Change ${existing.name}` : "Add a SAML 2.0 provider"}
      wide
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn primary"
            disabled={busy || !ready}
            onClick={() =>
              run(
                () => {
                  const body = {
                    name: f.name, issuer: f.issuer, default_role: f.default_role, auto_provision: f.auto_provision,
                    allowed_domains: f.allowed_domains.split(",").map((d) => d.trim()).filter(Boolean),
                    // When changing a provider, an emptied attribute name is sent as empty so that it is cleared.
                    config: { sso_url: f.sso_url, certificates: certs, ...(f.email_attribute.trim() || existing ? { email_attribute: f.email_attribute.trim() } : {}), ...(f.name_attribute.trim() || existing ? { name_attribute: f.name_attribute.trim() } : {}) },
                  };
                  return existing ? api("PATCH", `/api/v1/admin/auth-providers/${existing.id}`, body) : api("POST", "/api/v1/admin/auth-providers", { kind: "saml", ...body });
                },
                existing ? "Provider saved" : "Provider added. Give your identity provider the addresses shown on its card.",
              ).then((r) => r !== undefined && onClose())
            }
          >
            {existing ? "Save changes" : "Add provider"}
          </button>
        </>
      }
    >
      <div className="stack">
        <section className="stack tight">
          <h3>{existing ? "Read the identity provider's metadata again" : "Start from the identity provider's metadata"}</h3>
          <div className="form-grid">
            <Field label="Metadata address">
              <input className="input" type="url" value={source.url} onChange={(e) => setSource({ ...source, url: e.target.value })} placeholder="https://idp.example.org/metadata" />
            </Field>
            <Field label="Or paste the metadata XML">
              <textarea className="textarea" rows={2} value={source.xml} onChange={(e) => setSource({ ...source, xml: e.target.value })} />
            </Field>
          </div>
          <div>
            <button className="btn" disabled={busy || (!source.url.trim() && !source.xml.trim())} onClick={() => void read()}>
              Read metadata
            </button>
          </div>
        </section>
        <Field label="Button label" required hint="People see “Continue with …” on the sign-in page.">
          <input className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} placeholder="Company account" />
        </Field>
        <div className="form-grid">
          <Field label="Identity provider entity ID" required>
            <input className="input" value={f.issuer} onChange={(e) => setF({ ...f, issuer: e.target.value })} />
          </Field>
          <Field label="Sign-on address (HTTP-Redirect)" required>
            <input className="input" type="url" value={f.sso_url} onChange={(e) => setF({ ...f, sso_url: e.target.value })} />
          </Field>
        </div>
        <Field label="Signing certificates" required hint={`PEM, one after another. ${certs.length} found. Responses must be signed with one of these (RSA, SHA-256 or stronger). While the identity provider changes certificates, keep the old one and the new one here.`}>
          <textarea className="textarea" rows={5} style={{ fontFamily: "var(--mono)", fontSize: 12 }} value={f.certificates} onChange={(e) => setF({ ...f, certificates: e.target.value })} placeholder="-----BEGIN CERTIFICATE-----" />
        </Field>
        <div className="form-grid">
          <Field label="Attribute holding the email address" hint="Empty: the NameID, when it is an email address.">
            <input className="input" value={f.email_attribute} onChange={(e) => setF({ ...f, email_attribute: e.target.value })} placeholder="mail" />
          </Field>
          <Field label="Attribute holding the display name">
            <input className="input" value={f.name_attribute} onChange={(e) => setF({ ...f, name_attribute: e.target.value })} placeholder="displayName" />
          </Field>
        </div>
        <Field label="Allowed email domains" hint="Comma separated. Leave empty to allow any account the provider accepts.">
          <input className="input" value={f.allowed_domains} onChange={(e) => setF({ ...f, allowed_domains: e.target.value })} placeholder="example.org" />
        </Field>
        <label className="check">
          <input type="checkbox" checked={f.auto_provision} onChange={(e) => setF({ ...f, auto_provision: e.target.checked })} /> Create an account the first time someone signs in
        </label>
        <p className="small muted">Not supported: encrypted assertions, signed requests and single log-out. AI agent accounts can never sign in this way.</p>
      </div>
    </Dialog>
  );
}
