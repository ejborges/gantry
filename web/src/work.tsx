// Pieces shared by the pages that show work items.
import { useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { CalendarDays } from "lucide-react";
import { api, useGet, type Item, type ItemType, type Person } from "./api";
import { Avatar, CustomFieldInput, Dialog, Empty, Field, PersonSelect, Priority, SlaChip, StatusBadge, TypeIcon, useAction } from "./ui";
import { day, isOverdue, PRIORITIES, relative } from "./lib/format";

export interface ProjectDetail {
  project: { id: string; key: string; name: string; description: string; kind: "standard" | "service_desk"; access: string; default_type: string; settings: any; archived: boolean };
  permissions: string[];
  types: ItemType[];
  cycles: { id: string; name: string; state: string; start_date: string | null; end_date: string | null; goal: string }[];
  members: { binding_id: string; role: string; role_name: string; user: Person | null; group: { id: string; name: string } | null }[];
  space: { key: string; name: string; kind: string } | null;
  counts: Record<string, number>;
  lead?: Person | null;
  /** The levels above this project, from the top down to the one it is placed in. */
  container?: { id: string; name: string; level: string }[];
}

export function useProject(key: string | undefined) {
  return useGet<ProjectDetail>(key ? `/api/v1/projects/${key}` : null);
}
export function usePeople() {
  return useGet<Person[]>("/api/v1/users", { staleTime: 120_000 }).data ?? [];
}

/** Status display name for an item, looked up in its type's workflow. */
export function statusName(types: ItemType[] | undefined, item: { type: string; status: string; status_name?: string }): string {
  if (item.status_name) return item.status_name;
  const s = types?.find((t) => t.key === item.type)?.workflow.statuses.find((x) => x.key === item.status);
  return s?.name ?? item.status.replace(/_/g, " ");
}
/** Icon, color and name of an item's type; list responses carry them, so no project lookup is needed. */
export function typeOf(types: ItemType[] | undefined, item: Item) {
  return types?.find((t) => t.key === item.type) ?? { name: item.type_name ?? item.type, icon: item.type_icon ?? "circle", color: item.type_color ?? "neutral" };
}

export function DueDate({ item }: { item: Item }) {
  if (!item.due_date) return null;
  const late = isOverdue(item.due_date, item.status_category === "done");
  return (
    <span className={`row small nowrap ${late ? "" : "muted"}`} style={{ gap: 4, color: late ? "var(--danger)" : undefined, fontWeight: late ? 600 : 400 }} title={late ? "Overdue" : "Due date"}>
      <CalendarDays width={13} height={13} />
      {day(item.due_date)}
    </span>
  );
}

export function ItemTable({ items, types, showProject, showSla, hideAssignee, empty }: { items: Item[]; types?: ItemType[]; showProject?: boolean; showSla?: boolean; hideAssignee?: boolean; empty?: React.ReactNode }) {
  const navigate = useNavigate();
  if (items.length === 0) return <>{empty ?? <Empty title="Nothing here" />}</>;
  return (
    <div className="table-wrap">
      <table className="table items">
        <thead>
          <tr>
            <th>Item</th>
            <th>Status</th>
            {showSla && <th>Target</th>}
            <th className="hide-sm">Priority</th>
            {!hideAssignee && <th className="hide-sm">Assignee</th>}
            <th className="hide-sm">Due</th>
            {!hideAssignee && <th className="hide-sm">Updated</th>}
          </tr>
        </thead>
        <tbody>
          {items.map((i) => {
            const type = typeOf(types, i);
            return (
              <tr key={i.id} className="click" onClick={() => navigate(`/i/${i.key}`)}>
                <td className="title-cell">
                  <div className="row">
                    <TypeIcon type={type} />
                    <span className="key">{i.key}</span>
                    <Link to={`/i/${i.key}`} className="title truncate" onClick={(e) => e.stopPropagation()}>
                      {i.title}
                    </Link>
                  </div>
                  {showProject && <div className="small muted" style={{ marginLeft: 28 }}>{i.project}</div>}
                </td>
                <td>
                  <StatusBadge name={statusName(types, i)} category={i.status_category} />
                </td>
                {showSla && (
                  <td>{i.sla && <SlaChip due={i.sla.resolution_due} breached={i.sla.resolution_breached} metAt={i.resolved_at ?? null} what="Resolution" />}</td>
                )}
                <td className="hide-sm">
                  <Priority value={i.priority} />
                </td>
                {!hideAssignee && (
                  <td className="hide-sm">
                    <div className="row" style={{ gap: 6 }}>
                      <Avatar person={i.assignee} />
                      <span className="truncate" style={{ maxWidth: 130 }}>{i.assignee?.name ?? <span className="muted">Unassigned</span>}</span>
                    </div>
                  </td>
                )}
                <td className="hide-sm">
                  <DueDate item={i} />
                </td>
                {!hideAssignee && <td className="hide-sm muted small nowrap">{relative(i.updated_at)}</td>}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function NewItemDialog({ detail, onClose, defaults }: { detail: ProjectDetail; onClose: () => void; defaults?: { type?: string; parent?: string; cycle_id?: string } }) {
  const people = usePeople();
  const navigate = useNavigate();
  const [busy, run] = useAction();
  const [typeKey, setTypeKey] = useState(defaults?.type ?? detail.project.default_type);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [priority, setPriority] = useState(2);
  const [assignee, setAssignee] = useState<string | null>(null);
  const [due, setDue] = useState("");
  const [custom, setCustom] = useState<Record<string, any>>({});
  const [another, setAnother] = useState(false);
  const type = useMemo(() => detail.types.find((t) => t.key === typeKey) ?? detail.types[0], [detail.types, typeKey]);
  const canAssign = detail.permissions.includes("item.assign");

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const fields = Object.fromEntries(Object.entries(custom).filter(([k, v]) => v !== null && v !== "" && type.fields.some((f) => f.key === k)));
    void run(
      () => api<Item>("POST", `/api/v1/projects/${detail.project.key}/items`, { type: type.key, title, description, priority, assignee_id: assignee ?? undefined, due_date: due || undefined, custom: fields, parent: defaults?.parent, cycle_id: defaults?.cycle_id }),
      (item) => {
        if (another) {
          setTitle("");
          setDescription("");
        } else {
          onClose();
          navigate(`/i/${item.key}`);
        }
      },
    );
  }
  return (
    <Dialog
      title={`New ${type.name.toLowerCase()} in ${detail.project.name}`}
      onClose={onClose}
      wide
      footer={
        <>
          <label className="check" style={{ marginRight: "auto" }}>
            <input type="checkbox" checked={another} onChange={(e) => setAnother(e.target.checked)} /> Create another
          </label>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn primary" form="new-item" disabled={busy || !title.trim()}>
            Create {type.name.toLowerCase()}
          </button>
        </>
      }
    >
      <form id="new-item" className="stack" onSubmit={submit}>
        <div className="form-grid">
          <Field label="Type" hint={type.description}>
            <select className="select" value={type.key} onChange={(e) => setTypeKey(e.target.value)}>
              {detail.types.map((t) => (
                <option key={t.key} value={t.key}>
                  {t.name}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Priority">
            <select className="select" value={priority} onChange={(e) => setPriority(Number(e.target.value))}>
              {PRIORITIES.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label="Title" required>
          <input className="input" autoFocus value={title} maxLength={500} onChange={(e) => setTitle(e.target.value)} placeholder="What needs to happen?" />
        </Field>
        <Field label="Description" hint="Markdown is supported.">
          <textarea className="textarea" value={description} onChange={(e) => setDescription(e.target.value)} rows={4} />
        </Field>
        <div className="form-grid">
          {canAssign && (
            <Field label="Assignee">
              <PersonSelect people={people} value={assignee} onChange={setAssignee} />
            </Field>
          )}
          <Field label="Due date">
            <input className="input" type="date" value={due} onChange={(e) => setDue(e.target.value)} />
          </Field>
        </div>
        {type.fields.length > 0 && (
          <div className="form-grid">
            {type.fields.map((f) => (
              <Field key={f.key} label={f.name} required={f.required} hint={f.description}>
                <CustomFieldInput field={f} value={custom[f.key]} onChange={(v) => setCustom((c) => ({ ...c, [f.key]: v }))} people={people} />
              </Field>
            ))}
          </div>
        )}
      </form>
    </Dialog>
  );
}
