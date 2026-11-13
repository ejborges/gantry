// Administration console. Each section checks the permission it needs; the server enforces the same rules.
import { useState } from "react";
import { Link, NavLink, Navigate, Route, Routes, useNavigate } from "react-router-dom";
import { Activity, BellRing, Bot, KeyRound, ListChecks, Lock, Mail, Pencil, Plug, Plus, Puzzle, RefreshCw, ScrollText, Settings, Shapes, ShieldAlert, ShieldCheck, Trash2, UserCog, UsersRound } from "lucide-react";
import { api, qs, useGet } from "../api";
import { useSession } from "../session";
import { AgentBadge, Avatar, Confirm, CopyButton, Dialog, Empty, Field, Loaded, Menu, useAction } from "../ui";
import { AgentRules, Alerts, NotificationSettings, SamlDialog, SiteSettings } from "./AdminMore";
import { bytes, dateTime, duration, relative, titleCase } from "../lib/format";
import { TokenDialog, TokenTable } from "./Profile";
import { Addons, Integrations, Schemes } from "./AdminConfig";

const SECTIONS = [
  { group: "System" },
  { to: "overview", label: "Overview", icon: Activity, perm: "admin.access" },
  { to: "jobs", label: "Background jobs", icon: ListChecks, perm: "admin.access" },
  { to: "settings", label: "Settings", icon: Settings, perm: "admin.access" },
  { to: "notifications", label: "Notifications and email", icon: Mail, perm: "admin.access" },
  { to: "alerts", label: "Alerts", icon: BellRing, perm: "admin.access" },
  { group: "People and access" },
  { to: "users", label: "Users and agents", icon: UserCog, perm: "admin.users" },
  { to: "groups", label: "Groups", icon: UsersRound, perm: "admin.users" },
  { to: "roles", label: "Roles", icon: ShieldCheck, perm: "admin.access" },
  { to: "sign-in", label: "Sign-in", icon: KeyRound, perm: "admin.security" },
  { group: "Configuration" },
  { to: "schemes", label: "Types and workflows", icon: Shapes, perm: "admin.schemes" },
  { to: "add-ons", label: "Add-ons", icon: Puzzle, perm: "admin.schemes" },
  { to: "integrations", label: "Integrations", icon: Plug, perm: "admin.integrations" },
  { group: "Records" },
  { to: "audit", label: "Audit log", icon: ScrollText, perm: "admin.audit" },
  { to: "security", label: "Security events", icon: ShieldAlert, perm: "admin.security" },
] as const;

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

const BACKENDS: Record<string, string> = {
  fs: "Local file system", s3: "S3-compatible object store", pg: "PostgreSQL", opensearch: "OpenSearch", pg_array: "PostgreSQL (exact search)", pgvector: "pgvector (HNSW index)",
  valkey: "Valkey", migrations: "Built-in migrations", off: "Off (keyword search only)",
};

