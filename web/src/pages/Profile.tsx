import { useState } from "react";
import { Link } from "react-router-dom";
import { Bot, KeyRound, Trash2 } from "lucide-react";
import { NotificationChoices, ProfileEditor, StatusEditor } from "./People";
import { api, useGet } from "../api";
import { useSession, type Theme } from "../session";
import { CopyButton, Dialog, Empty, Field, Loaded, useAction } from "../ui";
import { dateTime, relative } from "../lib/format";

export function TokenDialog({ owner, onClose, allowAdmin }: { owner?: { id: string; name: string }; onClose: () => void; allowAdmin: boolean }) {
  const [busy, run] = useAction();
  const [name, setName] = useState("");
  const [scope, setScope] = useState("read,write");
  const [days, setDays] = useState("90");
  const [created, setCreated] = useState<string | null>(null);
  const path = owner ? `/api/v1/admin/users/${owner.id}/tokens` : "/api/v1/tokens";
  return (
    <Dialog
      title={created ? "Copy the token now" : owner ? `New API token for ${owner.name}` : "New API token"}
      onClose={onClose}
      footer={
        created ? (
          <button className="btn primary" onClick={onClose}>
            I have stored it
          </button>
        ) : (
          <>
            <button className="btn" onClick={onClose}>
              Cancel
            </button>
            <button className="btn primary" disabled={busy || !name.trim()} onClick={() => run(() => api("POST", path, { name, scopes: scope.split(","), expires_in_days: days ? Number(days) : undefined }), (t: any) => setCreated(t.token))}>
              Create token
            </button>
          </>
        )
      }
    >
      {created ? (
        <div className="stack">
          <p>This is the only time the token is shown. Store it in a password manager or your agent's secret store.</p>
          <div className="secret" data-testid="new-token">{created}</div>
          <CopyButton text={created} label="Copy token" />
        </div>
      ) : (
        <div className="stack">
          <Field label="Name" hint="Where it will be used, so you can recognize it later." required>
            <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Planning agent on build server" />
          </Field>
          <div className="form-grid">
            <Field label="Allowed to">
              <select className="select" value={scope} onChange={(e) => setScope(e.target.value)}>
                <option value="read">Read only</option>
                <option value="read,write">Read and change content</option>
                {allowAdmin && <option value="read,write,admin">Read, change and administer</option>}
              </select>
            </Field>
            <Field label="Expires after">
              <select className="select" value={days} onChange={(e) => setDays(e.target.value)}>
                <option value="30">30 days</option>
                <option value="90">90 days</option>
                <option value="365">1 year</option>
                <option value="">Never</option>
              </select>
            </Field>
          </div>
          <p className="small muted">A token can never do more than its owner's roles allow; the choice above narrows it further.</p>
        </div>
      )}
    </Dialog>
  );
}

