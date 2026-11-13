// Parts of a project's settings that came with planning: where the project sits in the
// hierarchy, who leads it, and a workflow of its own for any of its work item types.
import { lazy, Suspense, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { GitBranch, Pencil, Undo2 } from "lucide-react";
import { api, useGet, type ItemType, type Person } from "../api";
import { Dialog, Field, Loading, PersonSelect, TypeIcon, useAction } from "../ui";
import type { ProjectDetail } from "../work";
import { problems, removedStages, type WorkflowDef } from "../lib/workflow";

const WorkflowEditor = lazy(() => import("../components/WorkflowEditor"));

const strip = (w: any): WorkflowDef => ({ initial: w.initial, statuses: w.statuses, transitions: w.transitions, layout: w.layout ?? null });

/** Where work goes when the stage it is in disappears. */
export function StageMapping({ before, after, value, onChange }: { before: WorkflowDef; after: WorkflowDef; value: Record<string, string>; onChange: (m: Record<string, string>) => void }) {
  const gone = removedStages(before, after);
  if (gone.length === 0 || after.statuses.length === 0) return null;
  return (
    <fieldset className="notice warn stack tight" style={{ display: "block" }}>
      <legend className="label">Work that is in a status you removed moves to:</legend>
      {gone.map((s) => (
        <label key={s.key} className="row wrap" style={{ gap: 8, marginTop: 6 }}>
          <span style={{ minWidth: 160 }}>{s.name}</span>
          <span aria-hidden="true">→</span>
          <select className="select" style={{ maxWidth: 240 }} value={value[s.key] ?? after.initial} aria-label={`Work in ${s.name} moves to`} onChange={(e) => onChange({ ...value, [s.key]: e.target.value })}>
            {after.statuses.map((n) => (
              <option key={n.key} value={n.key}>{n.name}</option>
            ))}
          </select>
        </label>
      ))}
      <p className="small" style={{ marginTop: 6 }}>The move is recorded in each work item's history.</p>
    </fieldset>
  );
}

function WorkflowDialog({ projectKey, type, onClose }: { projectKey: string; type: ItemType & { shared_workflow?: any }; onClose: () => void }) {
  const before = useMemo(() => strip(type.workflow), [type]);
  const [def, setDef] = useState<WorkflowDef>(before);
  const [mapping, setMapping] = useState<Record<string, string>>({});
  const [busy, run] = useAction();
  const issues = problems(def);
  const own = (type.workflow as any).own;
  function save() {
    const gone = removedStages(before, def);
    const full = Object.fromEntries(gone.map((s) => [s.key, mapping[s.key] && def.statuses.some((n) => n.key === mapping[s.key]) ? mapping[s.key] : def.initial]));
    return run(() => api("PUT", `/api/v1/projects/${projectKey}/workflows/${type.key}`, { definition: def, mapping: full }), `${type.name} now follows its own workflow in this project`).then((r) => r !== undefined && onClose());
  }
  return (
    <Dialog
      wide="canvas"
      title={`Workflow for ${type.name.toLowerCase()} in ${projectKey}`}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={busy || issues.length > 0} onClick={save}>
            Save workflow
          </button>
        </>
      }
    >
      <div className="stack">
        <p className="small muted" style={{ margin: 0 }}>
          {own ? "This project has its own workflow for this type." : `This starts from the shared workflow “${type.workflow.name}”. Saving gives this project its own copy; other projects are not affected.`}
        </p>
        <Suspense fallback={<Loading what="Opening the workflow editor" />}>
          <WorkflowEditor value={before} onChange={setDef} fields={type.fields} />
        </Suspense>
        <StageMapping before={before} after={def} value={mapping} onChange={setMapping} />
      </div>
    </Dialog>
  );
}

