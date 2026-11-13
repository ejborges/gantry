import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { KeyRound } from "lucide-react";
import { api, ApiError, queryClient, useGet } from "../api";
import { Field, Spinner, setSiteName } from "../ui";

function Truss() {
  // A gantry: the lattice structure that holds work while it is assembled and serviced.
  const bays = [0, 1, 2, 3, 4, 5, 6];
  return (
    <svg className="truss" viewBox="0 0 700 600" preserveAspectRatio="xMaxYMin slice" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round">
      {bays.map((i) => (
        <g key={i} transform={`translate(${430 + (i % 2) * 0}, ${-20 + i * 86})`}>
          <path d="M0 0h150M0 86h150M0 0v86M150 0v86" opacity="0.55" />
          <path d={i % 2 ? "M0 0l150 86" : "M150 0L0 86"} opacity="0.9" />
        </g>
      ))}
      <path d="M580 66h120M580 238h90M580 410h120" opacity="0.55" />
    </svg>
  );
}

export default function Login() {
  const [params] = useSearchParams();
  const options = useGet<{ local_login: boolean; providers: { id: string; name: string; kind?: string; start?: string }[]; site: { name?: string; login_notice?: string } }>("/api/v1/auth/providers");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState(params.get("error") === "sso" ? "Single sign-on did not complete. Try again, or ask an administrator to check your account." : "");
  const [busy, setBusy] = useState(false);
  const next = params.get("next");
  const dest = next && next.startsWith("/") && !next.startsWith("//") ? next : "/";

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      await api("POST", "/api/v1/auth/login", { email, password });
      queryClient.clear();
      window.location.assign(dest);
    } catch (err) {
      const status = err instanceof ApiError ? err.status : 0;
      setError(status === 401 ? "That email and password do not match an active account. After several failed attempts an account is locked for a few minutes." : status === 429 ? "Too many attempts from this network. Wait a few minutes and try again." : err instanceof Error ? err.message : "Sign-in failed.");
      setBusy(false);
    }
  }

  const site = options.data?.site?.name || "Gantry";
  useEffect(() => setSiteName(options.data?.site?.name), [options.data?.site?.name]);
  return (
    <div className="login">
      <div className="login-art">
        <Truss />
        <p className="slogan">Work, held steady while it gets done.</p>
        <p>Projects, requests and documentation for every team in one place, with a record of who changed what.</p>
      </div>
      <div className="login-form">
        <div className="stack loose">
          <div className="stack tight">
            <h1>Sign in to {site}</h1>
            {options.data?.site?.login_notice && <p className="muted">{options.data.site.login_notice}</p>}
          </div>
          {error && (
            <div className="notice danger" role="alert">
              {error}
            </div>
          )}
          {options.data?.providers.map((p) => (
            <a key={p.id} className="btn block" href={p.start ?? `/api/v1/auth/oidc/${p.id}/start`}>
              <KeyRound /> Continue with {p.name}
            </a>
          ))}
          {options.data && options.data.providers.length > 0 && options.data.local_login && <div className="divider">or use a password</div>}
          {options.data?.local_login !== false && (
            <form className="stack" onSubmit={submit}>
              <Field label="Email">
                <input className="input" type="email" name="email" autoComplete="username" required autoFocus value={email} onChange={(e) => setEmail(e.target.value)} />
              </Field>
              <Field label="Password">
                <input className="input" type="password" name="password" autoComplete="current-password" required value={password} onChange={(e) => setPassword(e.target.value)} />
              </Field>
              <button className="btn primary block" disabled={busy}>
                {busy ? <Spinner /> : null} Sign in
              </button>
            </form>
          )}
          <p className="small muted">
            Trouble signing in? See the <a href="/manual/user/signing-in.html">sign-in guide</a>.
          </p>
          <nav className="small login-legal" aria-label="About this site">
            <a href="/legal/privacy">Privacy</a>
            <a href="/legal/terms">Terms</a>
            <a href="/legal/cookies">Cookies</a>
            <a href="/legal/security">Data security</a>
          </nav>
        </div>
      </div>
    </div>
  );
}