export function TokenTable({ tokens, onRevoke }: { tokens: any[]; onRevoke: (id: string) => void }) {
  if (tokens.length === 0) return <Empty icon={<KeyRound />} title="No API tokens"><p>Tokens let scripts and AI agents use the API and the MCP endpoint.</p></Empty>;
  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Allowed to</th>
            <th className="hide-sm">Last used</th>
            <th>Expires</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {tokens.map((t) => {
            const dead = t.revoked_at || (t.expires_at && new Date(t.expires_at) < new Date());
            return (
              <tr key={t.id} style={{ opacity: dead ? 0.55 : 1 }}>
                <td>
                  {t.name} <span className="muted small">gty_{t.prefix}…</span>
                </td>
                <td>{t.scopes.includes("admin") ? "Administer" : t.scopes.includes("write") ? "Read and change" : "Read only"}</td>
                <td className="hide-sm muted">{t.last_used_at ? relative(t.last_used_at) : "Never"}</td>
                <td>{t.revoked_at ? <span className="badge red">Revoked</span> : t.expires_at ? <span title={dateTime(t.expires_at)}>{dead ? <span className="badge red">Expired</span> : relative(t.expires_at)}</span> : "Never"}</td>
                <td style={{ textAlign: "right" }}>
                  {!t.revoked_at && (
                    <button className="btn ghost icon sm" aria-label={`Revoke ${t.name}`} title="Revoke" onClick={() => onRevoke(t.id)}>
                      <Trash2 />
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export default function Profile() {
  const { me, theme, setTheme, can } = useSession();
  const tokens = useGet<any[]>("/api/v1/tokens");
  const [busy, run] = useAction();
  const [pw, setPw] = useState({ current: "", next: "" });
  const [creating, setCreating] = useState(false);
  const mcp = `{
  "mcpServers": {
    "gantry": {
      "type": "http",
      "url": "${location.origin}/mcp",
      "headers": { "Authorization": "Bearer <your token>" }
    }
  }
}`;
  return (
    <div className="page narrow">
      <div className="page-head">
        <div>
          <h1>Profile</h1>
          <p className="sub">
            {me.user.email} · <Link to={`/people/${me.user.id}`}>See how others see you</Link>
          </p>
        </div>
      </div>
      <div className="stack loose">
        <ProfileEditor />
        <StatusEditor />
        <section className="panel">
          <header>
            <h2>Appearance</h2>
          </header>
          <div className="body">
            <Field label="Theme" hint="“Match device” follows your system's light or dark setting.">
              <select className="select" style={{ maxWidth: 260 }} value={theme} onChange={(e) => setTheme(e.target.value as Theme)}>
                <option value="system">Match device</option>
                <option value="light">Light</option>
                <option value="dark">Dark</option>
              </select>
            </Field>
          </div>
        </section>
        <NotificationChoices />

        {me.features.local_login && (
          <section className="panel">
            <header>
              <h2>Password</h2>
            </header>
            <form
              className="body stack"
              onSubmit={(e) => {
                e.preventDefault();
                void run(() => api("POST", "/api/v1/auth/password", { current: pw.current, new: pw.next }), "Password changed. Other devices were signed out.").then(() => setPw({ current: "", next: "" }));
              }}
            >
              <div className="form-grid">
                <Field label="Current password">
                  <input className="input" type="password" autoComplete="current-password" value={pw.current} onChange={(e) => setPw({ ...pw, current: e.target.value })} />
                </Field>
                <Field label="New password" hint="At least 12 characters. A sentence works well.">
                  <input className="input" type="password" autoComplete="new-password" value={pw.next} onChange={(e) => setPw({ ...pw, next: e.target.value })} />
                </Field>
              </div>
              <div>
                <button className="btn" disabled={busy || !pw.current || pw.next.length < 12}>
                  Change password
                </button>
              </div>
            </form>
          </section>
        )}

        <section className="panel">
          <header>
            <h2>API tokens</h2>
            <button className="btn sm" onClick={() => setCreating(true)}>
              <KeyRound /> New token
            </button>
          </header>
          <Loaded q={tokens} lines={2}>
            {(list) => <TokenTable tokens={list} onRevoke={(id) => run(() => api("DELETE", `/api/v1/tokens/${id}`), "Token revoked")} />}
          </Loaded>
        </section>

        <section className="panel">
          <header>
            <h2 className="row">
              <Bot width={17} height={17} /> Connect an AI assistant
            </h2>
            <CopyButton text={mcp} />
          </header>
          <div className="body stack">
            <p>
              Gantry speaks the Model Context Protocol. Add this server to an MCP-capable assistant and it can search, read and update work items and pages with your permissions. Every action it takes is recorded under your name. See the <a href="/manual/reference/mcp.html">MCP guide</a>.
            </p>
            <pre className="code">{mcp}</pre>
          </div>
        </section>
      </div>
      {creating && <TokenDialog onClose={() => setCreating(false)} allowAdmin={can("admin.access")} />}
    </div>
  );
}
