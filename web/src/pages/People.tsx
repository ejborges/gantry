// People and AI agents: the directory, a profile with the positions someone holds and has
// held, teams with their positions and leads, and the org chart.
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, Route, Routes, useNavigate, useParams } from "react-router-dom";
import { Bot, ChevronRight, Crown, Network, Pencil, Plus, Shuffle, Trash2, Upload, UserRound, Users } from "lucide-react";
import { api, upload, useGet, type Person } from "../api";
import { useSession } from "../session";
import { AgentBadge, Avatar, Confirm, Dialog, Empty, Field, Loaded, PersonChip, PersonSelect, PRESENCE, toast, useAction } from "../ui";
import { day, relative, timeZone, today } from "../lib/format";
import { useLiveRefresh } from "../realtime";
import { usePeople } from "../work";

/** "2 years, 3 months" from a number of days. */
export function span(days: number): string {
  if (days < 1) return "less than a day";
  if (days < 31) return `${days} ${days === 1 ? "day" : "days"}`;
  const months = Math.floor(days / 30.44);
  if (months < 12) return `${months} ${months === 1 ? "month" : "months"}`;
  const years = Math.floor(months / 12), rest = months % 12;
  return `${years} ${years === 1 ? "year" : "years"}${rest ? `, ${rest} ${rest === 1 ? "month" : "months"}` : ""}`;
}

export function PresenceLine({ person }: { person: Person }) {
  const state = person.presence && person.presence !== "available" ? (PRESENCE[person.presence] ?? person.presence) : null;
  if (!state && !person.status_message) return null;
  return (
    <span className="small muted">
      {state && <strong>{state}</strong>}
      {state && person.status_message ? ": " : ""}
      {person.status_message}
    </span>
  );
}

// ───────────────────────── own profile ─────────────────────────

const ZONES: string[] = (() => {
  try {
    return (Intl as any).supportedValuesOf("timeZone") as string[];
  } catch {
    return ["UTC", "America/New_York", "America/Chicago", "America/Denver", "America/Los_Angeles", "Europe/London", "Europe/Berlin", "Asia/Kolkata", "Asia/Tokyo", "Australia/Sydney"];
  }
})();

