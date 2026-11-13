// The conversation on a work item or a service request, and the record of what changed.
// Used by the work item page and by the help desk portal.
import { lazy, Suspense, useMemo, useRef, useState } from "react";
import { EllipsisVertical, History as HistoryIcon, Lock, Pencil, Trash2 } from "lucide-react";
import { api, useGet, type Attachment, type Person, type Refs } from "../api";
import { AgentBadge, Avatar, Confirm, Loading, Markdown, Menu, PersonName, StatusBadge, useAction } from "../ui";
import { dayKey, day, reducedMotion, relative, stamp, timeZone, priorityLabel, titleCase } from "../lib/format";
import { usePresence, useRoom } from "../realtime";

const MarkdownEditor = lazy(() => import("../editors/MarkdownEditor"));
const DiffView = lazy(() => import("./DiffView"));

export interface Message {
  id: string;
  internal: boolean;
  created_at: string;
  edited_at: string | null;
  deleted: boolean;
  author: Person | null;
  body: string;
  refs: Refs;
  mine: boolean;
}

interface Props {
  messages: Message[];
  /** Address new messages are posted to. */
  postTo: string;
  canPost: boolean;
  /** May write notes the requester cannot see (service desk agents). */
  canInternal?: boolean;
  /** May remove other people's messages. */
  canModerate?: boolean;
  uploadTo?: string;
  attachments?: Attachment[];
  /** The live room of the thing discussed; shows who else is writing. */
  room?: string;
  people?: Person[];
  /** Service desk wording: a message to the requester is a reply. */
  desk?: boolean;
  placeholder?: string;
}

function OneMessage({ m, canModerate, attachments, uploadTo, people }: { m: Message; canModerate?: boolean; attachments?: Attachment[]; uploadTo?: string; people?: Person[] }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(m.body);
  const [removing, setRemoving] = useState(false);
  const [busy, run] = useAction();
  const edited = m.edited_at && !m.deleted;
  return (
    <article className={`comment ${m.internal ? "internal" : ""} ${m.deleted ? "removed" : ""}`} id={`message-${m.id}`} aria-label={`Message from ${m.author?.name ?? "a former account"}`}>
      <Avatar person={m.author} presence />
      <div className="grow stack tight" style={{ minWidth: 0 }}>
        <header className="row wrap small">
          <PersonName person={m.author} />
          {m.author?.job_title && <span className="muted hide-sm">{m.author.job_title}</span>}
          <time className="muted" dateTime={m.created_at} title={relative(m.created_at)}>
            {stamp(m.created_at)}
          </time>
          {edited && (
            <span className="muted" title={`Edited ${stamp(m.edited_at)}`}>
              (edited)
            </span>
          )}
          {m.internal && (
            <span className="badge amber">
              <Lock /> Internal note
            </span>
          )}
          <span className="grow" />
          {!m.deleted && (m.mine || canModerate) && !editing && (
            <Menu
              trigger={(open, toggle) => (
                <button className="btn ghost icon sm" aria-label="Message actions" aria-expanded={open} onClick={toggle}>
                  <EllipsisVertical />
                </button>
              )}
            >
              {(close) => (
                <>
                  {m.mine && (
                    <button onClick={() => (setText(m.body), setEditing(true), close())}>
                      <Pencil /> Edit
                    </button>
                  )}
                  <button onClick={() => (setRemoving(true), close())}>
                    <Trash2 /> Remove
                  </button>
                </>
              )}
            </Menu>
          )}
        </header>
        {m.deleted ? (
          <p className="muted small">This message was removed.</p>
        ) : editing ? (
          <div className="stack">
            <Suspense fallback={<Loading lines={2} what="Opening the editor" />}>
              <MarkdownEditor value={text} onChange={setText} label="Message" compact uploadTo={uploadTo} attachments={attachments} people={people} autoFocus onSubmit={() => void save()} />
            </Suspense>
            <div className="row">
              <button className="btn primary sm" disabled={busy || !text.trim()} onClick={() => void save()}>
                Save changes
              </button>
              <button className="btn sm" onClick={() => setEditing(false)}>
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <Markdown source={m.body} refs={m.refs} attachments={attachments} compact />
        )}
      </div>
      {removing && (
        <Confirm
          title="Remove this message?"
          action="Remove message"
          danger
          body={m.mine ? "Its text is erased. The others will see that a message was removed." : `You are removing a message written by ${m.author?.name ?? "someone else"}. Its text is erased, and the removal is recorded.`}
          onClose={() => setRemoving(false)}
          onConfirm={async () => {
            await run(() => api("DELETE", `/api/v1/comments/${m.id}`), "Message removed");
          }}
        />
      )}
    </article>
  );
  async function save() {
    const r = await run(() => api("PATCH", `/api/v1/comments/${m.id}`, { body: text }), "Message updated");
    if (r) setEditing(false);
  }
}

