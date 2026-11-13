// Search. "Find work" matches work items and service desk requests on any combination of
// fields, including custom ones; the results are a sortable list and the item you pick opens
// beside it, so the list stays where it is. "Search text" looks for words in work and pages.
import { useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { ArrowDown, ArrowUp, Bell, BellOff, ExternalLink, FileText, ListFilter, Plus, Save, Search as SearchIcon, SquareCheck, Trash2, Users, X } from "lucide-react";
import { api, useGet, qs, type Item, type Person } from "../api";
import { useSession } from "../session";
import { Avatar, Dialog, Empty, ErrorNote, Field, Loaded, Loading, Pick, Priority, StatusBadge, TypeIcon, toast, useAction } from "../ui";
import { day, relative } from "../lib/format";
import { useLiveRefresh } from "../realtime";
import { DueDate, typeOf, usePeople } from "../work";
import { ItemPane } from "./Item";
import {
  EMPTY_QUERY, OP_LABEL, RELATIVE, SORT_LABEL, complete, controlFor, decodeQuery, defaultValue, describe, encodeQuery, forServer, fromSaved, newCriterion,
  type Criterion, type FieldInfo, type ItemQuery, type Option,
} from "../lib/criteria";

interface Hit {
  kind: string;
  id: string;
  label: string;
  title: string;
  url: string;
  snippet_parts: { text: string; hit: boolean }[];
}
interface FieldsInfo {
  fields: FieldInfo[];
  values: Record<string, (Option & { category?: string })[] | string[]>;
  fixed: Record<string, Option[]>;
  sorts: string[];
}
interface Saved {
  id: string;
  name: string;
  query: any;
  shared: boolean;
  alert: boolean;
  mine: boolean;
  owner: Person;
}

// ───────────────────────── words ─────────────────────────

function TextSearch() {
  const { me } = useSession();
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const kind = params.get("kind") ?? "";
  const mode = params.get("mode") ?? "";
  const [text, setText] = useState(q);
  useEffect(() => setText(q), [q]);
  const results = useGet<{ results: Hit[] }>(q.trim() ? `/api/v1/search${qs({ q, kind, mode, limit: 40 })}` : null);
  const set = (k: string, v: string) => {
    const next = new URLSearchParams(params);
    if (v) next.set(k, v);
    else next.delete(k);
    setParams(next);
  };
  return (
    <>
      <form className="filters" onSubmit={(e) => (e.preventDefault(), set("q", text.trim()))} role="search">
        <input className="input search" style={{ maxWidth: "none" }} autoFocus placeholder="Words, a phrase in quotes, or an item key like OPS-12" value={text} onChange={(e) => setText(e.target.value)} aria-label="Search" />
        <select className="select" value={kind} onChange={(e) => set("kind", e.target.value)} aria-label="Kind">
          <option value="">Everything</option>
          <option value="item">Work items</option>
          <option value="page">Pages</option>
        </select>
        {me.features.semantic_search && (
          <select className="select" value={mode} onChange={(e) => set("mode", e.target.value)} aria-label="Matching">
            <option value="">Best match</option>
            <option value="keyword">Exact words</option>
            <option value="semantic">Similar meaning</option>
          </select>
        )}
        <button className="btn primary">
          <SearchIcon /> Search
        </button>
      </form>
      {!q.trim() ? (
        <Empty icon={<SearchIcon />} title="Search across projects and docs">
          <p>Results only ever include things you are allowed to see.</p>
        </Empty>
      ) : (
        <Loaded q={results} what="Searching">
          {(d) =>
            d.results.length === 0 ? (
              <Empty title={`Nothing matches “${q}”`}>
                <p>Check the spelling, try fewer words, or search for part of a title.</p>
              </Empty>
            ) : (
              <div className="panel">
                {d.results.map((h) => (
                  <Link key={h.kind + h.id} to={h.url} className="list-row" style={{ alignItems: "flex-start" }}>
                    {h.kind === "page" ? <FileText width={16} height={16} style={{ marginTop: 3 }} /> : <SquareCheck width={16} height={16} style={{ marginTop: 3 }} />}
                    <span className="grow stack tight">
                      <span className="row">
                        <span className="key">{h.label}</span>
                        <strong className="truncate">{h.title}</strong>
                      </span>
                      {h.snippet_parts.length > 0 && <span className="small muted">{h.snippet_parts.map((p, i) => (p.hit ? <mark key={i}>{p.text}</mark> : <span key={i}>{p.text}</span>))}</span>}
                    </span>
                  </Link>
                ))}
              </div>
            )
          }
        </Loaded>
      )}
    </>
  );
}

// ───────────────────────── criteria ─────────────────────────

function DateValue({ value, onChange, label }: { value: unknown; onChange: (v: unknown) => void; label: string }) {
  const relativeDays = typeof value === "object" && value !== null ? (value as any).days : null;
  return (
    <span className="row" style={{ gap: 6 }}>
      <select className="select" style={{ maxWidth: 150 }} value={relativeDays === null ? "date" : String(relativeDays)} aria-label={`${label}: a date, or relative to today`} onChange={(e) => onChange(e.target.value === "date" ? "" : { days: Number(e.target.value) })}>
        <option value="date">a date</option>
        {RELATIVE.map((r) => (
          <option key={r.days} value={r.days}>{r.label}</option>
        ))}
      </select>
      {relativeDays === null && <input className="input" type="date" style={{ maxWidth: 160 }} value={typeof value === "string" ? value : ""} aria-label={label} onChange={(e) => onChange(e.target.value)} />}
    </span>
  );
}

function useOptions(info: FieldsInfo | undefined, people: Person[]) {
  return useMemo(() => {
    const users: Option[] = [{ value: "me", label: "Me" }, { value: "none", label: "Nobody" }, ...people.map((p) => ({ value: p.id, label: `${p.name}${p.kind === "agent" ? " (AI agent)" : ""}` }))];
    const of = (f: FieldInfo): Option[] => {
      if (f.kind === "user") return f.custom ? users.filter((u) => u.value !== "none") : users;
      if (f.options?.length) return f.options.map((o) => ({ value: o, label: o }));
      const fixed = info?.fixed[f.key];
      if (fixed) return fixed;
      const list = info?.values[f.key] ?? [];
      return (list as any[]).map((v) => (typeof v === "string" ? { value: v, label: v } : { value: v.value, label: v.label }));
    };
    const label = (field: string, v: unknown) => {
      const f = info?.fields.find((x) => x.key === field);
      return (f ? of(f).find((o) => o.value === v)?.label : undefined) ?? String(v);
    };
    return { of, label };
  }, [info, people]);
}

function Line({ c, info, options, onChange, onRemove }: { c: Criterion; info: FieldsInfo; options: ReturnType<typeof useOptions>; onChange: (c: Criterion) => void; onRemove: () => void }) {
  const f = info.fields.find((x) => x.key === c.field);
  const control = controlFor(f, c.op);
  const name = f?.label ?? c.field;
  return (
    <li className="criterion">
      <select
        className="select"
        value={c.field}
        aria-label="Field"
        onChange={(e) => {
          const nf = info.fields.find((x) => x.key === e.target.value)!;
          onChange(newCriterion(nf));
        }}
      >
        <optgroup label="Fields">
          {info.fields.filter((x) => !x.custom).map((x) => (
            <option key={x.key} value={x.key}>{x.label}</option>
          ))}
        </optgroup>
        {info.fields.some((x) => x.custom) && (
          <optgroup label="Custom fields">
            {info.fields.filter((x) => x.custom).map((x) => (
              <option key={x.key} value={x.key}>{x.label}</option>
            ))}
          </optgroup>
        )}
        {!f && <option value={c.field}>{c.field} (no longer exists)</option>}
      </select>
      {f && f.ops.length > 1 && (
        <select className="select" value={c.op} aria-label={`How ${name} is compared`} onChange={(e) => onChange({ field: c.field, op: e.target.value, value: controlFor(f, e.target.value) === control ? c.value : defaultValue(f, e.target.value) })}>
          {f.ops.map((op) => (
            <option key={op} value={op}>{OP_LABEL[op] ?? op}</option>
          ))}
        </select>
      )}
      <span className="criterion-value">
        {control === "text" && <input className="input" value={typeof c.value === "string" ? c.value : ""} maxLength={500} aria-label={`${name}: text`} placeholder={f?.kind === "parent" ? "Key of the parent, like OPS-12" : ""} onChange={(e) => onChange({ ...c, value: e.target.value })} />}
        {control === "number" && <input className="input" type="number" style={{ maxWidth: 140 }} value={typeof c.value === "number" ? c.value : ""} aria-label={`${name}: number`} onChange={(e) => onChange({ ...c, value: e.target.value === "" ? null : Number(e.target.value) })} />}
        {control === "priority" && (
          <select className="select" style={{ maxWidth: 160 }} value={String(c.value ?? 3)} aria-label={`${name}: priority`} onChange={(e) => onChange({ ...c, value: Number(e.target.value) })}>
            {(info.fixed.priority ?? []).slice().reverse().map((o) => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        )}
        {(control === "choice" || control === "people") && f && <Pick options={options.of(f)} value={Array.isArray(c.value) ? (c.value as (string | number)[]) : []} label={name} onChange={(value) => onChange({ ...c, value })} />}
        {control === "date" && <DateValue value={c.value} label={name} onChange={(value) => onChange({ ...c, value })} />}
        {control === "dates" && (
          <span className="row wrap" style={{ gap: 6 }}>
            <DateValue value={(c.value as any)?.[0]} label={`${name}: from`} onChange={(v) => onChange({ ...c, value: [v, (c.value as any)?.[1] ?? ""] })} />
            <span className="muted">and</span>
            <DateValue value={(c.value as any)?.[1]} label={`${name}: to`} onChange={(v) => onChange({ ...c, value: [(c.value as any)?.[0] ?? "", v] })} />
          </span>
        )}
        {control === "flag" && (
          <select className="select" style={{ maxWidth: 110 }} value={c.value === false ? "no" : "yes"} aria-label={name} onChange={(e) => onChange({ ...c, value: e.target.value === "yes" })}>
            <option value="yes">Yes</option>
            <option value="no">No</option>
          </select>
        )}
        {control === "none" && f?.kind === "watcher" && <span className="muted small">by me</span>}
      </span>
      <button type="button" className="btn ghost icon sm" aria-label={`Remove the line about ${name}`} onClick={onRemove}>
        <Trash2 />
      </button>
    </li>
  );
}

function SaveDialog({ query, existing, onClose }: { query: object; existing?: Saved; onClose: (id?: string) => void }) {
  const { can, me } = useSession();
  const [name, setName] = useState(existing?.name ?? "");
  const [shared, setShared] = useState(existing?.shared ?? false);
  const [alert, setAlert] = useState(existing?.alert ?? false);
  const [busy, run] = useAction();
  return (
    <Dialog
      title={existing ? "Update the saved search" : "Save this search"}
      onClose={() => onClose()}
      footer={
        <>
          <button className="btn" onClick={() => onClose()}>Cancel</button>
          <button
            className="btn primary"
            disabled={busy || !name.trim()}
            onClick={() => run(() => (existing ? api("PATCH", `/api/v1/searches/${existing.id}`, { name: name.trim(), query, shared, alert }).then(() => ({ id: existing.id })) : api("POST", "/api/v1/searches", { name: name.trim(), query, shared, alert })), (r: any) => (toast("Search saved"), onClose(r.id)))}
          >
            Save
          </button>
        </>
      }
    >
      <div className="stack">
        <Field label="Name" required>
          <input className="input" autoFocus value={name} maxLength={120} onChange={(e) => setName(e.target.value)} />
        </Field>
        {can("workspace.access") && (
          <label className="check">
            <input type="checkbox" checked={shared} onChange={(e) => setShared(e.target.checked)} /> Share with the workspace
          </label>
        )}
        <p className="small muted" style={{ marginTop: -6 }}>Everyone who opens a shared search sees only the work they themselves have access to.</p>
        <label className="check">
          <input type="checkbox" checked={alert} onChange={(e) => setAlert(e.target.checked)} /> Tell me when new work matches
        </label>
        <p className="small muted" style={{ marginTop: -6 }}>
          You get a notification when a work item starts to match{me.features.email ? ", by email as well if you chose that for saved search alerts in your profile" : ""}.
        </p>
      </div>
    </Dialog>
  );
}

const COLUMNS: { key: string; label: string; sort?: string; cls?: string }[] = [
  { key: "key", label: "Key", sort: "key" },
  { key: "title", label: "Name", sort: "title" },
  { key: "status", label: "Status", sort: "status" },
  { key: "priority", label: "Priority", sort: "priority", cls: "hide-sm" },
  { key: "assignee", label: "Assignee", sort: "assignee" },
  { key: "due", label: "Due", sort: "due", cls: "hide-sm" },
  { key: "updated", label: "Updated", sort: "updated", cls: "hide-sm" },
];

function FindWork() {
  const [params, setParams] = useSearchParams();
  const people = usePeople();
  const info = useGet<FieldsInfo>("/api/v1/items/query/fields", { staleTime: 60_000 });
  const saved = useGet<Saved[]>("/api/v1/searches");
  const options = useOptions(info.data, people);
  const savedId = params.get("saved");
  const open = params.get("open");
  const current = saved.data?.find((s) => s.id === savedId);
  const applied = useMemo(() => decodeQuery(params.get("find")), [params]);
  const [draft, setDraft] = useState<ItemQuery>(applied ?? EMPTY_QUERY);
  const [page, setPage] = useState(0);
  const [saving, setSaving] = useState<"new" | "update" | null>(null);
  const [busy, run] = useAction();
  const limit = 50;
  useLiveRefresh(["work"]);

  const go = (q: ItemQuery, more: Record<string, string | null> = {}) => {
    const next = new URLSearchParams(params);
    next.set("find", encodeQuery(q));
    for (const [k, v] of Object.entries(more)) v === null ? next.delete(k) : next.set(k, v);
    setParams(next);
    setPage(0);
  };
  // Opening a saved search runs it.
  useEffect(() => {
    if (current && !params.get("find")) {
      const q = fromSaved(current.query);
      setDraft(q);
      go(q);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current?.id]);
  // Going back or forward shows the search that produced that list.
  useEffect(() => {
    if (applied) setDraft(applied);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params.get("find")]);

  const fields = info.data?.fields ?? [];
  const sent = useMemo(() => (applied && info.data ? forServer(applied, fields) : null), [applied, info.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const results = useGet<{ items: Item[]; total: number }>(sent ? `/api/v1/items/query?q=${encodeURIComponent(JSON.stringify({ ...sent, limit, offset: page * limit }))}` : null, { placeholderData: (p: any) => p });
  const unfinished = draft.criteria.filter((c) => !complete(c, fields.find((f) => f.key === c.field))).length;
  const changed = !applied || encodeQuery(draft) !== encodeQuery(applied);
  const setOpen = (key: string | null) => {
    const next = new URLSearchParams(params);
    key ? next.set("open", key) : next.delete("open");
    setParams(next, { replace: !!open && !!key });
  };
  const sortBy = (field: string) => {
    const now = (applied ?? draft).sort[0];
    const q = { ...(applied ?? draft), sort: [{ field, dir: now?.field === field && now.dir === "asc" ? ("desc" as const) : ("asc" as const) }] };
    setDraft(q);
    go(q);
  };
  const list = results.data?.items ?? [];
  // Arrow keys walk the list while an item is open beside it.
  const step = (by: number) => {
    const i = list.findIndex((x) => x.key === open);
    const next = list[i + by];
    if (next) setOpen(next.key);
  };

  return (
    <div className={`find ${open ? "has-open" : ""}`}>
      <div className="find-main">
        <Loaded q={info} what="Loading the fields you can search on">
          {(d) => (
            <form className="panel find-form" onSubmit={(e) => (e.preventDefault(), go(draft))}>
              <header>
                <h2>
                  <ListFilter /> Work that matches
                </h2>
                {draft.criteria.length > 1 && (
                  <select className="select" style={{ maxWidth: 220 }} value={draft.match} aria-label="How the lines combine" onChange={(e) => setDraft({ ...draft, match: e.target.value as "all" | "any" })}>
                    <option value="all">every line below</option>
                    <option value="any">at least one line below</option>
                  </select>
                )}
                <span className="grow" />
                {(saved.data?.length ?? 0) > 0 && (
                  <select className="select" style={{ maxWidth: 240 }} value={savedId ?? ""} aria-label="Saved searches" onChange={(e) => setParams(e.target.value ? { view: "find", saved: e.target.value } : { view: "find" })}>
                    <option value="">Saved searches…</option>
                    {saved.data!.map((s) => (
                      <option key={s.id} value={s.id}>
                        {s.name}
                        {s.mine ? "" : ` (${s.owner.name})`}
                      </option>
                    ))}
                  </select>
                )}
              </header>
              <ul className="criteria">
                {draft.criteria.map((c, i) => (
                  <Line key={i} c={c} info={d} options={options} onChange={(nc) => setDraft({ ...draft, criteria: draft.criteria.map((x, n) => (n === i ? nc : x)) })} onRemove={() => setDraft({ ...draft, criteria: draft.criteria.filter((_, n) => n !== i) })} />
                ))}
                {draft.criteria.length === 0 && <li className="muted small" style={{ padding: "4px 0" }}>No conditions: every work item you can see matches. Add a line to narrow it down.</li>}
              </ul>
              <div className="body row wrap find-actions">
                <select className="select" style={{ maxWidth: 230 }} value="" aria-label="Add a condition on a field" onChange={(e) => e.target.value && setDraft({ ...draft, criteria: [...draft.criteria, newCriterion(d.fields.find((f) => f.key === e.target.value)!)] })}>
                  <option value="">Add a condition…</option>
                  <optgroup label="Fields">
                    {d.fields.filter((f) => !f.custom).map((f) => (
                      <option key={f.key} value={f.key}>{f.label}</option>
                    ))}
                  </optgroup>
                  {d.fields.some((f) => f.custom) && (
                    <optgroup label="Custom fields">
                      {d.fields.filter((f) => f.custom).map((f) => (
                        <option key={f.key} value={f.key}>{f.label}</option>
                      ))}
                    </optgroup>
                  )}
                </select>
                <span className="row" style={{ gap: 6 }}>
                  <span className="small muted nowrap">Sort by</span>
                  <select className="select" style={{ maxWidth: 150 }} value={draft.sort[0]?.field ?? "updated"} aria-label="Sort by" onChange={(e) => setDraft({ ...draft, sort: [{ field: e.target.value, dir: draft.sort[0]?.dir ?? "asc" }] })}>
                    {d.sorts.map((s) => (
                      <option key={s} value={s}>{SORT_LABEL[s] ?? s}</option>
                    ))}
                  </select>
                  <button type="button" className="btn icon sm" aria-label={draft.sort[0]?.dir === "desc" ? "Sorted last to first; switch to first to last" : "Sorted first to last; switch to last to first"} title={draft.sort[0]?.dir === "desc" ? "Last to first" : "First to last"} onClick={() => setDraft({ ...draft, sort: [{ field: draft.sort[0]?.field ?? "updated", dir: draft.sort[0]?.dir === "desc" ? "asc" : "desc" }] })}>
                    {draft.sort[0]?.dir === "desc" ? <ArrowDown /> : <ArrowUp />}
                  </button>
                </span>
                <span className="grow" />
                {unfinished > 0 && <span className="small warn-text">{unfinished === 1 ? "One line has no value yet and is left out." : `${unfinished} lines have no value yet and are left out.`}</span>}
                {current?.mine && (
                  <>
                    <button type="button" className="btn sm" disabled={busy} title={current.alert ? "Stop telling me about new matches" : "Tell me when new work matches"} onClick={() => run(() => api("PATCH", `/api/v1/searches/${current.id}`, { alert: !current.alert }), current.alert ? "Alerts off" : "You will be told about new matches")}>
                      {current.alert ? <Bell /> : <BellOff />} {current.alert ? "Alerting" : "Alert me"}
                    </button>
                    <button type="button" className="btn sm" onClick={() => setSaving("update")}>
                      <Save /> Update “{current.name}”
                    </button>
                  </>
                )}
                <button type="button" className="btn sm" onClick={() => setSaving("new")}>
                  <Plus /> Save as…
                </button>
                <button className={`btn ${changed ? "primary" : ""}`}>
                  <SearchIcon /> Search
                </button>
              </div>
              {current && (
                <div className="body row wrap small muted" style={{ borderTop: "1px solid var(--line)" }}>
                  <span>
                    Saved search <strong>{current.name}</strong>
                    {current.mine ? "" : ` by ${current.owner.name}`}
                  </span>
                  {current.shared && <span className="badge outline"><Users /> Shared</span>}
                  <span className="grow" />
                  {current.mine && (
                    <button type="button" className="btn ghost sm danger" disabled={busy} onClick={() => run(() => api("DELETE", `/api/v1/searches/${current.id}`), "Saved search deleted").then(() => setParams({ view: "find", find: params.get("find") ?? "" }))}>
                      <Trash2 /> Delete
                    </button>
                  )}
                </div>
              )}
            </form>
          )}
        </Loaded>
        {!applied ? (
          <Empty icon={<ListFilter />} title="Find work by what you know about it">
            <p>Name, assignee, reporter, dates, priority, cycle, label, status, or any field your organization added. Combine as many as you need, then save the search to run it again or to be told about new matches.</p>
          </Empty>
        ) : results.error ? (
          <ErrorNote error={results.error} />
        ) : !results.data ? (
          <Loading what="Searching" lines={6} />
        ) : results.data.total === 0 ? (
          <Empty title="Nothing matches">
            <p>Remove a line, or switch from “every line” to “at least one”. You only ever see work you have access to.</p>
          </Empty>
        ) : (
          <section className="panel">
            <header>
              <h2 role="status">
                {results.data.total.toLocaleString()} {results.data.total === 1 ? "match" : "matches"}
              </h2>
              <span className="small muted">
                {sent?.criteria.map((c) => describe(c, fields.find((f) => f.key === c.field), options.label, (iso) => day(iso))).join(sent.match === "any" ? " or " : " · ") || "All work you can see"}
              </span>
            </header>
            <div className="table-wrap">
              <table className="table find-results">
                <thead>
                  <tr>
                    {COLUMNS.filter((c) => !open || ["key", "title", "status"].includes(c.key)).map((c) => {
                      const s = applied.sort[0];
                      const on = s?.field === c.sort;
                      return (
                        <th key={c.key} scope="col" className={c.cls} aria-sort={on ? (s.dir === "desc" ? "descending" : "ascending") : undefined}>
                          <button type="button" className="th-sort" onClick={() => sortBy(c.sort!)} title={`Sort by ${c.label.toLowerCase()}`}>
                            {c.label}
                            {on && (s.dir === "desc" ? <ArrowDown /> : <ArrowUp />)}
                          </button>
                        </th>
                      );
                    })}
                  </tr>
                </thead>
                <tbody>
                  {list.map((i) => (
                    <tr key={i.id} className={i.key === open ? "selected" : ""} aria-selected={i.key === open}>
                      <td className="nowrap">
                        <span className="row" style={{ gap: 6 }}>
                          <TypeIcon type={typeOf(undefined, i)} title />
                          <span className="key">{i.key}</span>
                        </span>
                      </td>
                      <td style={{ minWidth: 180 }}>
                        <button type="button" className="row-open" onClick={() => setOpen(i.key)} aria-label={`${i.title}: open beside the list`}>
                          {i.title}
                        </button>
                      </td>
                      <td className="nowrap">
                        <StatusBadge name={i.status_name ?? i.status} category={i.status_category} />
                      </td>
                      {!open && (
                        <>
                          <td className="hide-sm nowrap"><Priority value={i.priority} /></td>
                          <td className="nowrap">
                            <span className="row" style={{ gap: 6 }}>
                              <Avatar person={i.assignee} size="sm" />
                              <span className="truncate hide-sm" style={{ maxWidth: 140 }}>{i.assignee?.name ?? ""}</span>
                            </span>
                          </td>
                          <td className="hide-sm nowrap"><DueDate item={i} /></td>
                          <td className="hide-sm nowrap small muted">{relative(i.updated_at)}</td>
                        </>
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {results.data.total > limit && (
              <div className="body row">
                <button className="btn sm" disabled={page === 0} onClick={() => setPage(page - 1)}>Previous</button>
                <span className="small muted num" role="status">
                  {page * limit + 1} to {Math.min(results.data.total, (page + 1) * limit)} of {results.data.total.toLocaleString()}
                </span>
                <button className="btn sm" disabled={(page + 1) * limit >= results.data.total} onClick={() => setPage(page + 1)}>Next</button>
              </div>
            )}
          </section>
        )}
      </div>
      {open && (
        <aside className="find-pane" aria-label={`Work item ${open}`}>
          <div className="find-pane-bar">
            <button className="btn ghost icon sm" aria-label="Previous in the list" disabled={list.findIndex((x) => x.key === open) <= 0} onClick={() => step(-1)}><ArrowUp /></button>
            <button className="btn ghost icon sm" aria-label="Next in the list" disabled={list.findIndex((x) => x.key === open) === list.length - 1} onClick={() => step(1)}><ArrowDown /></button>
            <span className="grow" />
            <Link className="btn ghost sm" to={`/i/${open}`}>
              <ExternalLink /> Open the full page
            </Link>
            <button className="btn ghost icon sm" aria-label="Close and return to the list" onClick={() => setOpen(null)}><X /></button>
          </div>
          <div className="find-pane-body pane">
            <ItemPane key={open} itemKey={open} />
          </div>
        </aside>
      )}
      {saving && info.data && <SaveDialog query={forServer(draft, fields)} existing={saving === "update" ? current : undefined} onClose={(id) => (setSaving(null), id && setParams({ view: "find", saved: id, find: encodeQuery(draft) }))} />}
    </div>
  );
}

export default function SearchPage() {
  const { can } = useSession();
  const [params, setParams] = useSearchParams();
  const canFind = can("workspace.access");
  // A link with words goes to the text search; everything else opens "Find work".
  const view = !canFind ? "text" : (params.get("view") ?? (params.get("q") ? "text" : "find"));
  return (
    <div className={`page ${view === "find" ? "wide" : "narrow"}`}>
      <div className="page-head">
        <div>
          <h1>Search</h1>
          <p className="sub">{view === "find" ? "Work items and service desk requests, matched on any of their fields." : "Words in work items and pages you have access to."}</p>
        </div>
      </div>
      {canFind && (
        <div className="tabs" role="tablist" style={{ marginBottom: 16 }}>
          <button role="tab" aria-selected={view === "find"} onClick={() => setParams({ view: "find" })}>
            <ListFilter /> Find work
          </button>
          <button role="tab" aria-selected={view === "text"} onClick={() => setParams({ view: "text" })}>
            <SearchIcon /> Search text
          </button>
        </div>
      )}
      {view === "find" ? <FindWork /> : <TextSearch />}
    </div>
  );
}