export function ProfileEditor() {
  const { me } = useSession();
  const rules = me.profile_rules;
  const u = me.user;
  const [busy, run] = useAction();
  const [form, setForm] = useState({ name: u.name, job_title: u.job_title ?? "", org_name: u.org_name ?? "", org_code: u.org_code ?? "", timezone: u.timezone ?? "auto" });
  const file = useRef<HTMLInputElement>(null);
  const may = (f: string) => rules.user_editable[f] !== false;
  const changed = Object.entries(form).filter(([k, v]) => v !== ((u as any)[k] ?? (k === "timezone" ? "auto" : "")));
  const device = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => setForm((f) => ({ ...f, [k]: e.target.value }));

  async function pick(f: File | undefined) {
    if (!f) return;
    if (!/^image\/(png|jpeg|webp|gif)$/.test(f.type)) return toast("Choose a PNG, JPEG, WebP or GIF picture.", "error");
    if (f.size > rules.avatar_max_kb * 1024) return toast(`That picture is ${Math.round(f.size / 1024)} KB. The limit is ${rules.avatar_max_kb} KB.`, "error");
    // Checked here for a quick answer; the server checks again.
    const dims = await new Promise<{ w: number; h: number } | null>((resolve) => {
      const img = new Image();
      img.onload = () => resolve({ w: img.naturalWidth, h: img.naturalHeight });
      img.onerror = () => resolve(null);
      img.src = URL.createObjectURL(f);
    });
    if (dims && Math.max(dims.w, dims.h) > rules.avatar_max_px) return toast(`That picture is ${dims.w} × ${dims.h} pixels. The limit is ${rules.avatar_max_px} on the longer side; ${rules.avatar_suggested_px} × ${rules.avatar_suggested_px} works best.`, "error");
    await run(() => upload("/api/v1/auth/me/avatar", f, "PUT"), "Picture updated");
  }

  return (
    <section className="panel">
      <header>
        <h2>You</h2>
      </header>
      <div className="body stack">
        <div className="row wrap" style={{ gap: 18, alignItems: "flex-start" }}>
          <Avatar person={u} size="xl" />
          <div className="stack tight">
            <div className="row wrap">
              <input ref={file} type="file" hidden accept="image/png,image/jpeg,image/webp,image/gif" onChange={(e) => (void pick(e.target.files?.[0]), (e.target.value = ""))} />
              <button className="btn" disabled={busy} onClick={() => file.current?.click()}>
                <Upload /> Upload a picture
              </button>
              {u.has_avatar ? (
                <button className="btn" disabled={busy} onClick={() => run(() => api("DELETE", "/api/v1/auth/me/avatar"), "Picture removed")}>
                  <Trash2 /> Remove
                </button>
              ) : (
                <button className="btn" disabled={busy} onClick={() => run(() => api("DELETE", "/api/v1/auth/me/avatar?shuffle=true"), "New pattern")} title="Draw a different pattern">
                  <Shuffle /> Another pattern
                </button>
              )}
            </div>
            <p className="small muted" style={{ maxWidth: 420 }}>
              A square picture of about {rules.avatar_suggested_px} × {rules.avatar_suggested_px} pixels works best. At most {rules.avatar_max_px} pixels on the longer side and {rules.avatar_max_kb} KB; PNG, JPEG, WebP or GIF.
              {!u.has_avatar && " Until you upload one, you have a pattern generated just for your account."}
            </p>
          </div>
        </div>
        <div className="form-grid">
          <Field label="Display name" hint={may("name") ? undefined : "Set by an administrator."}>
            <input className="input" value={form.name} maxLength={200} disabled={!may("name")} onChange={set("name")} />
          </Field>
          <Field label="Handle" hint="How others mention you. Ask an administrator to change it.">
            <input className="input" value={`@${u.handle ?? ""}`} readOnly />
          </Field>
          <Field label="Job title" hint={may("job_title") ? undefined : "Set by an administrator."}>
            <input className="input" value={form.job_title} maxLength={120} disabled={!may("job_title")} onChange={set("job_title")} />
          </Field>
          <Field label="Organization" hint={may("org_name") ? undefined : "Set by an administrator."}>
            <input className="input" value={form.org_name} maxLength={120} disabled={!may("org_name")} onChange={set("org_name")} />
          </Field>
          <Field label="Organization code" hint={may("org_code") ? "A short code such as a department or cost center." : "Set by an administrator."}>
            <input className="input" value={form.org_code} maxLength={40} disabled={!may("org_code")} onChange={set("org_code")} />
          </Field>
          <Field label="Time zone" hint={form.timezone === "auto" ? `Following this device: ${device.replace(/_/g, " ")}.` : "Dates and times are shown in this zone on every device."}>
            <select className="select" value={form.timezone} onChange={set("timezone")}>
              <option value="auto">Automatic (this device's time zone)</option>
              {ZONES.map((z) => (
                <option key={z} value={z}>
                  {z.replace(/_/g, " ")}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <div>
          <button className="btn primary" disabled={busy || !form.name.trim() || changed.length === 0} onClick={() => run(() => api("PATCH", "/api/v1/auth/me", Object.fromEntries(changed)), "Profile saved")}>
            Save profile
          </button>
        </div>
      </div>
    </section>
  );
}

const UNTIL = [
  { label: "Until I change it", hours: 0 },
  { label: "For an hour", hours: 1 },
  { label: "For four hours", hours: 4 },
  { label: "Until tomorrow", hours: -1 },
  { label: "For a week", hours: 168 },
];

export function StatusEditor() {
  const { me } = useSession();
  const u = me.user;
  const [busy, run] = useAction();
  const [presence, setPresence] = useState(u.presence ?? "available");
  const [message, setMessage] = useState(u.status_message ?? "");
  const [hours, setHours] = useState(0);
  function until(): string | null {
    if (hours === 0) return null;
    const d = new Date();
    if (hours < 0) {
      d.setDate(d.getDate() + 1);
      d.setHours(0, 0, 0, 0);
    } else d.setTime(d.getTime() + hours * 3600_000);
    return d.toISOString();
  }
  const clear = presence === "available" && !message.trim();
  return (
    <section className="panel">
      <header>
        <h2>Status</h2>
        {u.status_until && <span className="small muted">Clears {relative(u.status_until)}</span>}
      </header>
      <div className="body stack">
        <div className="form-grid">
          <Field label="I am">
            <select className="select" value={presence} onChange={(e) => setPresence(e.target.value)}>
              {Object.entries(PRESENCE).map(([k, v]) => (
                <option key={k} value={k}>
                  {v}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Message" hint="A few words others see next to your name.">
            <input className="input" value={message} maxLength={140} onChange={(e) => setMessage(e.target.value)} placeholder="In the lab until 3, back Monday, …" />
          </Field>
          <Field label="Keep this">
            <select className="select" value={hours} disabled={clear} onChange={(e) => setHours(Number(e.target.value))}>
              {UNTIL.map((o) => (
                <option key={o.hours} value={o.hours}>
                  {o.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <div className="row">
          <button className="btn primary" disabled={busy} onClick={() => run(() => api("PATCH", "/api/v1/auth/me", { presence, status_message: message.trim(), status_until: clear ? null : until() }), clear ? "Status cleared" : "Status set")}>
            {clear ? "Clear status" : "Set status"}
          </button>
        </div>
      </div>
    </section>
  );
}

interface Choices {
  categories: { key: string; label: string; description: string; in_app: boolean; email: boolean; locked: boolean }[];
  email_available: boolean;
  email_address: string;
}

export function NotificationChoices() {
  const q = useGet<Choices>("/api/v1/notifications/preferences");
  const [busy, run] = useAction();
  return (
    <section className="panel" id="notifications">
      <header>
        <h2>Notifications</h2>
      </header>
      <div className="body stack">
        <Loaded q={q} what="Loading your notification choices">
          {(p) => (
            <>
              <p className="small muted">
                Choose what you are told about, here in the application and by email{p.email_available ? ` to ${p.email_address}` : ""}.
                {!p.email_available && " Email is not available for this account or has not been set up by the administrators."}
              </p>
              <div className="table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <th scope="col">Tell me about</th>
                      <th scope="col" style={{ width: 110 }}>In the application</th>
                      <th scope="col" style={{ width: 90 }}>By email</th>
                    </tr>
                  </thead>
                  <tbody>
                    {p.categories.map((c) => (
                      <tr key={c.key}>
                        <th scope="row" style={{ fontWeight: 400 }}>
                          <strong>{c.label}</strong>
                          <div className="small muted">{c.description}</div>
                          {c.locked && <div className="small muted">Set by an administrator for everyone.</div>}
                        </th>
                        {(["in_app", "email"] as const).map((ch) => (
                          <td key={ch}>
                            <input
                              type="checkbox"
                              aria-label={`${c.label}: ${ch === "in_app" ? "in the application" : "by email"}`}
                              checked={c[ch]}
                              disabled={busy || c.locked || (ch === "email" && !p.email_available)}
                              onChange={(e) => run(() => api("PUT", "/api/v1/notifications/preferences", { [c.key]: { in_app: ch === "in_app" ? e.target.checked : c.in_app, email: ch === "email" ? e.target.checked : c.email } }))}
                            />
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </Loaded>
      </div>
    </section>
  );
}

// ───────────────────────── directory and profiles ─────────────────────────

function Directory() {
  const [q, setQ] = useState("");
  const [kind, setKind] = useState("");
  const people = useGet<Person[]>(`/api/v1/users?limit=500${kind ? `&kind=${kind}` : ""}${q.trim() ? `&q=${encodeURIComponent(q.trim())}` : ""}`);
  useLiveRefresh(["directory"]);
  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>People</h1>
          <p className="sub">Everyone with an account here, and the AI agents that work alongside them.</p>
        </div>
        <div className="row">
          <Link className="btn" to="/teams">
            <Users /> Teams
          </Link>
          <Link className="btn" to="/teams/chart">
            <Network /> Org chart
          </Link>
        </div>
      </div>
      <div className="filters">
        <input className="input" style={{ maxWidth: 320 }} type="search" placeholder="Name, handle or email" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search people" />
        <div className="mode-switch" role="group" aria-label="Kind of account">
          {[["", "Everyone"], ["human", "People"], ["agent", "AI agents"]].map(([v, label]) => (
            <button key={v} type="button" aria-pressed={kind === v} onClick={() => setKind(v)}>
              {label}
            </button>
          ))}
        </div>
      </div>
      <Loaded q={people} what="Loading the directory">
        {(list) =>
          list.length === 0 ? (
            <Empty icon={<UserRound />} title="Nobody matches" />
          ) : (
            <div className="card-grid people-grid">
              {list.map((p) => (
                <Link key={p.id} to={`/people/${p.id}`} className={`card person-card ${p.kind === "agent" ? "agent" : ""}`}>
                  <Avatar person={p} size="lg" presence />
                  <span className="stack tight grow" style={{ minWidth: 0 }}>
                    <span className="row wrap" style={{ gap: 6 }}>
                      <strong className="truncate">{p.name}</strong>
                      <AgentBadge person={p} />
                    </span>
                    <span className="small muted truncate">{[p.job_title, p.org_name].filter(Boolean).join(" · ") || `@${p.handle}`}</span>
                    <PresenceLine person={p} />
                  </span>
                </Link>
              ))}
            </div>
          )
        }
      </Loaded>
    </div>
  );
}

function PersonPage() {
  const { id } = useParams();
  const q = useGet<any>(`/api/v1/users/${id}`);
  const { me, can } = useSession();
  return (
    <div className="page narrow">
      <div className="crumbs">
        <Link to="/people">People</Link>
        <ChevronRight />
      </div>
      <Loaded q={q} what="Loading the profile">
        {(p) => {
          const agent = p.kind === "agent";
          const current = p.positions.filter((x: any) => !x.ended_on), past = p.positions.filter((x: any) => x.ended_on);
          let local = "";
          try {
            local = new Date().toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", timeZone: p.timezone && p.timezone !== "auto" ? p.timezone : undefined });
          } catch {
            /* an unknown zone: leave the local time out */
          }
          return (
            <div className="stack loose">
              <header className="row wrap" style={{ gap: 18, alignItems: "flex-start" }}>
                <Avatar person={p} size="xl" presence />
                <div className="stack tight grow">
                  <h1 className="row wrap" style={{ gap: 10 }}>
                    {p.name} <AgentBadge person={p} />
                  </h1>
                  <p className="muted">
                    @{p.handle}
                    {p.job_title ? ` · ${p.job_title}` : ""}
                    {p.org_name ? ` · ${p.org_name}` : ""}
                    {p.org_code ? ` (${p.org_code})` : ""}
                  </p>
                  <PresenceLine person={p} />
                  {!agent && p.email && (
                    <p className="small">
                      <a href={`mailto:${p.email}`}>{p.email}</a>
                      {p.timezone && p.timezone !== "auto" && local ? <span className="muted"> · {local} local time ({String(p.timezone).replace(/_/g, " ")})</span> : null}
                    </p>
                  )}
                  {!p.active && <span className="badge red">This account is deactivated</span>}
                </div>
                {id === me.user.id && (
                  <Link className="btn" to="/profile">
                    <Pencil /> Edit my profile
                  </Link>
                )}
                {agent && can("admin.agents") && (
                  <Link className="btn" to={`/admin/agents/${p.id}`}>
                    <Bot /> Rules for this agent
                  </Link>
                )}
              </header>
              {agent && (
                <section className="notice">
                  <Bot />
                  <span>
                    This is an AI agent account, not a person. It acts through the API within the rules an administrator has written for it.
                    {p.description ? ` Purpose: ${p.description}` : ""}
                    {p.owner ? ` Responsible: ${p.owner.name}.` : ""}
                  </span>
                </section>
              )}
              <section>
                <div className="section-title">
                  <h2>Positions</h2>
                </div>
                {current.length === 0 ? (
                  <p className="muted">Not in a team at the moment.</p>
                ) : (
                  <div className="panel">
                    {current.map((x: any) => (
                      <Link key={x.assignment_id} to={`/teams/${x.team_id}`} className="list-row">
                        {x.is_lead ? <Crown width={15} height={15} aria-label="Team lead" /> : <Users width={15} height={15} aria-hidden="true" />}
                        <span className="grow">
                          <strong>{x.title}</strong> in {x.team}
                        </span>
                        <span className="small muted nowrap">
                          since {day(x.started_on)} · {span(x.days)}
                        </span>
                      </Link>
                    ))}
                  </div>
                )}
              </section>
              {past.length > 0 && (
                <section>
                  <div className="section-title">
                    <h2>Earlier positions</h2>
                  </div>
                  <div className="panel">
                    {past.map((x: any) => (
                      <Link key={x.assignment_id} to={`/teams/${x.team_id}`} className="list-row">
                        <span className="grow">
                          {x.title} in {x.team}
                          {x.team_archived ? " (archived)" : ""}
                        </span>
                        <span className="small muted nowrap">
                          {day(x.started_on)} to {day(x.ended_on)} · {span(x.days)}
                        </span>
                      </Link>
                    ))}
                  </div>
                </section>
              )}
              <p className="small muted">Account created {day(String(p.created_at).slice(0, 10))}. Your times are shown in {timeZone().replace(/_/g, " ")}.</p>
            </div>
          );
        }}
      </Loaded>
    </div>
  );
}

// ───────────────────────── teams ─────────────────────────

interface TeamRow {
  id: string;
  name: string;
  description: string;
  parent_id: string | null;
  archived: boolean;
  leads: Person[];
  members: number;
  positions: number;
  mine: boolean;
}

function NewTeam({ parent, teams, onClose }: { parent?: string; teams: TeamRow[]; onClose: () => void }) {
  const [form, setForm] = useState({ name: "", description: "", parent_id: parent ?? "" });
  const [busy, run] = useAction();
  const navigate = useNavigate();
  return (
    <Dialog
      title="New team"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" disabled={busy || !form.name.trim()} onClick={() => run(() => api("POST", "/api/v1/teams", { name: form.name, description: form.description, parent_id: form.parent_id || null }), (t: any) => (onClose(), navigate(`/teams/${t.id}`)))}>
            Create team
          </button>
        </>
      }
    >
      <div className="stack">
        <Field label="Name" required>
          <input className="input" value={form.name} maxLength={200} onChange={(e) => setForm({ ...form, name: e.target.value })} autoFocus />
        </Field>
        <Field label="What the team does">
          <textarea className="textarea" rows={3} value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />
        </Field>
        <Field label="Part of" hint="A team can belong to a larger team.">
          <select className="select" value={form.parent_id} onChange={(e) => setForm({ ...form, parent_id: e.target.value })}>
            <option value="">No larger team</option>
            {teams.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        </Field>
        <p className="small muted">You become the team's first lead. Leads add positions and decide who fills them.</p>
      </div>
    </Dialog>
  );
}

function TeamTree({ teams, parent, depth }: { teams: TeamRow[]; parent: string | null; depth: number }) {
  const here = teams.filter((t) => (t.parent_id ?? null) === parent || (parent === null && t.parent_id && !teams.some((x) => x.id === t.parent_id)));
  if (here.length === 0) return null;
  return (
    <>
      {here.map((t) => (
        <div key={t.id}>
          <Link to={`/teams/${t.id}`} className="list-row" style={{ paddingLeft: 14 + depth * 22 }}>
            <Users width={15} height={15} aria-hidden="true" />
            <span className="grow stack tight" style={{ minWidth: 0 }}>
              <span className="row wrap" style={{ gap: 6 }}>
                <strong>{t.name}</strong>
                {t.mine && <span className="badge blue">Your team</span>}
                {t.archived && <span className="badge outline">Archived</span>}
              </span>
              {t.description && <span className="small muted truncate">{t.description}</span>}
            </span>
            <span className="row hide-sm" style={{ gap: 3 }} aria-label={`Led by ${t.leads.map((l) => l.name).join(", ") || "nobody"}`}>
              {t.leads.slice(0, 4).map((l) => (
                <Avatar key={l.id} person={l} size="sm" />
              ))}
            </span>
            <span className="small muted nowrap">
              {t.members} {t.members === 1 ? "member" : "members"}
            </span>
          </Link>
          <TeamTree teams={teams} parent={t.id} depth={depth + 1} />
        </div>
      ))}
    </>
  );
}

function Teams() {
  const [archived, setArchived] = useState(false);
  const q = useGet<{ teams: TeamRow[]; can_create: boolean; can_manage: boolean }>(`/api/v1/teams${archived ? "?archived=true" : ""}`);
  const [creating, setCreating] = useState(false);
  useLiveRefresh(["teams"]);
  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Teams</h1>
          <p className="sub">Who works together, in which position, and who leads.</p>
        </div>
        <div className="row wrap">
          <Link className="btn" to="/people">
            <UserRound /> People
          </Link>
          <Link className="btn" to="/teams/chart">
            <Network /> Org chart
          </Link>
          {q.data?.can_create && (
            <button className="btn primary" onClick={() => setCreating(true)}>
              <Plus /> New team
            </button>
          )}
        </div>
      </div>
      <Loaded q={q} what="Loading the teams">
        {(d) => {
          const mine = d.teams.filter((t) => t.mine);
          return (
            <div className="stack loose">
              {mine.length > 0 && (
                <section>
                  <div className="section-title">
                    <h2>Your teams</h2>
                  </div>
                  <div className="card-grid">
                    {mine.map((t) => (
                      <Link key={t.id} to={`/teams/${t.id}`} className="card">
                        <strong>{t.name}</strong>
                        <span className="small muted">{t.description || "No description."}</span>
                        <span className="row" style={{ gap: 3 }}>
                          {t.leads.map((l) => (
                            <Avatar key={l.id} person={l} size="sm" />
                          ))}
                          <span className="small muted" style={{ marginLeft: 6 }}>
                            {t.members} {t.members === 1 ? "member" : "members"}
                          </span>
                        </span>
                      </Link>
                    ))}
                  </div>
                </section>
              )}
              <section>
                <div className="section-title">
                  <h2>All teams</h2>
                  <label className="check small">
                    <input type="checkbox" checked={archived} onChange={(e) => setArchived(e.target.checked)} /> Show archived
                  </label>
                </div>
                {d.teams.length === 0 ? (
                  <Empty icon={<Users />} title="No teams yet">
                    <p>{d.can_create ? "Create the first team, add its positions, and put people in them." : "Nobody has created a team yet."}</p>
                  </Empty>
                ) : (
                  <div className="panel">
                    <TeamTree teams={d.teams} parent={null} depth={0} />
                  </div>
                )}
              </section>
              {creating && <NewTeam teams={d.teams} onClose={() => setCreating(false)} />}
            </div>
          );
        }}
      </Loaded>
    </div>
  );
}

function TeamPage() {
  const { id } = useParams();
  const q = useGet<any>(`/api/v1/teams/${id}`);
  const all = useGet<{ teams: TeamRow[] }>("/api/v1/teams");
  const people = usePeople();
  const [busy, run] = useAction();
  const [edit, setEdit] = useState<any | null>(null);
  const [position, setPosition] = useState<any | null>(null);
  const [assign, setAssign] = useState<{ position: any; user: string | null; started_on: string } | null>(null);
  const [end, setEnd] = useState<{ a: any; title: string; started_on: string; ended_on: string } | null>(null);
  const [remove, setRemove] = useState<any | null>(null);
  const [subteam, setSubteam] = useState(false);
  const [past, setPast] = useState(false);
  useLiveRefresh([id ? `team:${id}` : null]);
  const base = `/api/v1/teams/${id}`;
  return (
    <div className="page">
      <div className="crumbs">
        <Link to="/teams">Teams</Link>
        {q.data?.parent && (
          <>
            <ChevronRight />
            <Link to={`/teams/${q.data.parent.id}`}>{q.data.parent.name}</Link>
          </>
        )}
      </div>
      <Loaded q={q} what="Loading the team">
        {(t) => {
          const open = t.positions.filter((p: any) => !p.closed), closed = t.positions.filter((p: any) => p.closed);
          const hasPast = t.positions.some((p: any) => p.past.length > 0) || closed.length > 0;
          return (
            <div className="stack loose">
              <div className="page-head" style={{ marginBottom: 0 }}>
                <div>
                  <h1>
                    {t.name} {t.archived && <span className="badge outline">Archived</span>}
                  </h1>
                  <p className="sub">{t.description || "No description."}</p>
                </div>
                {t.can_edit && (
                  <div className="row wrap">
                    <button className="btn" onClick={() => setEdit({ name: t.name, description: t.description, parent_id: t.parent_id ?? "" })}>
                      <Pencil /> Edit team
                    </button>
                    <button className="btn" onClick={() => setSubteam(true)}>
                      <Plus /> Team within this one
                    </button>
                    <button className="btn primary" onClick={() => setPosition({ title: "", description: "", is_lead: false })}>
                      <Plus /> Add position
                    </button>
                  </div>
                )}
              </div>
              {t.children.length > 0 && (
                <p className="row wrap small">
                  <span className="muted">Teams within:</span>
                  {t.children.map((c: any) => (
                    <Link key={c.id} to={`/teams/${c.id}`} className="badge outline">
                      {c.name}
                    </Link>
                  ))}
                </p>
              )}
              <section>
                <div className="section-title">
                  <h2>Positions</h2>
                  <span className="small muted">A team is its positions. Each is filled by one or more people or AI agents.</span>
                </div>
                {open.length === 0 ? (
                  <Empty icon={<Users />} title="No positions yet">
                    {t.can_edit && <p>Add the positions this team needs, then put people in them.</p>}
                  </Empty>
                ) : (
                  <div className="positions">
                    {open.map((p: any) => (
                      <div key={p.id} className={`panel position ${p.is_lead ? "lead" : ""}`}>
                        <header>
                          <h3 className="row" style={{ gap: 6 }}>
                            {p.is_lead && <Crown width={15} height={15} aria-hidden="true" />}
                            {p.title}
                            {p.is_lead && <span className="badge amber">Lead</span>}
                          </h3>
                          {t.can_edit && (
                            <span className="row">
                              <button className="btn ghost sm" onClick={() => setAssign({ position: p, user: null, started_on: today() })}>
                                <Plus /> Add someone
                              </button>
                              <button className="btn ghost icon sm" aria-label={`Edit the position ${p.title}`} onClick={() => setPosition({ ...p })}>
                                <Pencil />
                              </button>
                            </span>
                          )}
                        </header>
                        {p.description && <p className="small muted" style={{ padding: "0 14px 8px" }}>{p.description}</p>}
                        {p.holders.length === 0 ? (
                          <p className="muted small" style={{ padding: "0 14px 12px" }}>Open: nobody is in this position.</p>
                        ) : (
                          p.holders.map((h: any) => (
                            <div key={h.assignment_id} className="list-row">
                              {/* The name gets the whole line; the date sits under it, so neither is cut short. */}
                              <span className="grow stack tight" style={{ minWidth: 0 }}>
                                <Link to={`/people/${h.user.id}`} style={{ minWidth: 0 }}>
                                  <PersonChip person={h.user} presence />
                                </Link>
                                <span className="small muted">since {day(h.started_on)}</span>
                              </span>
                              {t.can_edit && (
                                <button className="btn ghost sm" onClick={() => setEnd({ a: h, title: p.title, started_on: h.started_on, ended_on: today() })}>
                                  Change
                                </button>
                              )}
                            </div>
                          ))
                        )}
                        {past && p.past.length > 0 && (
                          <div className="past">
                            {p.past.map((h: any) => (
                              <div key={h.assignment_id} className="list-row small muted">
                                <span className="grow">
                                  {h.user.name}
                                  {h.user.kind === "agent" ? " (AI agent)" : ""}
                                </span>
                                <span className="nowrap">
                                  {day(h.started_on)} to {day(h.ended_on)}
                                </span>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
                {hasPast && (
                  <label className="check small" style={{ marginTop: 10 }}>
                    <input type="checkbox" checked={past} onChange={(e) => setPast(e.target.checked)} /> Show who held these positions before{closed.length ? `, and ${closed.length} closed ${closed.length === 1 ? "position" : "positions"}` : ""}
                  </label>
                )}
                {past && closed.length > 0 && (
                  <div className="panel" style={{ marginTop: 10 }}>
                    {closed.map((p: any) => (
                      <div key={p.id} className="list-row">
                        <span className="grow muted">
                          {p.title} <span className="badge outline">Closed</span>
                          <span className="small"> · held by {p.past.map((h: any) => h.user.name).join(", ") || "nobody"}</span>
                        </span>
                        {t.can_edit && (
                          <button className="btn sm" disabled={busy} onClick={() => run(() => api("PATCH", `${base}/positions/${p.id}`, { closed: false }), "Position reopened")}>
                            Reopen
                          </button>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </section>

              {edit && (
                <Dialog
                  title="Edit team"
                  onClose={() => setEdit(null)}
                  footer={
                    <>
                      <button className="btn danger" style={{ marginRight: "auto" }} disabled={busy} onClick={() => run(() => api("PATCH", base, { archived: !t.archived }), t.archived ? "Team restored" : "Team archived").then(() => setEdit(null))}>
                        {t.archived ? "Restore team" : "Archive team"}
                      </button>
                      <button className="btn" onClick={() => setEdit(null)}>
                        Cancel
                      </button>
                      <button className="btn primary" disabled={busy || !edit.name.trim()} onClick={() => run(() => api("PATCH", base, { name: edit.name, description: edit.description, parent_id: edit.parent_id || null }), "Team saved").then((r) => r !== undefined && setEdit(null))}>
                        Save
                      </button>
                    </>
                  }
                >
                  <div className="stack">
                    <Field label="Name" required>
                      <input className="input" value={edit.name} maxLength={200} onChange={(e) => setEdit({ ...edit, name: e.target.value })} />
                    </Field>
                    <Field label="What the team does">
                      <textarea className="textarea" rows={3} value={edit.description} onChange={(e) => setEdit({ ...edit, description: e.target.value })} />
                    </Field>
                    <Field label="Part of">
                      <select className="select" value={edit.parent_id} onChange={(e) => setEdit({ ...edit, parent_id: e.target.value })}>
                        <option value="">No larger team</option>
                        {(all.data?.teams ?? []).filter((x) => x.id !== t.id).map((x) => (
                          <option key={x.id} value={x.id}>
                            {x.name}
                          </option>
                        ))}
                      </select>
                    </Field>
                  </div>
                </Dialog>
              )}
              {position && (
                <Dialog
                  title={position.id ? "Edit position" : "Add a position"}
                  onClose={() => setPosition(null)}
                  footer={
                    <>
                      {position.id && (
                        <button className="btn danger" style={{ marginRight: "auto" }} onClick={() => (setRemove(position), setPosition(null))}>
                          Remove position
                        </button>
                      )}
                      <button className="btn" onClick={() => setPosition(null)}>
                        Cancel
                      </button>
                      <button
                        className="btn primary"
                        disabled={busy || !position.title.trim()}
                        onClick={() => run(() => api(position.id ? "PATCH" : "POST", position.id ? `${base}/positions/${position.id}` : `${base}/positions`, { title: position.title, description: position.description ?? "", is_lead: !!position.is_lead }), "Position saved").then((r) => r !== undefined && setPosition(null))}
                      >
                        Save
                      </button>
                    </>
                  }
                >
                  <div className="stack">
                    <Field label="Title" required hint="For example: Test engineer, Scrum master, On-call responder.">
                      <input className="input" value={position.title} maxLength={200} onChange={(e) => setPosition({ ...position, title: e.target.value })} autoFocus />
                    </Field>
                    <Field label="What the position is responsible for">
                      <textarea className="textarea" rows={3} value={position.description ?? ""} onChange={(e) => setPosition({ ...position, description: e.target.value })} />
                    </Field>
                    <label className="check">
                      <input type="checkbox" checked={!!position.is_lead} onChange={(e) => setPosition({ ...position, is_lead: e.target.checked })} /> This position leads the team (whoever holds it can edit the team and the teams within it)
                    </label>
                  </div>
                </Dialog>
              )}
              {assign && (
                <Dialog
                  title={`Add someone as ${assign.position.title}`}
                  onClose={() => setAssign(null)}
                  footer={
                    <>
                      <button className="btn" onClick={() => setAssign(null)}>
                        Cancel
                      </button>
                      <button className="btn primary" disabled={busy || !assign.user} onClick={() => run(() => api("POST", `${base}/positions/${assign.position.id}/assignments`, { user_id: assign.user, started_on: assign.started_on }), "Added to the team").then((r) => r !== undefined && setAssign(null))}>
                        Add
                      </button>
                    </>
                  }
                >
                  <div className="stack">
                    <Field label="Who" required>
                      <PersonSelect people={people} value={assign.user} onChange={(user) => setAssign({ ...assign, user })} none="Choose a person or an AI agent" />
                    </Field>
                    <Field label="In this position since" hint="Today, or the date they actually started. Their time in the position is counted from here.">
                      <input className="input" type="date" value={assign.started_on} onChange={(e) => setAssign({ ...assign, started_on: e.target.value })} />
                    </Field>
                  </div>
                </Dialog>
              )}
              {end && (
                <Dialog
                  title={`${end.a.user.name} as ${end.title}`}
                  onClose={() => setEnd(null)}
                  footer={
                    <>
                      <button className="btn" onClick={() => setEnd(null)}>
                        Cancel
                      </button>
                      <button className="btn" disabled={busy || end.started_on === end.a.started_on} onClick={() => run(() => api("PATCH", `${base}/assignments/${end.a.assignment_id}`, { started_on: end.started_on }), "Start date changed").then((r) => r !== undefined && setEnd(null))}>
                        Save start date
                      </button>
                      <button className="btn primary" disabled={busy} onClick={() => run(() => api("PATCH", `${base}/assignments/${end.a.assignment_id}`, { ended_on: end.ended_on }), `${end.a.user.name} left the position`).then((r) => r !== undefined && setEnd(null))}>
                        End on this date
                      </button>
                    </>
                  }
                >
                  <div className="stack">
                    <div className="form-grid">
                      <Field label="Started on" hint="Correct this if they started earlier or later.">
                        <input className="input" type="date" value={end.started_on} onChange={(e) => setEnd({ ...end, started_on: e.target.value })} />
                      </Field>
                      <Field label="Leaves the position on">
                        <input className="input" type="date" value={end.ended_on} min={end.started_on} onChange={(e) => setEnd({ ...end, ended_on: e.target.value })} />
                      </Field>
                    </div>
                    <p className="small muted">Ending keeps the record: the position and how long they held it stay on their profile.</p>
                    <button className="btn danger sm" style={{ alignSelf: "flex-start" }} disabled={busy} onClick={() => run(() => api("DELETE", `${base}/assignments/${end.a.assignment_id}`), "Entry deleted").then(() => setEnd(null))}>
                      <Trash2 /> This was entered by mistake: delete it
                    </button>
                  </div>
                </Dialog>
              )}
              {remove && (
                <Confirm
                  title={`Remove the position ${remove.title}?`}
                  action="Remove position"
                  danger
                  body="If anyone has held it, the position is closed instead, so their history is kept."
                  onClose={() => setRemove(null)}
                  onConfirm={async () => {
                    await run(() => api("DELETE", `${base}/positions/${remove.id}`), "Position removed");
                  }}
                />
              )}
              {subteam && <NewTeam parent={t.id} teams={all.data?.teams ?? []} onClose={() => setSubteam(false)} />}
            </div>
          );
        }}
      </Loaded>
    </div>
  );
}

interface ChartTeam {
  id: string;
  name: string;
  description: string;
  parent_id: string | null;
  positions: { id: string; title: string; is_lead: boolean; holders: Person[] }[];
}

function ChartNode({ team, all, mine }: { team: ChartTeam; all: ChartTeam[]; mine: string }) {
  const kids = all.filter((t) => t.parent_id === team.id);
  const has = team.positions.some((p) => p.holders.some((h) => h.id === mine));
  return (
    <li>
      <div className={`org-team ${has ? "mine" : ""}`}>
        <Link to={`/teams/${team.id}`} className="org-team-name">
          {team.name}
        </Link>
        {team.positions.length === 0 && <span className="small muted">No positions</span>}
        {team.positions.map((p) => (
          <div key={p.id} className="org-position">
            <span className="small muted row" style={{ gap: 4 }}>
              {p.is_lead && <Crown width={12} height={12} aria-label="Lead" />}
              {p.title}
            </span>
            {p.holders.length === 0 ? (
              <span className="small muted">Open</span>
            ) : (
              p.holders.map((h) => (
                <Link key={h.id} to={`/people/${h.id}`} className="org-person">
                  <PersonChip person={h} presence />
                </Link>
              ))
            )}
          </div>
        ))}
      </div>
      {kids.length > 0 && (
        <ul>
          {kids.map((k) => (
            <ChartNode key={k.id} team={k} all={all} mine={mine} />
          ))}
        </ul>
      )}
    </li>
  );
}

function OrgChart() {
  const q = useGet<ChartTeam[]>("/api/v1/org-chart");
  const { me } = useSession();
  useLiveRefresh(["teams"]);
  const scroller = useRef<HTMLDivElement>(null);
  useEffect(() => {
    scroller.current?.querySelector(".org-team.mine")?.scrollIntoView({ block: "nearest", inline: "center" });
  }, [q.data]);
  const roots = useMemo(() => (q.data ?? []).filter((t) => !t.parent_id || !(q.data ?? []).some((x) => x.id === t.parent_id)), [q.data]);
  return (
    <div className="page wide">
      <div className="page-head">
        <div>
          <h1>Org chart</h1>
          <p className="sub">Teams, the positions in them, and who fills each. Your own teams are outlined.</p>
        </div>
        <div className="row">
          <Link className="btn" to="/teams">
            <Users /> Teams
          </Link>
          <Link className="btn" to="/people">
            <UserRound /> People
          </Link>
        </div>
      </div>
      <Loaded q={q} what="Drawing the org chart">
        {(teams) =>
          teams.length === 0 ? (
            <Empty icon={<Network />} title="No teams yet">
              <p>The chart is drawn from teams and their positions.</p>
            </Empty>
          ) : (
            <div className="org-scroll" ref={scroller} tabIndex={0} role="region" aria-label="Org chart">
              <ul className="org">
                {roots.map((t) => (
                  <ChartNode key={t.id} team={t} all={teams} mine={me.user.id} />
                ))}
              </ul>
            </div>
          )
        }
      </Loaded>
    </div>
  );
}

export function PeopleRoutes() {
  return (
    <Routes>
      <Route index element={<Directory />} />
      <Route path=":id" element={<PersonPage />} />
    </Routes>
  );
}

export function TeamRoutes() {
  return (
    <Routes>
      <Route index element={<Teams />} />
      <Route path="chart" element={<OrgChart />} />
      <Route path=":id" element={<TeamPage />} />
    </Routes>
  );
}
