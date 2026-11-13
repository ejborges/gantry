// Shared building blocks. Pages compose these and never style from scratch.
import { cloneElement, isValidElement, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { Bot, Check, CircleAlert, CircleCheck, CircleDashed, CircleDot, Clock, Copy, LoaderCircle, Minus, ArrowDown, ArrowUp, ChevronsUp, X, Inbox } from "lucide-react";
import { ApiError, refresh, type Attachment, type FieldDef, type ItemType, type Person, type Refs } from "./api";
import { initials, priorityLabel, slaState } from "./lib/format";
import { enhance, render } from "./lib/markdown";
import { NamedIcon } from "./icons";

// ───────── toasts ─────────
type Toast = { id: number; text: string; kind: "info" | "error" };
let toasts: Toast[] = [];
const listeners = new Set<() => void>();
function emit() {
  listeners.forEach((l) => l());
}
export function toast(text: string, kind: Toast["kind"] = "info") {
  const t = { id: Date.now() + Math.random(), text, kind };
  toasts = [...toasts, t];
  emit();
  setTimeout(() => {
    toasts = toasts.filter((x) => x.id !== t.id);
    emit();
  }, kind === "error" ? 8000 : 3500);
}
export function Toaster() {
  const list = useSyncExternalStore((cb) => (listeners.add(cb), () => listeners.delete(cb)), () => toasts);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {list.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`}>
          {t.text}
        </div>
      ))}
    </div>
  );
}

/** Run a change: disables the trigger while busy, reports failure, refreshes what is on screen. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  async function run<T>(fn: () => Promise<T>, done?: string | ((r: T) => void)): Promise<T | undefined> {
    setBusy(true);
    try {
      const r = await fn();
      await refresh();
      if (typeof done === "string") toast(done);
      else done?.(r);
      return r;
    } catch (e) {
      toast(e instanceof ApiError ? e.message : "Something went wrong. Check your connection and try again.", "error");
      return undefined;
    } finally {
      setBusy(false);
    }
  }
  return [busy, run] as const;
}

// ───────── the browser's title ─────────
let siteName = "Gantry";
/** The site's name, which follows the page's own heading in the browser's title. */
export function setSiteName(name: string | undefined) {
  siteName = name?.trim() || "Gantry";
  window.dispatchEvent(new Event("gantry:title"));
}
/**
 * Names the browser tab after the page's main heading: "Planning · Gantry". Done in one
 * place, so that no page forgets and none keeps the title of the page before it. A heading
 * may carry a fuller name for the tab in `data-title`.
 */
export function PageTitle() {
  useEffect(() => {
    const apply = () => {
      const h = document.querySelector<HTMLElement>("main h1") ?? document.querySelector<HTMLElement>("h1");
      // A heading may be made of parts (a name and a key, say): read them as separate words.
      const parts: string[] = [];
      if (h && h.dataset.title === undefined) for (let w = document.createTreeWalker(h, NodeFilter.SHOW_TEXT), n = w.nextNode(); n; n = w.nextNode()) parts.push(n.textContent ?? "");
      const text = (h?.dataset.title ?? parts.join(" ")).trim().replace(/\s+/g, " ");
      const next = text && text !== siteName ? `${text} · ${siteName}` : siteName;
      if (document.title !== next) document.title = next;
    };
    apply();
    const seen = new MutationObserver(apply);
    seen.observe(document.body, { childList: true, subtree: true, characterData: true });
    window.addEventListener("gantry:title", apply);
    return () => {
      seen.disconnect();
      window.removeEventListener("gantry:title", apply);
    };
  }, []);
  return null;
}

// ───────── overlays ─────────
const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"]), [contenteditable="true"]';

export function Dialog({ title, onClose, children, footer, wide }: { title: string; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean | "canvas" }) {
  const ref = useRef<HTMLDivElement>(null);
  const id = useId();
  // What had focus when the dialog was asked for. Read while rendering, before any field
  // inside (one marked autoFocus, say) takes focus, so that closing can give it back.
  const [opener] = useState(() => (typeof document === "undefined" ? null : (document.activeElement as HTMLElement | null)));
  useEffect(() => {
    const prev = opener;
    // A field that asked for focus keeps it; otherwise the first field, or the main button.
    if (!ref.current?.contains(document.activeElement)) ref.current?.querySelector<HTMLElement>("input:not([type=hidden]), select, textarea, button.primary")?.focus();
    const onKey = (e: KeyboardEvent) => {
      const box = ref.current;
      // With one dialog on top of another, keys belong to the one on top.
      const open = document.querySelectorAll(".backdrop > .dialog");
      if (!box || open[open.length - 1] !== box) return;
      if (e.key === "Escape") return onClose();
      if (e.key !== "Tab") return;
      // A dialog is modal: Tab moves among its own controls and does not wander into the page behind.
      const stops = Array.from(box.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => el.getClientRects().length > 0);
      if (stops.length === 0) return e.preventDefault();
      const at = document.activeElement as HTMLElement | null;
      const [head, tail] = [stops[0], stops[stops.length - 1]];
      if (!at || !box.contains(at)) (e.preventDefault(), head.focus());
      else if (e.shiftKey && at === head) (e.preventDefault(), tail.focus());
      else if (!e.shiftKey && at === tail) (e.preventDefault(), head.focus());
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      if (prev?.isConnected) prev.focus?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`dialog ${wide === "canvas" ? "wide canvas" : wide ? "wide" : ""}`} role="dialog" aria-modal="true" aria-labelledby={id} ref={ref}>
        <header>
          <h2 id={id}>{title}</h2>
          <button className="btn ghost icon sm" onClick={onClose} aria-label="Close">
            <X />
          </button>
        </header>
        <div className="body">{children}</div>
        {footer && <footer>{footer}</footer>}
      </div>
    </div>
  );
}

export function Menu({ trigger, children, align = "right" }: { trigger: (open: boolean, toggle: () => void) => ReactNode; children: (close: () => void) => ReactNode; align?: "left" | "right" }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const box = menu.current;
    // A menu as assistive technology expects it: its choices are menu items, focus starts on
    // the first, the arrow keys move between them, and Escape returns to the button.
    const items = () => Array.from(box?.querySelectorAll<HTMLElement>("button:not([disabled]), a[href]") ?? []);
    for (const el of items()) if (!el.getAttribute("role")) el.setAttribute("role", "menuitem");
    items()[0]?.focus();
    const keys = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        ref.current?.querySelector<HTMLElement>("button, a[href]")?.focus();
        return;
      }
      const list = items();
      const at = list.indexOf(document.activeElement as HTMLElement);
      const to = e.key === "ArrowDown" ? (at + 1) % list.length : e.key === "ArrowUp" ? (at - 1 + list.length) % list.length : e.key === "Home" ? 0 : e.key === "End" ? list.length - 1 : -1;
      if (to < 0 || list.length === 0) return;
      e.preventDefault();
      list[to].focus();
    };
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", keys);
    return () => {
      document.removeEventListener("mousedown", away);
      document.removeEventListener("keydown", keys);
    };
  }, [open]);
  return (
    <span className="menu-anchor" ref={ref}>
      {trigger(open, () => setOpen((o) => !o))}
      {open && (
        <div ref={menu} className={`menu ${align === "left" ? "left" : ""}`} role="menu">
          {children(() => setOpen(false))}
        </div>
      )}
    </span>
  );
}

export function Confirm({ title, body, action, danger, onConfirm, onClose }: { title: string; body: ReactNode; action: string; danger?: boolean; onConfirm: () => void | Promise<void>; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      title={title}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className={`btn ${danger ? "danger" : "primary"}`}
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              await onConfirm();
              onClose();
            }}
          >
            {action}
          </button>
        </>
      }
    >
      <div>{body}</div>
    </Dialog>
  );
}

// ───────── states ─────────
/** How long a wait stays a quiet skeleton before it is explained. */
export const SLOW_AFTER_MS = 10_000;

/**
 * A wait. First the outline of what is coming; after about ten seconds a moving bar and a
 * sentence that says what is being waited for and what, if anything, to do about it.
 */
export function Loading({ lines = 4, what = "Loading" }: { lines?: number; what?: string }) {
  const [waited, setWaited] = useState(0);
  useEffect(() => {
    const started = Date.now();
    const t = setInterval(() => setWaited(Date.now() - started), 1000);
    return () => clearInterval(t);
  }, []);
  if (waited < SLOW_AFTER_MS) {
    return (
      <div className="stack" aria-busy="true" aria-label={what}>
        {Array.from({ length: lines }, (_, i) => (
          <div key={i} className="skeleton" style={{ width: `${88 - i * 13}%` }} />
        ))}
      </div>
    );
  }
  const seconds = Math.round(waited / 1000);
  return (
    <div className="slow" role="status" aria-live="polite">
      <div className="progress" aria-hidden="true">
        <span />
      </div>
      <p className="slow-title">{what}…</p>
      <p className="muted small">
        {seconds < 25
          ? "This is taking longer than usual. The server is still working on it; there is no need to reload."
          : seconds < 60
            ? `Still waiting after ${seconds} seconds. Large projects and slow connections can take a while.`
            : "No answer after a minute. Check your connection. If other pages open normally, tell an administrator what you were opening."}
      </p>
      {seconds >= 60 && (
        <button className="btn sm" onClick={() => window.location.reload()}>
          Reload the page
        </button>
      )}
    </div>
  );
}
export function Spinner() {
  return <LoaderCircle className="spin" width={16} height={16} aria-label="Working" />;
}
export function ErrorNote({ error }: { error: unknown }) {
  return (
    <div className="notice danger" role="alert">
      <CircleAlert />
      <span>{error instanceof Error ? error.message : "This could not be loaded."}</span>
    </div>
  );
}
export function Empty({ icon, title, children }: { icon?: ReactNode; title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      {icon ?? <Inbox />}
      <h3>{title}</h3>
      {children && <div className="stack tight" style={{ alignItems: "center" }}>{children}</div>}
    </div>
  );
}
/** Renders children once data has loaded; shows a skeleton or the error otherwise. */
export function Loaded<T>({ q, children, lines, what }: { q: { data?: T; error: unknown; isPending: boolean }; children: (data: T) => ReactNode; lines?: number; what?: string }) {
  if (q.data !== undefined) return <>{children(q.data)}</>;
  if (q.error) return <ErrorNote error={q.error} />;
  return <Loading lines={lines} what={what} />;
}

// ───────── small pieces ─────────
/**
 * A labeled form control. A single input, select or textarea is tied to its label so that
 * clicking the label focuses it and screen readers announce it; anything else (a group of
 * checkboxes, a composite picker) is announced as a group named by the label.
 */
export function Field({ label, hint, required, children }: { label: string; hint?: ReactNode; required?: boolean; children: ReactNode }) {
  const id = useId();
  const child = isValidElement<Record<string, any>>(children) ? children : null;
  const native = !!child && typeof child.type === "string" && ["input", "select", "textarea"].includes(child.type);
  const controlId = native ? (child!.props.id ?? `${id}-control`) : undefined;
  const hintId = hint ? `${id}-hint` : undefined;
  const box = useRef<HTMLDivElement>(null);
  const [single, setSingle] = useState(false);
  // A field built from one of our own controls (a person picker, say) holds a single input
  // somewhere inside: that input takes the field's label, as a plain input would. A field
  // with several controls is a group that carries the label for all of them.
  useEffect(() => {
    if (native) return;
    const controls = box.current?.querySelectorAll<HTMLElement>("input:not([type=hidden]), select, textarea");
    const only = controls?.length === 1 ? controls[0] : null;
    const takes = !!only && (!only.getAttribute("aria-label") || only.getAttribute("aria-labelledby") === `${id}-label`) && !only.id;
    if (only && takes && only.getAttribute("aria-labelledby") !== `${id}-label`) {
      only.setAttribute("aria-labelledby", `${id}-label`);
      if (hintId) only.setAttribute("aria-describedby", hintId);
    }
    if (takes !== single) setSingle(takes);
  });
  const grouped = !native && !single;
  return (
    <div ref={box} className="field" role={grouped ? "group" : undefined} aria-labelledby={grouped ? `${id}-label` : undefined}>
      <label id={`${id}-label`} htmlFor={controlId}>
        {label}
        {required && <span className="req" aria-hidden="true"> *</span>}
      </label>
      {native ? cloneElement(child!, { id: controlId, "aria-describedby": child!.props["aria-describedby"] ?? hintId, "aria-required": required || undefined }) : children}
      {hint && <span className="hint" id={hintId}>{hint}</span>}
    </div>
  );
}

const CATEGORY_ICON = { todo: CircleDashed, in_progress: CircleDot, done: CircleCheck };
export function StatusBadge({ name, category }: { name: string; category: "todo" | "in_progress" | "done" | string }) {
  const Icon = CATEGORY_ICON[category as keyof typeof CATEGORY_ICON] ?? CircleDashed;
  return (
    <span className={`badge ${category}`}>
      <Icon aria-hidden="true" />
      {name}
    </span>
  );
}

const PRIO_ICON = [Minus, ArrowDown, Minus, ArrowUp, ChevronsUp];
export function Priority({ value, label = true }: { value: number; label?: boolean }) {
  const Icon = PRIO_ICON[value] ?? Minus;
  return (
    <span className={`prio p${value}`} title={`Priority: ${priorityLabel(value)}`}>
      <Icon aria-hidden="true" />
      {label ? priorityLabel(value) : <span className="sr-only">{priorityLabel(value)}</span>}
    </span>
  );
}

export function TypeIcon({ type, title }: { type?: Pick<ItemType, "icon" | "color" | "name"> | null; title?: boolean }) {
  return (
    <span className={`type-icon ${type?.color ?? ""}`} title={title === false ? undefined : type?.name}>
      <NamedIcon name={type?.icon ?? "circle"} />
    </span>
  );
}

export const PRESENCE: Record<string, string> = { available: "Available", away: "Away", busy: "Busy", out_of_office: "Out of office" };

/**
 * An account's picture: the one they uploaded, or the pattern generated for them. An AI agent
 * always carries the robot mark, whatever picture it has, so it cannot pass for a person.
 */
export function Avatar({ person, size = "md", presence }: { person: Person | null | undefined; size?: "sm" | "md" | "lg" | "xl"; presence?: boolean }) {
  const [broken, setBroken] = useState(false);
  if (!person) return <span className={`avatar ${size}`} title="Unassigned" role="img" aria-label="Unassigned" />;
  const agent = person.kind === "agent";
  const state = presence && person.presence && person.presence !== "available" ? person.presence : null;
  const label = `${person.name}${agent ? " (AI agent)" : ""}${state ? `, ${PRESENCE[state] ?? state}` : ""}`;
  return (
    <span className={`avatar ${size} ${agent ? "agent" : ""}`} title={label} role="img" aria-label={label}>
      {person.avatar && !broken ? <img src={person.avatar} alt="" loading="lazy" onError={() => setBroken(true)} /> : agent ? <Bot /> : initials(person.name)}
      {agent && (
        <span className="avatar-mark" aria-hidden="true">
          <Bot />
        </span>
      )}
      {state && <span className={`presence-dot ${state}`} aria-hidden="true" />}
    </span>
  );
}

/** The words "AI agent" next to a name, wherever an account is named. */
export function AgentBadge({ person }: { person: Person | null | undefined }) {
  if (person?.kind !== "agent") return null;
  return (
    <span className="badge agent" title="This account is an AI agent, not a person">
      <Bot aria-hidden="true" />
      AI agent
    </span>
  );
}

/** A name with what kind of account it is. */
export function PersonName({ person, fallback = "Former account" }: { person: Person | null | undefined; fallback?: string }) {
  return (
    <span className="person-name">
      <strong>{person?.name ?? fallback}</strong>
      <AgentBadge person={person} />
    </span>
  );
}

export function PersonChip({ person, fallback = "Unassigned", presence }: { person: Person | null | undefined; fallback?: string; presence?: boolean }) {
  return (
    <span className="row person-chip" style={{ gap: 7, minWidth: 0 }}>
      <Avatar person={person} presence={presence} />
      <span className={`truncate ${person ? "" : "muted"}`}>{person?.name ?? fallback}</span>
      <AgentBadge person={person} />
    </span>
  );
}

/** The people who have the same thing open right now, and where in it they are. */
export function Peers({ peers, where }: { peers: { conn: string; user: Person; state: Record<string, any> }[]; where?: (state: Record<string, any>) => string | null }) {
  if (peers.length === 0) return null;
  const names = peers.map((p) => p.user.name).join(", ");
  return (
    <span className="peers" role="status" aria-label={`Also here: ${names}`}>
      {peers.slice(0, 5).map((p) => {
        const at = where?.(p.state);
        return (
          <span key={p.conn} className="peer" title={`${p.user.name}${p.user.kind === "agent" ? " (AI agent)" : ""}${at ? `: ${at}` : " is viewing this"}`}>
            <Avatar person={p.user} size="sm" />
            {at && <span className="peer-at">{at}</span>}
          </span>
        );
      })}
      {peers.length > 5 && <span className="small muted">+{peers.length - 5}</span>}
    </span>
  );
}

export function SlaChip({ due, breached, metAt, what }: { due: string | null; breached: boolean; metAt: string | null; what: string }) {
  const s = slaState(due, breached, metAt);
  if (!s) return null;
  return (
    <span className={`sla ${s.state === "met" ? "ok" : s.state}`} title={`${what} target: ${due ? new Date(due).toLocaleString() : ""}`}>
      {s.state === "met" ? <Check /> : <Clock />}
      {s.label}
    </span>
  );
}

/**
 * Rendered Markdown. `refs` is what the server resolved for this reader (mentions, work item
 * keys, page links); `attachments` are the files the text may show by name.
 */
export function Markdown({ source, compact, empty, refs, attachments }: { source: string; compact?: boolean; empty?: string; refs?: Refs; attachments?: Attachment[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const html = useMemo(() => render(source, { refs, attachments }), [source, refs, attachments]);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // The markup is put in place here, not by React, because what `enhance` draws into it
    // (diagrams, formulas, frames) must not be undone by an unrelated re-render.
    el.innerHTML = html; // sanitized by DOMPurify in lib/markdown.ts; this is the only place it is inserted
    return enhance(el);
  }, [html]);
  if (!source.trim()) return empty ? <p className="muted">{empty}</p> : null;
  return <div ref={ref} className={`prose ${compact ? "compact" : ""}`} />;
}

export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      className="btn sm"
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setDone(true);
        setTimeout(() => setDone(false), 1500);
      }}
    >
      {done ? <Check /> : <Copy />}
      {done ? "Copied" : label}
    </button>
  );
}

export function PersonSelect({ people, value, onChange, none = "Unassigned", label }: { people: Person[]; value: string | null; onChange: (id: string | null) => void; none?: string; label?: string }) {
  return (
    <select className="select" value={value ?? ""} aria-label={label} onChange={(e) => onChange(e.target.value || null)}>
      <option value="">{none}</option>
      {people.map((p) => (
        <option key={p.id} value={p.id}>
          {p.name}
          {p.kind === "agent" ? " (AI agent)" : ""}
        </option>
      ))}
    </select>
  );
}

/** Input for one custom field, chosen by the field's type. */
export function CustomFieldInput({ field, value, onChange, people }: { field: FieldDef; value: any; onChange: (v: any) => void; people: Person[] }) {
  switch (field.type) {
    case "textarea":
      return <textarea className="textarea" value={value ?? ""} onChange={(e) => onChange(e.target.value || null)} />;
    case "number":
      return <input className="input" type="number" step="any" value={value ?? ""} onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))} />;
    case "date":
      return <input className="input" type="date" value={value ?? ""} onChange={(e) => onChange(e.target.value || null)} />;
    case "select":
      return (
        <select className="select" value={value ?? ""} onChange={(e) => onChange(e.target.value || null)}>
          <option value="">Not set</option>
          {field.options.map((o) => (
            <option key={o}>{o}</option>
          ))}
        </select>
      );
    case "multiselect":
      return (
        <div className="row wrap">
          {field.options.map((o) => {
            const on = Array.isArray(value) && value.includes(o);
            return (
              <label key={o} className="check">
                <input type="checkbox" checked={on} onChange={() => onChange(on ? value.filter((x: string) => x !== o) : [...(value ?? []), o])} />
                {o}
              </label>
            );
          })}
        </div>
      );
    case "user":
      return <PersonSelect people={people} value={value ?? null} onChange={onChange} none="Not set" />;
    case "checkbox":
      return (
        <label className="check">
          <input type="checkbox" checked={!!value} onChange={(e) => onChange(e.target.checked)} />
          Yes
        </label>
      );
    case "url":
      return <input className="input" type="url" placeholder="https://" value={value ?? ""} onChange={(e) => onChange(e.target.value || null)} />;
    default:
      return <input className="input" value={value ?? ""} onChange={(e) => onChange(e.target.value || null)} />;
  }
}

/** Read-only display of a custom field value. */
export function fieldText(field: FieldDef, value: any, people: Person[]): string {
  if (value === null || value === undefined || value === "") return "";
  if (field.type === "checkbox") return value ? "Yes" : "No";
  if (field.type === "user") return people.find((p) => p.id === value)?.name ?? "Unknown person";
  if (Array.isArray(value)) return value.join(", ");
  return String(value);
}

/** Several values picked from a list: chosen ones are chips, the rest are in the menu. */
export function Pick({ options, value, onChange, label }: { options: { value: string | number; label: string }[]; value: (string | number)[]; onChange: (v: (string | number)[]) => void; label: string }) {
  const left = options.filter((o) => !value.includes(o.value));
  return (
    <span className="pick" role="group" aria-label={label}>
      {value.map((v) => (
        <span key={String(v)} className="chip">
          {options.find((o) => o.value === v)?.label ?? String(v)}
          <button type="button" aria-label={`Remove ${options.find((o) => o.value === v)?.label ?? v}`} onClick={() => onChange(value.filter((x) => x !== v))}>
            <X />
          </button>
        </span>
      ))}
      {left.length > 0 && (
        <select className="select" value="" aria-label={`Add to ${label}`} onChange={(e) => e.target.value !== "" && onChange([...value, options.find((o) => String(o.value) === e.target.value)!.value])}>
          <option value="">{value.length ? "Add…" : "Choose…"}</option>
          {left.map((o) => (
            <option key={String(o.value)} value={String(o.value)}>{o.label}</option>
          ))}
        </select>
      )}
    </span>
  );
}
