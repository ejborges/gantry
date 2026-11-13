// What the application tells people about itself: the organization's privacy, terms, cookie,
// retention and security statements; what the application itself does in each of those
// areas; the installed components and add-ons; and the licenses of everything it is built from.
// The statements are public, so they can be read from the sign-in page too.
import { useMemo, useState } from "react";
import { Link, NavLink, Route, Routes, useParams } from "react-router-dom";
import { Boxes, Cookie, FileText, Info, LifeBuoy, Lock, Scale, ShieldCheck, Timer } from "lucide-react";
import { useGet } from "../api";
import { Empty, Loaded, Markdown } from "../ui";
import { day } from "../lib/format";
import { SupportBlock, type Support } from "./Status";

interface AboutInfo {
  site: { name: string; support: Support };
  documents: { key: string; title: string; text: string }[];
  product: { name: string; license: string; version?: string };
}
// What Gantry itself does, stated as facts about the software. An organization's own
// statement is shown above these; it is the organization that decides policy.
const FACTS: Record<string, { lead: string; items: string[] }> = {
  privacy: {
    lead: "What this application stores about you",
    items: [
      "Your account: name, email address, the profile fields you or an administrator fill in, your picture if you upload one, and your notification choices.",
      "What you create: work items, comments, pages, files, dashboards and visualizations, with your name on them.",
      "A log of actions: who did what and when, and the network address it came from. Administrators and auditors can read it. The application offers no way to change it.",
      "What you see is decided by your permissions. Search, reports, dashboards and links to other work only ever include what your account may open.",
      "The application sends nothing to its makers or to other parties. It contacts other systems only where an administrator has set up an integration, sign-in provider or mail server.",
    ],
  },
  terms: {
    lead: "About the software",
    items: ["Gantry is open-source software under the Apache License 2.0, run by your organization on its own servers. Your organization decides the terms on which you may use this installation."],
  },
  cookies: {
    lead: "The cookies and browser storage this application uses",
    items: [
      "One cookie, named gantry_session (__Host-gantry_session when served over HTTPS). It keeps you signed in. Scripts on the page cannot read it, and browsers do not send it along with requests that other sites make. It ends when you sign out, after a period without use, and at the latest a fixed time after you signed in; an administrator sets both periods.",
      "No advertising, analytics or tracking cookies.",
      "No content is loaded from other sites: fonts, icons, and chart and diagram libraries are served by this installation. Pictures from other sites in a text are shown as links, not loaded.",
      "On this device, your browser remembers a few display choices: light or dark theme, whether an editor shows its preview, and how you last arranged a Gantt chart. They stay in the browser and are not sent anywhere.",
    ],
  },
  retention: {
    lead: "How long this application keeps things by itself",
    items: [
      "Work items, comments, pages, files, dashboards and the like are kept until someone deletes or archives them. Archiving hides; it does not delete.",
      "The audit log is kept. The application never deletes entries from it.",
      "Notifications in your inbox: 90 days.",
      "The record of emails sent (recipient, subject and outcome, not the content): 90 days.",
      "The record of alerts that fired: 180 days.",
      "The record of deliveries to integrations such as webhooks: 30 days.",
      "System performance samples: 7 days.",
      "Sign-in sessions: until they end. Unsaved text in an editor that several people had open: 30 days after the last change.",
      "Backups and their retention are set by the people who run this installation.",
    ],
  },
  security: {
    lead: "How this application protects data",
    items: [
      "Every request is checked against the permissions of the account making it. What an account may not open is answered as if it did not exist.",
      "Passwords are stored only as Argon2id hashes. Sessions and API tokens are stored as hashes too, so a copy of the database does not reveal them.",
      "AI agent accounts are marked as such everywhere, can do nothing until an administrator writes rules for them, and are limited to those rules through the API and MCP.",
      "Every action is logged with who, what, when and from where. The audit log is chained with hashes, so removing or changing an entry is detectable.",
      "Custom HTML and visualization scripts run in a sealed frame that cannot reach the network, the page around it, or your session. They can read data only as you, and can change nothing.",
      "Text you enter is cleaned before it is shown, and files are served so that the browser will not run them.",
      "Encryption in transit (HTTPS) and at rest is set up by the people who run this installation.",
    ],
  },
};
const ICON: Record<string, typeof Info> = { privacy: Lock, terms: Scale, cookies: Cookie, retention: Timer, security: ShieldCheck };

function Statement({ doc, support }: { doc: { key: string; title: string; text: string }; support: Support }) {
  const facts = FACTS[doc.key];
  return (
    <article className="stack loose" style={{ maxWidth: 820 }}>
      <h1>{doc.title}</h1>
      {doc.text.trim() ? (
        <section className="panel">
          <div className="body">
            <Markdown source={doc.text} />
          </div>
        </section>
      ) : (
        <div className="notice">Your organization has not written its own statement on this yet. Ask the contact below if you need one.</div>
      )}
      {facts && (
        <section className="panel">
          <header>
            <h2>{facts.lead}</h2>
          </header>
          <ul className="facts-list">
            {facts.items.map((x) => (
              <li key={x}>{x}</li>
            ))}
          </ul>
        </section>
      )}
      <SupportBlock support={support} heading="Questions about this" />
    </article>
  );
}