function RevertDialog({ projectKey, type, onClose }: { projectKey: string; type: ItemType & { shared_workflow?: any }; onClose: () => void }) {
  const before = useMemo(() => strip(type.workflow), [type]);
  const shared = useMemo(() => strip(type.shared_workflow), [type]);
  const [mapping, setMapping] = useState<Record<string, string>>({});
  const [busy, run] = useAction();
  const gone = removedStages(before, shared);
  return (
    <Dialog
      title={`Return ${type.name.toLowerCase()} to the shared workflow?`}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={busy} onClick={() => run(() => api("PUT", `/api/v1/projects/${projectKey}/workflows/${type.key}`, { definition: null, mapping: Object.fromEntries(gone.map((s) => [s.key, mapping[s.key] ?? shared.initial])) }), "Back on the shared workflow").then((r) => r !== undefined && onClose())}>
            Use the shared workflow
          </button>
        </>
      }
    >
      <div className="stack">
        <p style={{ margin: 0 }}>
          This project's own workflow for {type.name.toLowerCase()} is discarded, and it follows “{type.workflow.name}” again: {shared.statuses.map((s) => s.name).join(", ")}.
        </p>
        <StageMapping before={before} after={shared} value={mapping} onChange={setMapping} />
      </div>
    </Dialog>
  );
}

export function ProjectWorkflows({ detail }: { detail: ProjectDetail }) {
  const [edit, setEdit] = useState<string | null>(null);
  const [revert, setRevert] = useState<string | null>(null);
  const types = detail.types as (ItemType & { shared_workflow?: any })[];
  return (
    <section className="panel">
      <header>
        <h2>Workflows</h2>
        <span className="small muted">The statuses each type of work moves through here, and the moves between them.</span>
      </header>
      {types.map((t) => {
        const own = (t.workflow as any).own;
        return (
          <div key={t.key} className="list-row wrap">
            <TypeIcon type={t} title={false} />
            <span style={{ minWidth: 130 }}>{t.name}</span>
            <span className="grow small muted truncate" title={t.workflow.statuses.map((s) => s.name).join(" → ")}>
              {own ? <span className="badge outline"><GitBranch /> Own workflow</span> : <>Shared: {t.workflow.name}</>} · {t.workflow.statuses.map((s) => s.name).join(" → ")}
            </span>
            {own && (
              <button className="btn ghost sm" onClick={() => setRevert(t.key)}>
                <Undo2 /> Use the shared one
              </button>
            )}
            <button className="btn sm" onClick={() => setEdit(t.key)}>
              <Pencil /> {own ? "Edit" : "Design its own"}
            </button>
          </div>
        );
      })}
      {edit && <WorkflowDialog projectKey={detail.project.key} type={types.find((t) => t.key === edit)!} onClose={() => setEdit(null)} />}
      {revert && <RevertDialog projectKey={detail.project.key} type={types.find((t) => t.key === revert)!} onClose={() => setRevert(null)} />}
    </section>
  );
}

export function ProjectPlacement({ detail, people }: { detail: ProjectDetail; people: Person[] }) {
  const all = useGet<{ containers: { id: string; name: string; level: { name: string; position: number } }[]; levels: unknown[] }>("/api/v1/containers");
  const [busy, run] = useAction();
  const chain = detail.container ?? [];
  const current = chain.length ? chain[chain.length - 1].id : "";
  const key = detail.project.key;
  return (
    <section className="panel">
      <header>
        <h2>Lead and place in the hierarchy</h2>
      </header>
      <div className="body form-grid">
        <Field label="Project lead" hint="Shown on the project, in the hierarchy graph and in reports. The lead can always open this project's report; nothing else changes.">
          <PersonSelect people={people.filter((p) => p.kind !== "agent")} value={detail.lead?.id ?? null} none="Nobody" onChange={(id) => run(() => api("PATCH", `/api/v1/projects/${key}`, { lead_id: id }), "Lead saved")} />
        </Field>
        <Field
          label="Part of"
          hint={
            (all.data?.levels.length ?? 0) === 0 ? (
              <>No levels above projects are defined yet. <Link to="/planning/levels">See the levels</Link></>
            ) : chain.length ? (
              chain.map((c) => `${c.level}: ${c.name}`).join(" › ")
            ) : (
              "Its owner must let you add to it, or you must be allowed to manage planning."
            )
          }
        >
          <select className="select" value={current} disabled={busy || !all.data} onChange={(e) => run(() => api("PUT", `/api/v1/projects/${key}/container`, { container_id: e.target.value || null }), e.target.value ? "Project placed" : "Project taken out")}>
            <option value="">Nothing</option>
            {(all.data?.containers ?? []).map((c) => (
              <option key={c.id} value={c.id}>{c.level.name}: {c.name}</option>
            ))}
          </select>
        </Field>
      </div>
    </section>
  );
}