// ───────── overview ─────────
function Overview() {
  const q = useGet<any>("/api/v1/admin/overview", { refetchInterval: 30_000 });
  return (
    <>
      <Head title="Overview" sub="Health of every component, refreshed every 30 seconds." />
      <Loaded q={q}>
        {(o) => {
          const bad = o.components.filter((c: any) => !c.status.ok);
          return (
            <div className="stack loose">
              {bad.length > 0 && (
                <div className="notice danger" role="alert">
                  <ShieldAlert />
                  <span>
                    {bad.length} component{bad.length > 1 ? "s need" : " needs"} attention: {bad.map((c: any) => c.name).join(", ")}. See the system administrator guide for what to check.
                  </span>
                </div>
              )}
              <div className="stat-row">
                {[
                  ["People", o.counts.users],
                  ["AI agents", o.counts.agents],
                  ["Projects", o.counts.projects],
                  ["Work items", o.counts.items],
                  ["Pages", o.counts.pages],
                  ["Signed in now", o.counts.sessions],
                ].map(([l, n]) => (
                  <div key={l} className="stat">
                    <div className="n">{n}</div>
                    <div className="l">{l}</div>
                  </div>
                ))}
                <div className={`stat ${o.counts.jobs_failed ? "alert" : ""}`}>
                  <div className="n">{o.counts.jobs_failed}</div>
                  <div className="l">Failed background jobs</div>
                </div>
                <div className={`stat ${o.counts.failed_logins_24h > 20 ? "warn" : ""}`}>
                  <div className="n">{o.counts.failed_logins_24h}</div>
                  <div className="l">Failed sign-ins, 24 h</div>
                </div>
              </div>
              <section className="panel">
                <header>
                  <h2>Components</h2>
                  <span className="small muted">
                    Version {o.version}, node {o.node}, up {duration(o.uptime_seconds * 1000)}, database {bytes(o.counts.database_bytes)}
                  </span>
                </header>
                <div className="table-wrap">
                  <table className="table">
                    <thead>
                      <tr>
                        <th>Component</th>
                        <th>Provided by</th>
                        <th>State</th>
                      </tr>
                    </thead>
                    <tbody>
                      {o.components.map((c: any) => (
                        <tr key={c.name}>
                          <td>{c.name}</td>
                          <td className="muted">{BACKENDS[c.backend] ?? c.backend}</td>
                          <td>
                            <span className={`health ${c.status.ok ? "" : "bad"}`}>{c.status.ok ? "Healthy" : c.status.error}</span>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </section>
              {(o.nodes?.length ?? 0) > 0 && (
                <section className="panel">
                  <header>
                    <h2>Nodes</h2>
                    <span className="small muted">Every server and worker process that reported in during the last day</span>
                  </header>
                  <div className="table-wrap">
                    <table className="table">
                      <thead>
                        <tr>
                          <th scope="col">Node</th>
                          <th scope="col">Role</th>
                          <th scope="col">Version</th>
                          <th scope="col">Running since</th>
                          <th scope="col">State</th>
                        </tr>
                      </thead>
                      <tbody>
                        {o.nodes.map((n: any) => (
                          <tr key={n.node}>
                            <td>
                              <code>{n.node}</code>
                              {n.node === o.node && <span className="small muted"> (answered this request)</span>}
                            </td>
                            <td>{n.role === "server" ? "Server" : n.role === "worker" ? "Worker" : n.role}</td>
                            <td className={n.version === o.version ? "" : "warn-text"}>
                              {n.version}
                              {n.version !== o.version && " (differs from this node)"}
                            </td>
                            <td className="nowrap">{dateTime(n.started_at)}</td>
                            <td>
                              <span className={`health ${n.alive ? "" : "bad"}`}>{n.alive ? "Reporting" : `Silent since ${relative(n.seen_at)}`}</span>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </section>
              )}
              {!o.public_url && (
                <div className="notice warn">
                  <span>
                    <strong>server.public_url is not set.</strong> Links in chat notifications will be relative and cookies are not marked Secure. Set it in /etc/gantry/gantry.toml and restart.
                  </span>
                </div>
              )}
            </div>
          );
        }}
      </Loaded>
    </>
  );
}

// ───────── users ─────────
function Users() {
  const { can } = useSession();
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState("");
  const q = useGet<{ users: any[]; total: number }>(`/api/v1/admin/users${qs({ q: search, kind, limit: 200 })}`, { placeholderData: (p) => p });
  const roles = useGet<{ roles: any[] }>("/api/v1/admin/roles");
  const [busy, run] = useAction();
  const [creating, setCreating] = useState<"human" | "agent" | null>(null);
  const [form, setForm] = useState({ email: "", name: "", password: "", role: "member", description: "" });
  const navigate = useNavigate();
  const [tokensFor, setTokensFor] = useState<any | null>(null);
  const [newToken, setNewToken] = useState<any | null>(null);
  const [profileOf, setProfileOf] = useState<any | null>(null);
  const tokens = useGet<any[]>(tokensFor ? `/api/v1/admin/users/${tokensFor.id}/tokens` : null);
  const globalRoles = (roles.data?.roles ?? []).filter((r) => r.scope === "global");
  const patch = (id: string, body: object, msg: string) => run(() => api("PATCH", `/api/v1/admin/users/${id}`, body), msg);
  return (
    <>
      <Head title="Users and agents" sub="People sign in. AI agents and scripts act through API tokens, are marked as agents everywhere, and can do only what their rules allow: nothing, until you write them.">
        <button className="btn" onClick={() => (setForm({ email: "", name: "", password: "", role: "member", description: "" }), setCreating("agent"))}>
          <Bot /> New agent
        </button>
        <button className="btn primary" onClick={() => (setForm({ email: "", name: "", password: "", role: "member", description: "" }), setCreating("human"))}>
          <Plus /> New user
        </button>
      </Head>
      <div className="filters">
        <input className="input search" placeholder="Search by name or email" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Search users" />
        <select className="select" value={kind} onChange={(e) => setKind(e.target.value)} aria-label="Kind">
          <option value="">People and agents</option>
          <option value="human">People</option>
          <option value="agent">AI agents</option>
        </select>
      </div>
      <div className="panel">
        <Loaded q={q}>
          {(d) => (
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th>Name</th>
                    <th>Global roles</th>
                    <th className="hide-sm">Last sign-in</th>
                    <th>State</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {d.users.map((u) => (
                    <tr key={u.id}>
                      <td>
                        <div className="row">
                          <Avatar person={u} />
                          <div className="stack" style={{ gap: 0 }}>
                            <span className="row wrap" style={{ gap: 6 }}>
                              <Link to={`/people/${u.id}`}>
                                <strong>{u.name}</strong>
                              </Link>
                              <AgentBadge person={u} />
                              {u.kind === "agent" && (u.agent_mode === "unrestricted" ? <span className="badge amber">No rules</span> : null)}
                            </span>
                            <span className="small muted">
                              {u.kind === "agent" ? `${u.active_tokens} active token${u.active_tokens === 1 ? "" : "s"}${u.owner ? ` · responsible: ${u.owner.name}` : ""}` : u.email}
                              {u.job_title ? ` · ${u.job_title}` : ""}
                            </span>
                          </div>
                        </div>
                      </td>
                      <td>
                        <div className="row wrap" style={{ gap: 4 }}>
                          {u.roles.map((r: any) => (
                            <span key={r.key} className={`badge ${r.key === "system_admin" ? "red" : r.key.includes("admin") || r.key === "auditor" ? "amber" : ""}`}>
                              {r.name}
                            </span>
                          ))}
                          {u.groups.map((g: any) => (
                            <span key={g.id} className="badge outline">
                              {g.name}
                            </span>
                          ))}
                        </div>
                      </td>
                      <td className="hide-sm muted">{u.kind === "agent" ? "–" : u.last_login_at ? relative(u.last_login_at) : "Never"}</td>
                      <td>{!u.active ? <span className="badge red">Deactivated</span> : u.locked ? <span className="badge amber"><Lock /> Locked</span> : <span className="badge green">Active</span>}</td>
                      <td style={{ textAlign: "right" }}>
                        <Menu trigger={(_, t) => <button className="btn sm" onClick={t}>Manage</button>}>
                          {(close) => (
                            <>
                              <div className="menu-label">Add role</div>
                              {globalRoles.filter((r) => !u.roles.some((x: any) => x.key === r.key)).map((r) => (
                                <button key={r.key} onClick={() => (close(), run(() => api("POST", `/api/v1/admin/users/${u.id}/roles`, { role: r.key }), `${r.name} granted`))}>
                                  {r.name}
                                </button>
                              ))}
                              {u.roles.length > 0 && <div className="menu-label">Remove role</div>}
                              {u.roles.map((r: any) => (
                                <button key={r.key} onClick={() => (close(), run(() => api("DELETE", `/api/v1/admin/users/${u.id}/roles/${r.key}`), `${r.name} removed`))}>
                                  {r.name}
                                </button>
                              ))}
                              <hr />
                              <button onClick={() => (close(), setProfileOf({ id: u.id, name: u.name, kind: u.kind, handle: u.handle ?? "", job_title: u.job_title ?? "", org_name: u.org_name ?? "", org_code: u.org_code ?? "", description: u.description ?? "", has_avatar: u.has_avatar }))}>Edit profile</button>
                              {u.kind === "agent" && can("admin.agents") && (
                                <Link to={`/admin/agents/${u.id}`} onClick={close}>
                                  Rules: what it may do
                                </Link>
                              )}
                              {u.kind === "agent" && can("admin.security") && <button onClick={() => (close(), setTokensFor(u))}>API tokens</button>}
                              {u.locked && <button onClick={() => (close(), patch(u.id, { unlock: true }, "Account unlocked"))}>Unlock</button>}
                              {u.kind === "human" && <button onClick={() => (close(), run(() => api("POST", `/api/v1/admin/users/${u.id}/sessions/revoke`), "Signed out on all devices"))}>Sign out everywhere</button>}
                              <button onClick={() => (close(), patch(u.id, { active: !u.active }, u.active ? "Deactivated. Sessions and tokens were ended." : "Reactivated"))}>{u.active ? "Deactivate" : "Reactivate"}</button>
                            </>
                          )}
                        </Menu>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Loaded>
      </div>
      {creating && (
        <Dialog
          title={creating === "agent" ? "New AI agent" : "New user"}
          onClose={() => setCreating(null)}
          footer={
            <>
              <button className="btn" onClick={() => setCreating(null)}>
                Cancel
              </button>
              <button
                className="btn primary"
                disabled={busy || !form.name.trim() || (creating === "human" && !form.email.includes("@"))}
                onClick={() =>
                  run(() => api("POST", "/api/v1/admin/users", creating === "agent" ? { kind: "agent", name: form.name, role: form.role, description: form.description || undefined } : { email: form.email, name: form.name, role: form.role, password: form.password || undefined }), creating === "agent" ? "Agent created. It can do nothing yet: write its rules, give it roles, then create a token." : "User created").then((r: any) => {
                    if (!r) return;
                    setCreating(null);
                    if (creating === "agent" && r.id && can("admin.agents")) navigate(`/admin/agents/${r.id}`);
                  })
                }
              >
                Create {creating === "agent" ? "agent" : "user"}
              </button>
            </>
          }
        >
          <div className="stack">
            <Field label="Name" required hint={creating === "agent" ? "Shown wherever the agent acts, with an “AI agent” marker." : undefined}>
              <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder={creating === "agent" ? "Triage agent" : "Ada Lovelace"} />
            </Field>
            {creating === "agent" && (
              <>
                <Field label="What this agent is for" hint="Shown on its profile. You are recorded as the person responsible for it.">
                  <textarea className="textarea" rows={2} maxLength={2000} value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} placeholder="Sorts new requests and suggests an assignee" />
                </Field>
                <div className="notice">
                  <Bot />
                  <span>A new agent is allowed nothing. After creating it you choose exactly what it may read and change, and where.</span>
                </div>
              </>
            )}
            {creating === "human" && (
              <>
                <Field label="Email" required>
                  <input className="input" type="email" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} />
                </Field>
                <Field label="Initial password" hint="At least 12 characters. Leave empty for people who will use single sign-on.">
                  <input className="input" type="password" autoComplete="new-password" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} />
                </Field>
              </>
            )}
            <Field label="Global role" hint="Access to individual projects and spaces is granted in each project or space.">
              <select className="select" value={form.role} onChange={(e) => setForm({ ...form, role: e.target.value })}>
                {globalRoles.map((r) => (
                  <option key={r.key} value={r.key}>
                    {r.name}
                  </option>
                ))}
              </select>
            </Field>
          </div>
        </Dialog>
      )}
      {tokensFor && !newToken && (
        <Dialog title={`API tokens for ${tokensFor.name}`} wide onClose={() => setTokensFor(null)} footer={<button className="btn primary" onClick={() => setNewToken(tokensFor)}>New token</button>}>
          <Loaded q={tokens}>{(list) => <TokenTable tokens={list} onRevoke={(id) => run(() => api("DELETE", `/api/v1/tokens/${id}`), "Token revoked")} />}</Loaded>
        </Dialog>
      )}
      {newToken && <TokenDialog owner={newToken} allowAdmin onClose={() => setNewToken(null)} />}
      {profileOf && (
        <Dialog
          title={`Profile of ${profileOf.name}`}
          onClose={() => setProfileOf(null)}
          footer={
            <>
              {profileOf.has_avatar && (
                <button className="btn" style={{ marginRight: "auto" }} disabled={busy} onClick={() => patch(profileOf.id, { remove_avatar: true }, "Picture removed").then(() => setProfileOf({ ...profileOf, has_avatar: false }))}>
                  Remove picture
                </button>
              )}
              <button className="btn" onClick={() => setProfileOf(null)}>
                Cancel
              </button>
              <button
                className="btn primary"
                disabled={busy || !profileOf.name.trim()}
                onClick={() => patch(profileOf.id, { name: profileOf.name, handle: profileOf.handle, job_title: profileOf.job_title, org_name: profileOf.org_name, org_code: profileOf.org_code, ...(profileOf.kind === "agent" ? { description: profileOf.description } : {}) }, "Profile saved").then((r) => r !== undefined && setProfileOf(null))}
              >
                Save
              </button>
            </>
          }
        >
          <div className="stack">
            <div className="form-grid">
              <Field label="Display name" required>
                <input className="input" maxLength={200} value={profileOf.name} onChange={(e) => setProfileOf({ ...profileOf, name: e.target.value })} />
              </Field>
              <Field label="Handle" hint="Used to mention the account: lower-case letters, digits, dot, dash, underscore.">
                <input className="input" maxLength={40} value={profileOf.handle} onChange={(e) => setProfileOf({ ...profileOf, handle: e.target.value.toLowerCase() })} />
              </Field>
              <Field label="Job title">
                <input className="input" maxLength={120} value={profileOf.job_title} onChange={(e) => setProfileOf({ ...profileOf, job_title: e.target.value })} />
              </Field>
              <Field label="Organization">
                <input className="input" maxLength={120} value={profileOf.org_name} onChange={(e) => setProfileOf({ ...profileOf, org_name: e.target.value })} />
              </Field>
              <Field label="Organization code">
                <input className="input" maxLength={40} value={profileOf.org_code} onChange={(e) => setProfileOf({ ...profileOf, org_code: e.target.value })} />
              </Field>
            </div>
            {profileOf.kind === "agent" && (
              <Field label="What this agent is for" hint="Shown on its profile, so people know what is acting on their work.">
                <textarea className="textarea" rows={3} maxLength={2000} value={profileOf.description} onChange={(e) => setProfileOf({ ...profileOf, description: e.target.value })} />
              </Field>
            )}
            <p className="small muted">Which of these people may change themselves is set under Settings, Profiles.</p>
          </div>
        </Dialog>
      )}
    </>
  );
}

// ───────── groups ─────────
function Groups() {
  const groups = useGet<any[]>("/api/v1/admin/groups");
  const users = useGet<{ users: any[] }>("/api/v1/admin/users?limit=200&kind=human");
  const [busy, run] = useAction();
  const [name, setName] = useState("");
  return (
    <>
      <Head title="Groups" sub="Give a role to a group once instead of to each person. Identity providers can maintain groups through SCIM." />
      <div className="stack">
        <div className="row">
          <input className="input" style={{ maxWidth: 280 }} placeholder="Propulsion team" value={name} onChange={(e) => setName(e.target.value)} aria-label="New group name" />
          <button className="btn primary" disabled={!name.trim() || busy} onClick={() => run(() => api("POST", "/api/v1/admin/groups", { name }), "Group created").then(() => setName(""))}>
            <Plus /> Create group
          </button>
        </div>
        <Loaded q={groups}>
          {(list) =>
            list.length === 0 ? (
              <div className="panel">
                <Empty icon={<UsersRound />} title="No groups yet" />
              </div>
            ) : (
              list.map((g) => (
                <section key={g.id} className="panel">
                  <header>
                    <h2 className="row">
                      {g.name} {g.provisioned && <span className="badge outline">From identity provider</span>}
                      {g.roles.map((r: any) => (
                        <span key={r.key} className="badge amber">{r.name}</span>
                      ))}
                    </h2>
                    <button className="btn ghost icon sm" aria-label={`Delete ${g.name}`} onClick={() => run(() => api("DELETE", `/api/v1/admin/groups/${g.id}`), "Group deleted")}>
                      <Trash2 />
                    </button>
                  </header>
                  <div className="body row wrap">
                    {g.members.map((m: any) => (
                      <span key={m.id} className="badge">
                        {m.name}
                        <button className="btn ghost icon sm" style={{ width: 18, height: 18, minHeight: 0 }} aria-label={`Remove ${m.name} from ${g.name}`} onClick={() => run(() => api("DELETE", `/api/v1/admin/groups/${g.id}/members/${m.id}`))}>
                          ×
                        </button>
                      </span>
                    ))}
                    <select className="select" style={{ width: "auto" }} value="" aria-label={`Add a member to ${g.name}`} onChange={(e) => e.target.value && run(() => api("POST", `/api/v1/admin/groups/${g.id}/members`, { user_id: e.target.value }))}>
                      <option value="">Add a member…</option>
                      {(users.data?.users ?? []).filter((u) => u.active && !g.members.some((m: any) => m.id === u.id)).map((u) => (
                        <option key={u.id} value={u.id}>
                          {u.name}
                        </option>
                      ))}
                    </select>
                  </div>
                </section>
              ))
            )
          }
        </Loaded>
      </div>
    </>
  );
}

// ───────── roles ─────────
function Roles() {
  const { can } = useSession();
  const q = useGet<{ roles: any[]; permissions: { key: string; scope: string; description: string }[] }>("/api/v1/admin/roles");
  const [busy, run] = useAction();
  const [editing, setEditing] = useState<any | null>(null);
  return (
    <>
      <Head title="Roles" sub="A role is a named set of permissions. Built-in roles are fixed; add custom roles for anything else.">
        {can("admin.roles") && (
          <button className="btn primary" onClick={() => setEditing({ isNew: true, key: "", name: "", scope: "project", permissions: [], description: "" })}>
            <Plus /> New role
          </button>
        )}
      </Head>
      <Loaded q={q}>
        {(d) => (
          <div className="stack loose">
            {(["global", "project", "space"] as const).map((scope) => (
              <section key={scope} className="panel">
                <header>
                  <h2>{scope === "global" ? "Workspace-wide roles" : scope === "project" ? "Project roles" : "Space roles"}</h2>
                </header>
                {d.roles.filter((r) => r.scope === scope).map((r) => (
                  <div key={r.key} className="list-row" style={{ alignItems: "flex-start" }}>
                    <div className="grow stack tight">
                      <span className="row">
                        <strong>{r.name}</strong>
                        {!r.builtin && <span className="badge blue">Custom</span>}
                      </span>
                      <span className="small muted">{r.description}</span>
                      <span className="small">{r.permissions.includes("*") ? "Every permission" : r.permissions.length === 0 ? "No permissions on its own" : r.permissions.join(", ")}</span>
                    </div>
                    {!r.builtin && can("admin.roles") && (
                      <span className="row">
                        <button className="btn sm" onClick={() => setEditing(r)}>
                          Edit
                        </button>
                        <button className="btn ghost icon sm" aria-label={`Delete ${r.name}`} onClick={() => run(() => api("DELETE", `/api/v1/admin/roles/${r.key}`), "Role deleted")}>
                          <Trash2 />
                        </button>
                      </span>
                    )}
                  </div>
                ))}
              </section>
            ))}
            {editing && (
              <Dialog
                title={editing.isNew ? "New role" : `Edit ${editing.name}`}
                wide
                onClose={() => setEditing(null)}
                footer={
                  <>
                    <button className="btn" onClick={() => setEditing(null)}>
                      Cancel
                    </button>
                    <button className="btn primary" disabled={busy || !editing.name.trim() || (editing.isNew && !/^[a-z][a-z0-9_]+$/.test(editing.key))} onClick={() => run(() => api(editing.isNew ? "POST" : "PUT", editing.isNew ? "/api/v1/admin/roles" : `/api/v1/admin/roles/${editing.key}`, editing), "Role saved").then((r) => r !== undefined && setEditing(null))}>
                      Save role
                    </button>
                  </>
                }
              >
                <div className="stack">
                  <div className="form-grid">
                    <Field label="Name" required>
                      <input className="input" value={editing.name} onChange={(e) => setEditing({ ...editing, name: e.target.value, key: editing.isNew ? e.target.value.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "") : editing.key })} />
                    </Field>
                    {editing.isNew && (
                      <Field label="Applies to">
                        <select className="select" value={editing.scope} onChange={(e) => setEditing({ ...editing, scope: e.target.value, permissions: [] })}>
                          <option value="project">One project at a time</option>
                          <option value="space">One documentation space at a time</option>
                          <option value="global">The whole workspace</option>
                        </select>
                      </Field>
                    )}
                  </div>
                  <Field label="Description">
                    <input className="input" value={editing.description} onChange={(e) => setEditing({ ...editing, description: e.target.value })} />
                  </Field>
                  <div className="stack tight">
                    <span className="label">Permissions</span>
                    <div className="perm-grid">
                      {d.permissions.filter((p) => editing.scope === "global" || p.scope === editing.scope).map((p) => (
                        <label key={p.key} className="check" style={{ alignItems: "flex-start" }}>
                          <input type="checkbox" checked={editing.permissions.includes(p.key)} onChange={(e) => setEditing({ ...editing, permissions: e.target.checked ? [...editing.permissions, p.key] : editing.permissions.filter((x: string) => x !== p.key) })} />
                          <span className="small">{p.description}</span>
                        </label>
                      ))}
                    </div>
                  </div>
                </div>
              </Dialog>
            )}
          </div>
        )}
      </Loaded>
    </>
  );
}

// ───────── sign-in ─────────
function SignIn() {
  const q = useGet<any[]>("/api/v1/admin/auth-providers");
  const [busy, run] = useAction();
  const blank = { name: "", issuer: "", client_id: "", client_secret: "", allowed_domains: "", default_role: "member", auto_provision: true };
  const [form, setForm] = useState<typeof blank | null>(null);
  // `true` adds a SAML provider; a provider object changes that one.
  const [saml, setSaml] = useState<any>(false);
  const [remove, setRemove] = useState<any | null>(null);
  return (
    <>
      <Head title="Sign-in" sub="Connect your identity provider with OpenID Connect or SAML 2.0. Works with Microsoft Entra ID, Okta, Keycloak, Ping, ADFS, Shibboleth, Google Workspace and others.">
        <button className="btn" onClick={() => setSaml(true)}>
          <Plus /> Add SAML provider
        </button>
        <button className="btn primary" onClick={() => setForm(blank)}>
          <Plus /> Add OpenID Connect provider
        </button>
      </Head>
      <div className="stack loose">
        <Loaded q={q}>
          {(list) =>
            list.length === 0 ? (
              <div className="panel">
                <Empty icon={<KeyRound />} title="No single sign-on provider yet">
                  <p>People currently sign in with an email and password stored in Gantry. Add a provider to use your organization's accounts and multi-factor rules.</p>
                </Empty>
              </div>
            ) : (
              list.map((p) => (
                <section key={p.id} className="panel">
                  <header>
                    <h2 className="row">
                      {p.name} <span className="badge outline">{p.kind === "saml" ? "SAML 2.0" : "OpenID Connect"}</span> {p.enabled ? <span className="badge green">On</span> : <span className="badge">Off</span>}
                    </h2>
                    <span className="row">
                      <button className="btn sm" disabled={busy} onClick={() => run(() => api("PATCH", `/api/v1/admin/auth-providers/${p.id}`, { enabled: !p.enabled }), p.enabled ? "Provider switched off" : "Provider switched on")}>
                        {p.enabled ? "Turn off" : "Turn on"}
                      </button>
                      {p.kind === "saml" && (
                        <button className="btn sm" onClick={() => setSaml(p)}>
                          <Pencil /> Change
                        </button>
                      )}
                      <button className="btn ghost icon sm" aria-label={`Remove ${p.name}`} onClick={() => setRemove(p)}>
                        <Trash2 />
                      </button>
                    </span>
                  </header>
                  <div className="body">
                    <dl className="props" style={{ gridTemplateColumns: "170px minmax(0, 1fr)" }}>
                      <dt>{p.kind === "saml" ? "Identity provider entity ID" : "Issuer"}</dt>
                      <dd className="truncate">{p.issuer}</dd>
                      {p.kind === "saml" ? (
                        <>
                          <dt>Sign-on address</dt>
                          <dd className="truncate">{p.config?.sso_url}</dd>
                          <dt>Reply URL (ACS)</dt>
                          <dd className="row">
                            <code className="truncate small">{p.saml?.acs_url}</code> <CopyButton text={p.saml?.acs_url ?? ""} />
                          </dd>
                          <dt>Our entity ID (audience)</dt>
                          <dd className="row">
                            <code className="truncate small">{p.saml?.sp_entity_id}</code> <CopyButton text={p.saml?.sp_entity_id ?? ""} />
                          </dd>
                          <dt>Our metadata</dt>
                          <dd className="truncate">
                            <a href={p.saml?.metadata_url} target="_blank" rel="noreferrer">
                              {p.saml?.metadata_url}
                            </a>
                          </dd>
                          <dt>Signing certificates</dt>
                          <dd>
                            {(p.saml?.certificate_expiry ?? []).map((e: string, i: number) => (
                              <span key={i} className={`badge ${new Date(e).getTime() < Date.now() + 30 * 86400_000 ? "amber" : ""}`} style={{ marginRight: 4 }}>
                                valid until {new Date(e).toLocaleDateString()}
                              </span>
                            ))}
                          </dd>
                        </>
                      ) : (
                        <>
                          <dt>Client ID</dt>
                          <dd className="truncate">{p.client_id}</dd>
                          <dt>Redirect URI</dt>
                          <dd className="row">
                            <code className="truncate small">{p.redirect_uri}</code> <CopyButton text={p.redirect_uri} />
                          </dd>
                        </>
                      )}
                      <dt>New accounts</dt>
                      <dd>{p.auto_provision ? `Created on first sign-in as ${titleCase(p.default_role)}` : "Must be created by an administrator first"}</dd>
                      <dt>Allowed email domains</dt>
                      <dd>{p.allowed_domains.length ? p.allowed_domains.join(", ") : "Any"}</dd>
                    </dl>
                  </div>
                </section>
              ))
            )
          }
        </Loaded>
        <section className="panel">
          <header>
            <h2>Automatic provisioning (SCIM 2.0)</h2>
          </header>
          <div className="body stack">
            <p>Your identity provider can create, update and deactivate accounts and groups. Create an agent with the <strong>Provisioning client</strong> role, give it a token allowed to administer, and enter these in the provider:</p>
            <dl className="props" style={{ gridTemplateColumns: "170px minmax(0, 1fr)" }}>
              <dt>Tenant URL</dt>
              <dd className="row">
                <code className="small">{location.origin}/scim/v2</code> <CopyButton text={`${location.origin}/scim/v2`} />
              </dd>
              <dt>Secret token</dt>
              <dd>The agent's API token</dd>
            </dl>
          </div>
        </section>
      </div>
      {form && (
        <Dialog
          title="Add an OpenID Connect provider"
          onClose={() => setForm(null)}
          footer={
            <>
              <button className="btn" onClick={() => setForm(null)}>
                Cancel
              </button>
              <button className="btn primary" disabled={busy || !form.name || !form.issuer || !form.client_id} onClick={() => run(() => api("POST", "/api/v1/admin/auth-providers", { ...form, allowed_domains: form.allowed_domains.split(",").map((d) => d.trim()).filter(Boolean) }), "Provider added. Register the redirect URI shown on its card with your identity provider.").then((r) => r && setForm(null))}>
                Check and add
              </button>
            </>
          }
        >
          <div className="stack">
            <Field label="Button label" required hint="People see “Continue with …” on the sign-in page.">
              <input className="input" value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="Company account" />
            </Field>
            <Field label="Issuer URL" required hint="Gantry reads /.well-known/openid-configuration from here and refuses to save if it cannot.">
              <input className="input" type="url" value={form.issuer} onChange={(e) => setForm({ ...form, issuer: e.target.value })} placeholder="https://login.microsoftonline.com/<tenant>/v2.0" />
            </Field>
            <div className="form-grid">
              <Field label="Client ID" required>
                <input className="input" value={form.client_id} onChange={(e) => setForm({ ...form, client_id: e.target.value })} />
              </Field>
              <Field label="Client secret" hint="Stored encrypted. Never shown again.">
                <input className="input" type="password" autoComplete="off" value={form.client_secret} onChange={(e) => setForm({ ...form, client_secret: e.target.value })} />
              </Field>
            </div>
            <Field label="Allowed email domains" hint="Comma separated. Leave empty to allow any account the provider accepts.">
              <input className="input" value={form.allowed_domains} onChange={(e) => setForm({ ...form, allowed_domains: e.target.value })} placeholder="example.org" />
            </Field>
            <label className="check">
              <input type="checkbox" checked={form.auto_provision} onChange={(e) => setForm({ ...form, auto_provision: e.target.checked })} /> Create an account the first time someone signs in
            </label>
          </div>
        </Dialog>
      )}
      {saml && <SamlDialog existing={saml === true ? undefined : saml} onClose={() => setSaml(false)} />}
      {remove && <Confirm title={`Remove ${remove.name}?`} action="Remove provider" danger body="People who only sign in through this provider will be locked out until another way is set up." onClose={() => setRemove(null)} onConfirm={async () => void (await run(() => api("DELETE", `/api/v1/admin/auth-providers/${remove.id}`), "Provider removed"))} />}
    </>
  );
}

// ───────── audit and security ─────────
function Audit() {
  const [f, setF] = useState({ actor: "", action: "", target: "" });
  const [offset, setOffset] = useState(0);
  const q = useGet<{ entries: any[]; total: number }>(`/api/v1/admin/audit${qs({ ...f, limit: 50, offset })}`, { placeholderData: (p) => p, staleTime: 5_000 });
  const [open, setOpen] = useState<any | null>(null);
  const [check, setCheck] = useState<any | null>(null);
  const [busy, run] = useAction();
  const filter = (k: keyof typeof f, v: string) => (setOffset(0), setF({ ...f, [k]: v }));
  return (
    <>
      <Head title="Audit log" sub="Every sign-in, change and administrative action, with who did it and from where. Entries cannot be edited or removed.">
        <button className="btn" disabled={busy} onClick={() => run(() => api("POST", "/api/v1/admin/audit/verify"), (r) => setCheck(r))}>
          <ShieldCheck /> Verify integrity
        </button>
      </Head>
      {check && (
        <div className={`notice ${check.intact ? "ok" : "danger"}`} role="status" style={{ marginBottom: 14 }}>
          {check.intact ? <ShieldCheck /> : <ShieldAlert />}
          <span>{check.intact ? `All ${check.entries} entries are intact: the hash chain is unbroken.` : `The log has been altered. The chain breaks at entry ${check.first_broken_id}. Treat this as a security incident; see the system administrator guide.`}</span>
        </div>
      )}
      <div className="filters">
        <input className="input" placeholder="Who (name or email)" value={f.actor} onChange={(e) => filter("actor", e.target.value)} aria-label="Filter by actor" />
        <input className="input" placeholder="Action, e.g. item. or auth.login" value={f.action} onChange={(e) => filter("action", e.target.value)} aria-label="Filter by action" />
        <input className="input" placeholder="Target, e.g. OPS-12" value={f.target} onChange={(e) => filter("target", e.target.value)} aria-label="Filter by target" />
      </div>
      <div className="panel">
        <Loaded q={q}>
          {(d) => (
            <>
              <div className="table-wrap" tabIndex={0} role="region" aria-label="Table: scroll sideways to see every column">
                <table className="table">
                  <thead>
                    <tr>
                      <th>When</th>
                      <th>Who</th>
                      <th>Action</th>
                      <th>Target</th>
                      <th className="hide-sm">From</th>
                      <th>Result</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.entries.map((e) => (
                      <tr key={e.id} className="click" onClick={() => setOpen(e)}>
                        <td className="nowrap small" title={dateTime(e.at)}>{relative(e.at)}</td>
                        <td>
                          <span className="truncate" title={e.actor_label}>{(e.actor_kind === "agent" && e.actor_name) || e.actor_label || "System"}</span> {e.actor_kind === "agent" && <span className="badge agent">AI agent</span>} {e.auth_method === "token" && e.actor_kind !== "agent" && <span className="badge outline">Token</span>}
                        </td>
                        <td>
                          <code className="small">{e.action}</code>
                        </td>
                        <td className="truncate" style={{ maxWidth: 200 }}>{e.target_id ? `${e.target_type} ${e.target_id}` : e.target_type}</td>
                        <td className="hide-sm small muted num">{e.ip}</td>
                        <td>{e.outcome === "success" ? <span className="badge green">OK</span> : <span className="badge red">Failed</span>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <div className="spread" style={{ padding: "10px 16px", borderTop: "1px solid var(--line)" }}>
                <span className="small muted num">
                  {d.total === 0 ? "No entries" : `${offset + 1}–${Math.min(offset + 50, d.total)} of ${d.total}`}
                </span>
                <span className="row">
                  <button className="btn sm" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - 50))}>
                    Newer
                  </button>
                  <button className="btn sm" disabled={offset + 50 >= d.total} onClick={() => setOffset(offset + 50)}>
                    Older
                  </button>
                </span>
              </div>
            </>
          )}
        </Loaded>
      </div>
      {open && (
        <Dialog title={`Audit entry ${open.id}`} onClose={() => setOpen(null)}>
          <dl className="props" style={{ gridTemplateColumns: "110px minmax(0, 1fr)", alignItems: "start" }}>
            <dt>When</dt>
            <dd>{dateTime(open.at)}</dd>
            <dt>Who</dt>
            <dd>
              {open.actor_name && open.actor_name !== open.actor_label ? `${open.actor_name}, ` : ""}
              {open.actor_label || "System"} ({open.actor_kind === "agent" ? "AI agent" : open.actor_kind === "human" ? "person" : open.actor_kind}
              {open.auth_method ? `, ${open.auth_method}` : ""})
            </dd>
            <dt>Action</dt>
            <dd><code>{open.action}</code></dd>
            <dt>Target</dt>
            <dd>{open.target_type} {open.target_id}</dd>
            <dt>Address</dt>
            <dd>{open.ip || "–"}</dd>
            <dt>Request ID</dt>
            <dd><code className="small">{open.request_id || "–"}</code></dd>
            <dt>Detail</dt>
            <dd><pre className="code">{JSON.stringify(open.detail, null, 2)}</pre></dd>
          </dl>
          <p className="small muted" style={{ marginTop: 12 }}>Search the server log for the request ID to see everything that happened during this request.</p>
        </Dialog>
      )}
    </>
  );
}

const EVENT_HELP: Record<string, string> = {
  "auth.login_failed": "Wrong password or unknown account",
  "auth.lockout": "Account locked after repeated failures",
  "auth.login_throttled": "Too many attempts from one address",
  "auth.token_rejected": "Invalid, expired or revoked API token",
  "auth.sso_failed": "Single sign-on did not complete",
  "authz.denied": "Someone tried something they are not permitted to do",
  "authz.read_only_token": "A read-only token tried to change something",
  "csrf.rejected": "Request without a valid anti-forgery token",
  "webhook.signature_invalid": "Inbound webhook with a bad signature",
  "rate_limit.exceeded": "Request rate limit reached",
  "mcp.origin_rejected": "MCP request from another website",
  "audit.chain_broken": "Audit log integrity check failed",
};

function SecurityEvents() {
  const [kind, setKind] = useState("");
  const q = useGet<{ events: any[]; total: number; last_24h: { kind: string; count: number }[] }>(`/api/v1/admin/security-events${qs({ kind, limit: 100 })}`, { refetchInterval: 30_000, placeholderData: (p) => p });
  return (
    <>
      <Head title="Security events" sub="Failed sign-ins, denied actions, rejected tokens and signatures. Review spikes and anything marked critical." />
      <Loaded q={q}>
        {(d) => (
          <div className="stack">
            <div className="row wrap">
              <button className={`btn sm ${kind === "" ? "primary" : ""}`} onClick={() => setKind("")}>
                All
              </button>
              {d.last_24h.map((s) => (
                <button key={s.kind} className={`btn sm ${kind === s.kind ? "primary" : ""}`} onClick={() => setKind(s.kind)} title={EVENT_HELP[s.kind]}>
                  {EVENT_HELP[s.kind] ?? s.kind} <span className="num">({s.count} in 24 h)</span>
                </button>
              ))}
            </div>
            <div className="panel">
              {d.events.length === 0 ? (
                <Empty icon={<ShieldCheck />} title="No security events">
                  <p>Nothing has been refused or rejected.</p>
                </Empty>
              ) : (
                <div className="table-wrap" tabIndex={0} role="region" aria-label="Table: scroll sideways to see every column">
                  <table className="table">
                    <thead>
                      <tr>
                        <th>When</th>
                        <th style={{ minWidth: 250 }}>What happened</th>
                        <th>Who</th>
                        <th className="hide-sm">From</th>
                        <th className="hide-sm">Detail</th>
                      </tr>
                    </thead>
                    <tbody>
                      {d.events.map((e) => (
                        <tr key={e.id}>
                          <td className="nowrap small" title={dateTime(e.at)}>{relative(e.at)}</td>
                          <td>
                            <span className={`badge ${e.severity === "critical" ? "red" : e.severity === "warning" ? "amber" : ""}`}>{titleCase(e.severity)}</span> {EVENT_HELP[e.kind] ?? e.kind}
                          </td>
                          <td style={{ maxWidth: 220 }}>
                            {e.actor_kind === "agent" ? (
                              // An agent's address is made up; its name says which account it was.
                              <span className="row wrap" style={{ gap: 6 }}>
                                <span className="truncate">{e.actor_name}</span> <span className="badge agent">AI agent</span>
                              </span>
                            ) : (
                              <span className="truncate" title={e.actor_name ?? undefined}>{e.actor_label || <span className="muted">Not signed in</span>}</span>
                            )}
                          </td>
                          <td className="hide-sm small muted num">{e.ip}</td>
                          <td className="hide-sm small muted truncate" style={{ maxWidth: 260 }}>{Object.entries(e.detail).map(([k, v]) => `${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`).join(", ")}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          </div>
        )}
      </Loaded>
    </>
  );
}

// ───────── jobs and settings ─────────
function Jobs() {
  const { can } = useSession();
  const q = useGet<any[]>("/api/v1/admin/jobs", { refetchInterval: 10_000 });
  const [busy, run] = useAction();
  return (
    <>
      <Head title="Background jobs" sub="Deliveries to chat and webhooks, search embeddings and imports run here. Failed jobs have used all their retries." />
      <div className="panel">
        <Loaded q={q}>
          {(list) =>
            list.length === 0 ? (
              <Empty title="Nothing queued, running or failed" />
            ) : (
              <div className="table-wrap" tabIndex={0} role="region" aria-label="Table: scroll sideways to see every column">
                <table className="table">
                  <thead>
                    <tr>
                      <th>Job</th>
                      <th>State</th>
                      <th>Attempts</th>
                      <th>Last error</th>
                      <th />
                    </tr>
                  </thead>
                  <tbody>
                    {list.map((j) => (
                      <tr key={j.id}>
                        <td>
                          <span className="num muted">#{j.id}</span> {titleCase(j.kind)} <span className="small muted">{relative(j.created_at)}</span>
                        </td>
                        <td>
                          <span className={`badge ${j.state === "failed" ? "red" : j.state === "done" ? "green" : j.state === "running" ? "blue" : ""}`}>{titleCase(j.state)}</span>
                        </td>
                        <td className="num">
                          {j.attempts} of {j.max_attempts}
                        </td>
                        <td className="small muted" style={{ maxWidth: 360, overflowWrap: "anywhere" }}>{j.last_error}</td>
                        <td>
                          {j.state === "failed" && can("admin.settings") && (
                            <button className="btn sm" disabled={busy} onClick={() => run(() => api("POST", `/api/v1/admin/jobs/${j.id}/retry`), "Job queued again")}>
                              <RefreshCw /> Retry
                            </button>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )
          }
        </Loaded>
      </div>
    </>
  );
}

export default function Admin() {
  const { can } = useSession();
  const visible = SECTIONS.filter((s) => "group" in s || can(s.perm));
  const first = visible.find((s) => "to" in s) as { to: string } | undefined;
  if (!can("admin.access") || !first) {
    return (
      <div className="page narrow">
        <Empty icon={<Lock />} title="Administration is limited to administrators">
          <p>Ask a system administrator if you need access.</p>
        </Empty>
      </div>
    );
  }
  const guard = (perm: string, el: React.ReactNode) => (can(perm) ? el : <Empty icon={<Lock />} title="Your role does not include this section" />);
  return (
    <div className="page">
      <div className="admin">
        <nav className="subnav" aria-label="Administration sections">
          {visible.map((s, i) =>
            "group" in s ? (
              visible[i + 1] && "to" in visible[i + 1] ? <span key={s.group} className="group">{s.group}</span> : null
            ) : (
              <NavLink key={s.to} to={`/admin/${s.to}`}>
                <s.icon /> {s.label}
              </NavLink>
            ),
          )}
        </nav>
        <div style={{ minWidth: 0 }}>
          <Routes>
            <Route index element={<Navigate to={`/admin/${first.to}`} replace />} />
            <Route path="overview" element={<Overview />} />
            <Route path="jobs" element={<Jobs />} />
            <Route path="settings" element={<SiteSettings />} />
            <Route path="notifications" element={<NotificationSettings />} />
            <Route path="alerts" element={<Alerts />} />
            <Route path="agents/:id" element={guard("admin.agents", <AgentRules />)} />
            <Route path="users" element={guard("admin.users", <Users />)} />
            <Route path="groups" element={guard("admin.users", <Groups />)} />
            <Route path="roles" element={<Roles />} />
            <Route path="sign-in" element={guard("admin.security", <SignIn />)} />
            <Route path="schemes" element={guard("admin.schemes", <Schemes />)} />
            <Route path="add-ons" element={guard("admin.schemes", <Addons />)} />
            <Route path="integrations" element={guard("admin.integrations", <Integrations />)} />
            <Route path="audit" element={guard("admin.audit", <Audit />)} />
            <Route path="security" element={guard("admin.security", <SecurityEvents />)} />
          </Routes>
        </div>
      </div>
    </div>
  );
}
