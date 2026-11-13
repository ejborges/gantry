import { useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { FolderKanban, LifeBuoy, Lock, Plus, Users } from "lucide-react";
import { api, useGet } from "../api";
import { useLiveRefresh } from "../realtime";
import { useSession } from "../session";
import { Dialog, Empty, Field, Loaded, useAction } from "../ui";

interface Template {
  key: string;
  name: string;
  domain: string;
  kind: string;
  description: string;
}

function suggestKey(name: string) {
  const words = name.toUpperCase().replace(/[^A-Z0-9 ]/g, "").split(/\s+/).filter(Boolean);
  const k = words.length > 1 ? words.map((w) => w[0]).join("") : (words[0] ?? "").slice(0, 4);
  return k.replace(/^[0-9]+/, "").slice(0, 6);
}

function NewProject({ onClose }: { onClose: () => void }) {
  const templates = useGet<Template[]>("/api/v1/project-templates");
  const navigate = useNavigate();
  const [busy, run] = useAction();
  const [template, setTemplate] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [key, setKey] = useState("");
  const [keyTouched, setKeyTouched] = useState(false);
  const [access, setAccess] = useState("private");
  const groups = useMemo(() => {
    const by = new Map<string, Template[]>();
    for (const t of templates.data ?? []) by.set(t.domain, [...(by.get(t.domain) ?? []), t]);
    return [...by.entries()];
  }, [templates.data]);
  const k = keyTouched ? key : suggestKey(name);
  const keyOk = /^[A-Z][A-Z0-9]{1,9}$/.test(k);

  return (
    <Dialog
      title="New project"
      onClose={onClose}
      wide
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn primary"
            disabled={busy || !template || !name.trim() || !keyOk}
            onClick={() => run(() => api("POST", "/api/v1/projects", { key: k, name, template, access }), (p: any) => (onClose(), navigate(`/projects/${p.key}`)))}
          >
            Create project
          </button>
        </>
      }
    >
      <div className="stack loose">
        <div className="form-grid">
          <Field label="Name" required>
            <input className="input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Pump station upgrade" />
          </Field>
          <Field label="Key" required hint={keyOk || !k ? "Prefix for item numbers, like OPS-12. Cannot be changed later." : "2 to 10 capital letters or digits, starting with a letter."}>
            <input className="input" value={k} onChange={(e) => (setKeyTouched(true), setKey(e.target.value.toUpperCase()))} maxLength={10} />
          </Field>
          <Field label="Who can see it">
            <select className="select" value={access} onChange={(e) => setAccess(e.target.value)}>
              <option value="private">Only people I add</option>
              <option value="internal">Everyone in the workspace</option>
            </select>
          </Field>
        </div>
        <div className="stack">
          <span className="label">Start from a template</span>
          <Loaded q={templates}>
            {() => (
              <div className="stack">
                {groups.map(([domain, list]) => (
                  <div key={domain} className="stack tight">
                    <span className="small muted">{domain}</span>
                    <div className="card-grid" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(230px, 1fr))", gap: 8 }}>
                      {list.map((t) => (
                        <button key={t.key} type="button" className="card" aria-pressed={template === t.key} onClick={() => setTemplate(t.key)} style={{ padding: 12, gap: 4 }}>
                          <strong>{t.name}</strong>
                          <span className="desc small">{t.description}</span>
                        </button>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </Loaded>
        </div>
      </div>
    </Dialog>
  );
}

export default function Projects() {
  const { can } = useSession();
  const [params, setParams] = useSearchParams();
  useLiveRefresh(["work", "planning"]);
  const projects = useGet<any[]>("/api/v1/projects");
  const creating = params.get("new") === "1";
  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Projects</h1>
          <p className="sub">Each project has its own work item types, workflow and members.</p>
        </div>
        {can("project.create") && (
          <button className="btn primary" onClick={() => setParams({ new: "1" })}>
            <Plus /> New project
          </button>
        )}
      </div>
      <Loaded q={projects}>
        {(list) =>
          list.length === 0 ? (
            <div className="panel">
              <Empty icon={<FolderKanban />} title="No projects yet">
                <p>A project holds the work of one team or program. Start from a template for software, engineering change, research, finance, legal, logistics, procurement, safety or a service desk.</p>
                {can("project.create") && (
                  <button className="btn primary" onClick={() => setParams({ new: "1" })}>
                    <Plus /> Create the first project
                  </button>
                )}
              </Empty>
            </div>
          ) : (
            <div className="card-grid">
              {list.map((p) => (
                <Link key={p.key} to={`/projects/${p.key}`} className="card">
                  <div className="spread">
                    <span className="row">
                      <span className="key">{p.key}</span>
                      {p.kind === "service_desk" && (
                        <span className="badge teal">
                          <LifeBuoy /> Service desk
                        </span>
                      )}
                    </span>
                    <span className="muted" title={p.access === "private" ? "Only members can see this project" : "Visible to everyone in the workspace"}>
                      {p.access === "private" ? <Lock width={14} height={14} /> : <Users width={14} height={14} />}
                    </span>
                  </div>
                  <h2>{p.name}</h2>
                  {p.description && <p className="desc">{p.description}</p>}
                  <div className="row small muted" style={{ marginTop: "auto" }}>
                    <span>
                      <strong style={{ color: "var(--ink)" }}>{p.open_items}</strong> open of {p.total_items}
                    </span>
                    {p.lead && <span>Lead: {p.lead.name}</span>}
                  </div>
                </Link>
              ))}
            </div>
          )
        }
      </Loaded>
      {creating && <NewProject onClose={() => setParams({})} />}
    </div>
  );
}