function Components() {
  const q = useGet<{ components: { key: string; name: string; version: string | null; date: string | null; summary: string; description: string }[]; addons: { id: string; name: string; version: string; description: string; installed_at: string }[]; storage: Record<string, string>; license: { name: string } }>("/api/v1/about/components");
  return (
    <Loaded q={q} what="Reading the installed components">
      {(d) => (
        <div className="stack loose">
          <section className="panel">
            <header>
              <h2>Components</h2>
            </header>
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">Component</th>
                    <th scope="col">Version</th>
                    <th scope="col">Date</th>
                    <th scope="col" className="hide-sm">What it is</th>
                  </tr>
                </thead>
                <tbody>
                  {d.components.map((c) => (
                    <tr key={c.key}>
                      <th scope="row" style={{ fontWeight: 400 }}>
                        <strong>{c.name}</strong>
                        <div className="small muted">{c.summary}</div>
                      </th>
                      <td className="num nowrap">{c.version ?? "Unknown"}</td>
                      <td className="nowrap">{c.date ? day(c.date) : <span className="muted">Not recorded</span>}</td>
                      <td className="hide-sm small">{c.description}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
          <section className="panel">
            <header>
              <h2>Add-ons</h2>
              <span className="small muted">Packs of work item types, workflows and templates</span>
            </header>
            <div className="table-wrap">
              <table className="table">
                <thead>
                  <tr>
                    <th scope="col">Add-on</th>
                    <th scope="col">Version</th>
                    <th scope="col">Installed</th>
                    <th scope="col" className="hide-sm">Description</th>
                  </tr>
                </thead>
                <tbody>
                  {d.addons.map((a) => (
                    <tr key={a.id}>
                      <th scope="row"><strong>{a.name}</strong></th>
                      <td className="num nowrap">{a.version}</td>
                      <td className="nowrap">{day(a.installed_at)}</td>
                      <td className="hide-sm small">{a.description}</td>
                    </tr>
                  ))}
                  {d.addons.length === 0 && (
                    <tr>
                      <td colSpan={4} className="muted">No add-ons are installed.</td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </section>
        </div>
      )}
    </Loaded>
  );
}

interface LicenseFile {
  generated: string;
  product: { name: string; version: string; license: string; text?: string };
  groups: { key: string; name: string; description: string; note?: string; packages: { name: string; version: string; license: string; url: string; texts: string[] }[] }[];
  summary: { license: string; count: number }[];
  texts: Record<string, string>;
}

function Licenses() {
  const q = useGet<LicenseFile>("/licenses.json", { staleTime: Infinity });
  const [find, setFind] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const term = find.trim().toLowerCase();
  return (
    <div className="stack loose">
      <h1>Licenses</h1>
      <Loaded q={q} what="Reading the list of software this is built from">
        {(d) => {
          const total = d.groups.reduce((n, g) => n + g.packages.length, 0);
          return (
            <>
              <section className="panel">
                <div className="body stack">
                  <p style={{ margin: 0 }}>
                    <strong>{d.product.name}</strong> is open-source software under the Apache License 2.0. It is built from {total.toLocaleString()} other open-source packages, listed here with their licenses as they were when this version was built ({day(d.generated)}).
                  </p>
                  {d.product.text && d.texts[d.product.text] && (
                    <details>
                      <summary>The Apache License 2.0</summary>
                      <pre className="license-text">{d.texts[d.product.text]}</pre>
                    </details>
                  )}
                  <p className="small muted" style={{ margin: 0 }}>
                    {d.summary.slice(0, 8).map((s) => `${s.license}: ${s.count}`).join(" · ")}
                    {d.summary.length > 8 ? ` · and ${d.summary.length - 8} other licenses` : ""}
                  </p>
                </div>
              </section>
              <div className="filters">
                <input className="input search" type="search" value={find} onChange={(e) => setFind(e.target.value)} placeholder="Find a package or a license" aria-label="Find a package or a license" />
              </div>
              {d.groups.map((g) => {
                const list = g.packages.filter((p) => !term || p.name.toLowerCase().includes(term) || p.license.toLowerCase().includes(term));
                return (
                  <section key={g.key} className="panel">
                    <header>
                      <h2>
                        {g.name} <span className="count num">{list.length}</span>
                      </h2>
                      <span className="small muted">{g.description}</span>
                    </header>
                    {g.note && <p className="muted small" style={{ padding: "10px 14px 0" }}>{g.note}</p>}
                    {list.length === 0 && <p className="muted" style={{ padding: 14 }}>{term ? "Nothing here matches." : "None."}</p>}
                    <ul className="license-list">
                      {list.slice(0, term ? 400 : 1000).map((p) => {
                        const id = `${g.key}:${p.name}@${p.version}`;
                        return (
                          <li key={id}>
                            <div className="list-row wrap">
                              <span className="grow" style={{ minWidth: 0 }}>
                                <strong>{p.name}</strong> <span className="muted num">{p.version}</span>
                              </span>
                              <span className="badge outline">{p.license}</span>
                              {p.url && /^https?:\/\//.test(p.url) && (
                                <a className="small" href={p.url} rel="noopener noreferrer" target="_blank">
                                  Source
                                </a>
                              )}
                              {p.texts.length > 0 && (
                                <button className="btn ghost sm" aria-expanded={open === id} onClick={() => setOpen(open === id ? null : id)}>
                                  {open === id ? "Hide the license" : "License text"}
                                </button>
                              )}
                            </div>
                            {open === id && p.texts.map((t) => <pre key={t} className="license-text">{d.texts[t]}</pre>)}
                          </li>
                        );
                      })}
                    </ul>
                  </section>
                );
              })}
            </>
          );
        }}
      </Loaded>
    </div>
  );
}

function Overview({ info }: { info: AboutInfo }) {
  return (
    <div className="stack loose">
      <h1>About {info.site.name}</h1>
      <section className="panel">
        <div className="body stack">
          <p style={{ margin: 0 }}>
            {info.site.name === info.product.name ? "This" : `${info.site.name}`} is an installation of <strong>{info.product.name}</strong>
            {info.product.version ? `, version ${info.product.version}` : ""}: project management, issue tracking, a service desk, and documentation, run by your organization on its own servers.
          </p>
          <p className="small muted" style={{ margin: 0 }}>
            Open-source software under the Apache License 2.0. <Link to="/about/licenses">Licenses of everything it is built from</Link> · <a href="/manual/" target="_blank" rel="noreferrer">The manual</a>
          </p>
        </div>
      </section>
      <Components />
      <SupportBlock support={info.site.support} />
    </div>
  );
}

function DocRoute({ info }: { info: AboutInfo }) {
  const { doc } = useParams();
  const d = info.documents.find((x) => x.key === doc);
  if (!d) return <Empty title="There is no such statement" />;
  return <Statement doc={d} support={info.site.support} />;
}

function Frame({ base, children, signedIn }: { base: string; children: (info: AboutInfo) => React.ReactNode; signedIn: boolean }) {
  const q = useGet<AboutInfo>("/api/v1/about", { staleTime: 60_000 });
  const cls = ({ isActive }: { isActive: boolean }) => (isActive ? "active" : "");
  return (
    <Loaded q={q} what="Loading">
      {(info) => (
        <div className="about">
          <nav className="about-nav" aria-label="About this application">
            {signedIn && (
              <NavLink to={base} end className={cls}>
                <Info /> About
              </NavLink>
            )}
            {info.documents.map((d) => {
              const I = ICON[d.key] ?? FileText;
              return (
                <NavLink key={d.key} to={`${base}/${d.key}`} className={cls}>
                  <I /> {d.title}
                </NavLink>
              );
            })}
            {signedIn && (
              <NavLink to={`${base}/licenses`} className={cls}>
                <Boxes /> Licenses
              </NavLink>
            )}
            {signedIn && (
              <NavLink to="/status" className={cls}>
                <LifeBuoy /> Error messages and help
              </NavLink>
            )}
          </nav>
          <div className="about-main">{children(info)}</div>
        </div>
      )}
    </Loaded>
  );
}

/** Inside the application, for signed-in people. */
export default function AboutRoutes() {
  return (
    <div className="page">
      <Frame base="/about" signedIn>
        {(info) => (
          <Routes>
            <Route index element={<Overview info={info} />} />
            <Route path="licenses" element={<Licenses />} />
            <Route path=":doc" element={<DocRoute info={info} />} />
          </Routes>
        )}
      </Frame>
    </div>
  );
}

/** The statements alone, for people who are not signed in (linked from the sign-in page). */
export function LegalRoutes() {
  const site = useGet<AboutInfo>("/api/v1/about", { staleTime: 60_000 });
  const name = useMemo(() => site.data?.site.name ?? "Gantry", [site.data]);
  return (
    <div className="public-page">
      <header className="public-head">
        <strong>{name}</strong>
        <a className="btn sm" href="/login">Sign in</a>
      </header>
      <main className="page">
        <Frame base="/legal" signedIn={false}>
          {(info) => (
            <Routes>
              <Route index element={<Statement doc={info.documents[0]} support={info.site.support} />} />
              <Route path=":doc" element={<DocRoute info={info} />} />
            </Routes>
          )}
        </Frame>
      </main>
    </div>
  );
}