export function Discussion({ messages, postTo, canPost, canInternal, canModerate, uploadTo, attachments, room, people, desk, placeholder }: Props) {
  const [draft, setDraft] = useState("");
  const [internal, setInternal] = useState(false);
  const [busy, run] = useAction();
  const [editorKey, setEditorKey] = useState(0);
  const end = useRef<HTMLDivElement>(null);
  const { peers } = useRoom(room);
  usePresence(room, draft.trim() ? { writing: true } : null);
  const writing = peers.filter((p) => p.state.writing).map((p) => p.user.name);
  const groups = useMemo(() => {
    const out: { day: string; items: Message[] }[] = [];
    for (const m of messages) {
      const d = dayKey(m.created_at);
      if (out.length && out[out.length - 1].day === d) out[out.length - 1].items.push(m);
      else out.push({ day: d, items: [m] });
    }
    return out;
  }, [messages]);

  async function send() {
    if (!draft.trim() || busy) return;
    const r = await run(() => api("POST", postTo, { body: draft, ...(canInternal ? { internal } : {}) }), internal ? "Internal note added" : desk ? "Reply sent" : "Message posted");
    if (r) {
      setDraft("");
      setEditorKey((k) => k + 1);
      requestAnimationFrame(() => end.current?.scrollIntoView({ block: "nearest", behavior: reducedMotion() ? "auto" : "smooth" }));
    }
  }

  return (
    <div className="discussion">
      {messages.length === 0 && <p className="muted" style={{ padding: "14px 0" }}>{desk ? "No replies yet." : "Nothing has been said here yet."}</p>}
      {/* A log: what is already here is not read out, and a message that arrives is, without taking focus. */}
      <div role="log" aria-label={desk ? "Replies and notes" : "Messages"}>
        {groups.map((g) => (
          <section key={g.day} aria-label={day(g.day)}>
            <div className="day-sep" role="separator">
              <span>{new Date(`${g.day}T12:00:00`).toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" })}</span>
            </div>
            {g.items.map((m) => (
              <OneMessage key={m.id} m={m} canModerate={canModerate} attachments={attachments} uploadTo={uploadTo} people={people} />
            ))}
          </section>
        ))}
      </div>
      <div ref={end} />
      <p className="small muted writing" aria-live="polite">
        {writing.length === 1 ? `${writing[0]} is writing…` : writing.length > 1 ? `${writing.slice(0, 3).join(", ")} are writing…` : " "}
      </p>
      {canPost && (
        <form
          className="stack"
          onSubmit={(e) => {
            e.preventDefault();
            void send();
          }}
        >
          <Suspense fallback={<Loading lines={2} what="Opening the editor" />}>
            <MarkdownEditor
              key={editorKey}
              value={draft}
              onChange={setDraft}
              label={internal ? "Internal note" : desk ? "Reply" : "Message"}
              placeholder={placeholder ?? (desk && !internal ? "Reply to the requester…" : "Write a message. @ mentions someone, [[ links work or a page.")}
              compact
              storageKey="chat"
              uploadTo={uploadTo}
              attachments={attachments}
              people={people}
              onSubmit={() => void send()}
            />
          </Suspense>
          <div className="row wrap">
            <button className="btn primary" disabled={busy || !draft.trim()}>
              {internal ? "Add internal note" : desk ? "Send reply" : "Post message"}
            </button>
            {canInternal && (
              <label className="check">
                <input type="checkbox" checked={internal} onChange={(e) => setInternal(e.target.checked)} /> Internal note (the requester cannot see it)
              </label>
            )}
            <span className="grow" />
            <span className="small muted" title="Change the time zone in your profile">
              Times are shown in {timeZone().replace(/_/g, " ")}
            </span>
          </div>
        </form>
      )}
    </div>
  );
}

// ───────────────────────── history ─────────────────────────

export interface Change {
  id: number;
  field: string;
  field_name?: string | null;
  old: any;
  new: any;
  at: string;
  actor: Person | null;
}

const FIELD: Record<string, string> = { title: "the title", description: "the description", priority: "the priority", labels: "the labels", start_date: "the start date", due_date: "the due date", estimate: "the estimate", cycle: "the cycle", parent: "the parent", assignee: "the assignee", type: "the type" };

function value(field: string, v: any): React.ReactNode {
  if (v === null || v === undefined || v === "" || (Array.isArray(v) && v.length === 0)) return <em className="muted">nothing</em>;
  if (field === "priority") return priorityLabel(Number(v));
  if (field === "start_date" || field === "due_date") return day(String(v));
  if (field === "labels" && Array.isArray(v)) return v.join(", ");
  if (field === "assignee" && typeof v === "object") return <span className="person-name"><strong>{v.name}</strong>{v.kind === "agent" && <AgentBadge person={v} />}</span>;
  if ((field === "cycle" || field === "status") && typeof v === "object") return v.name ?? v.key;
  if (field === "parent" && typeof v === "object") return v.key;
  if (typeof v === "boolean") return v ? "yes" : "no";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

/** One entry of the record, as a sentence. */
export function describe(h: Change): React.ReactNode {
  const f = h.field;
  if (f === "created") return "created this";
  if (f === "archived") return h.new ? "archived this" : "restored this";
  if (f === "status")
    return (
      <>
        moved it from <StatusBadge name={h.old?.name ?? titleCase(String(h.old?.key ?? h.old))} category={h.old?.category ?? ""} /> to <StatusBadge name={h.new?.name ?? titleCase(String(h.new?.key ?? h.new))} category={h.new?.category ?? ""} />
        {h.old?.reason ? <span className="muted"> ({h.old.reason})</span> : null}
      </>
    );
  if (f === "comment") return h.new?.action === "edited" ? "edited a message" : h.new?.action === "removed" ? "removed a message" : "posted a message";
  if (f === "note") return "added an internal note";
  if (f === "attachment") return h.new ? <>attached <strong>{String(h.new)}</strong></> : <>deleted the file <strong>{String(h.old)}</strong></>;
  if (f === "link") return h.new ? <>linked this as <em>{String(h.new.rel).replace(/_/g, " ")}</em> {h.new.target ?? ""}</> : <>removed a link ({String(h.old?.rel ?? "").replace(/_/g, " ")} {h.old?.target ?? ""})</>;
  if (f === "sla") return <>missed the {String(h.new ?? "service-level")} target</>;
  if (f === "description") return "changed the description";
  const name = h.field_name ? h.field_name : (FIELD[f] ?? titleCase(f.replace(/^custom\./, "")));
  return (
    <>
      changed {h.field_name ? <strong>{name}</strong> : name} from {value(f, h.old)} to {value(f, h.new)}
    </>
  );
}

/** Everything that happened to a work item: who changed which field, from what to what, and when. */
export function ItemHistory({ itemKey, total }: { itemKey: string; total: number }) {
  const [limit, setLimit] = useState(50);
  const q = useGet<{ entries: Change[]; total: number }>(`/api/v1/items/${itemKey}/history?limit=${limit}`);
  const [diff, setDiff] = useState<number | null>(null);
  if (!q.data) return <Loading lines={5} what="Loading the history" />;
  const entries = q.data.entries;
  return (
    <div className="history" style={{ paddingTop: 10 }}>
      <p className="small muted row">
        <HistoryIcon width={14} height={14} aria-hidden="true" />
        {q.data.total} {q.data.total === 1 ? "entry" : "entries"}, newest first. Times are shown in {timeZone().replace(/_/g, " ")}.
      </p>
      <ol className="history-list">
        {entries.map((h) => (
          <li key={h.id} className="timeline-row">
            <time className="nowrap small" dateTime={h.at} title={relative(h.at)}>
              {stamp(h.at)}
            </time>
            <span className="grow">
              <span className="row wrap" style={{ gap: 6 }}>
                <Avatar person={h.actor} size="sm" />
                <span>
                  <PersonName person={h.actor} fallback="The system" /> {describe(h)}
                  {h.field === "description" && (
                    <button className="btn ghost sm" style={{ marginLeft: 6 }} onClick={() => setDiff(diff === h.id ? null : h.id)} aria-expanded={diff === h.id}>
                      {diff === h.id ? "Hide what changed" : "Show what changed"}
                    </button>
                  )}
                </span>
              </span>
              {diff === h.id && (
                <Suspense fallback={<Loading lines={2} />}>
                  <DiffView before={String(h.old ?? "")} after={String(h.new ?? "")} beforeLabel="Before" afterLabel="After" />
                </Suspense>
              )}
            </span>
          </li>
        ))}
      </ol>
      {entries.length < Math.max(total, q.data.total) && (
        <button className="btn sm" onClick={() => setLimit((l) => l + 100)} disabled={q.isFetching}>
          Show older entries
        </button>
      )}
    </div>
  );
}
