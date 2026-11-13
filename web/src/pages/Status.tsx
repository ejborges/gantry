// A page for each response code: what happened, what to do, and how to reach support.
// The texts come from the server (GET /api/v1/about/status-codes), which shows the same ones
// when a browser opens a failing address directly. If even that cannot be fetched, a short
// built-in text is shown, because this page must work when little else does.
import { Component, type ErrorInfo, type ReactNode } from "react";
import { Link, useLocation, useParams } from "react-router-dom";
import { useGet } from "../api";
import { LifeBuoy } from "lucide-react";

export interface Support {
  name: string;
  email: string;
  url: string;
  text: string;
}

/** How to reach the people who run this installation. */
export function SupportBlock({ support, heading = "Getting help" }: { support: Support | undefined; heading?: string }) {
  const s = support;
  const url = s?.url && /^https?:\/\//.test(s.url) ? s.url : null;
  const email = s?.email && /^[^\s@<>"]+@[^\s@<>"]+$/.test(s.email) ? s.email : null;
  return (
    <section className="support" aria-label={heading}>
      <h2>
        <LifeBuoy aria-hidden="true" /> {heading}
      </h2>
      {s?.name && <p><strong>{s.name}</strong></p>}
      {s?.text && <p>{s.text}</p>}
      {email && (
        <p>
          Email: <a href={`mailto:${email}`}>{email}</a>
        </p>
      )}
      {url && (
        <p>
          Web: <a href={url} rel="noopener noreferrer">{url}</a>
        </p>
      )}
      {!s?.text && !email && !url && <p>Contact the people who run this installation in your organization.</p>}
    </section>
  );
}

interface Codes {
  codes: { code: number; title: string; meaning: string; action: string }[];
  support: Support;
}
const FALLBACK: Record<string, { title: string; meaning: string; action: string }> = {
  "404": { title: "This does not exist, or you cannot see it", meaning: "Nothing is at this address for your account. It may have been moved, archived or deleted, or it may be in a project or space you are not a member of.", action: "Check the address, use search, or ask the person who sent you the link." },
  "0": { title: "The server cannot be reached", meaning: "Your browser did not get an answer. You may be offline, or the application may be restarting.", action: "Check your connection, wait a minute, and reload." },
  app: { title: "This page ran into a problem", meaning: "Something in the page failed while it was being shown. Your work on the server is not affected.", action: "Reload the page. If it happens again, tell support what you were doing." },
};

export function StatusView({ code, reference, detail }: { code: number | "app"; reference?: string; detail?: string }) {
  const q = useGet<Codes>("/api/v1/about/status-codes", { staleTime: 300_000, retry: false });
  const listed = typeof code === "number" ? q.data?.codes.find((c) => c.code === code) : undefined;
  const t = listed ?? FALLBACK[String(code)] ?? (typeof code === "number" && code >= 500 ? { title: "The server could not complete the request", meaning: "Something went wrong on the server's side.", action: "Try again in a moment. If it keeps happening, tell support and include the reference below." } : { title: "The request could not be completed", meaning: "The server refused or could not process the request.", action: "Go back and try again." });
  return (
    <div className="status-page">
      <p className="status-code">{code === "app" ? "Application error" : code === 0 ? "No connection" : `Error ${code}`}</p>
      <h1>{t.title}</h1>
      <p>{t.meaning}</p>
      {detail && <p className="muted">{detail}</p>}
      <h2>What you can do</h2>
      <p>{t.action}</p>
      <div className="row wrap" style={{ marginTop: 14 }}>
        {code === 401 ? (
          <a className="btn primary" href="/login">Sign in</a>
        ) : (
          <a className="btn primary" href="/">Go to my work</a>
        )}
        <button className="btn" onClick={() => window.location.reload()}>Reload</button>
        <button className="btn" onClick={() => window.history.back()}>Go back</button>
      </div>
      <SupportBlock support={q.data?.support} />
      <p className="small muted status-ref">
        When you ask for help, say what you were doing and when.
        {reference ? (
          <>
            {" "}Reference: <code>{reference}</code>.
          </>
        ) : null}
      </p>
    </div>
  );
}

/** /status/:code, for links from the manual and for looking a code up. */
export function StatusRoute() {
  const { code } = useParams();
  const n = Number(code);
  const reference = new URLSearchParams(useLocation().search).get("ref") ?? undefined;
  return (
    <div className="page narrow">
      <StatusView code={Number.isInteger(n) && n >= 400 && n < 600 ? n : 404} reference={reference && /^[\w-]{1,64}$/.test(reference) ? reference : undefined} />
    </div>
  );
}

/** Every address the application does not know. */
export function NotFound() {
  return (
    <div className="page narrow">
      <StatusView code={404} />
    </div>
  );
}

/** The list of all codes, for the curious and for support staff. */
export function StatusList() {
  const q = useGet<Codes>("/api/v1/about/status-codes", { staleTime: 300_000 });
  return (
    <div className="page narrow">
      <div className="page-head">
        <div>
          <h1>When something goes wrong</h1>
          <p className="sub">What each error means and what to do about it.</p>
        </div>
      </div>
      <section className="panel">
        {(q.data?.codes ?? []).map((c) => (
          <Link key={c.code} to={`/status/${c.code}`} className="list-row">
            <span className="key num">{c.code}</span>
            <span className="grow stack tight">
              <strong>{c.title}</strong>
              <span className="small muted">{c.meaning}</span>
            </span>
          </Link>
        ))}
      </section>
      <SupportBlock support={q.data?.support} />
    </div>
  );
}

/** Catches a failure while drawing a page, so one broken view never leaves a blank screen. */
export class ErrorBoundary extends Component<{ children: ReactNode; resetKey?: string }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidCatch(error: Error, info: ErrorInfo) {
    // The browser console is where a developer or support engineer will look.
    console.error("A page failed while rendering", error, info.componentStack);
  }
  componentDidUpdate(prev: { resetKey?: string }) {
    // Moving to another page gives the application another chance.
    if (this.state.failed && prev.resetKey !== this.props.resetKey) this.setState({ failed: false });
  }
  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="page narrow">
        <StatusView code="app" />
      </div>
    );
  }
}
