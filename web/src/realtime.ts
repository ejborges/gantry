// One WebSocket per browser tab. It carries three things:
//   events    "this changed" hints; what changed is then fetched through the ordinary API
//   presence  who else has the same thing open, and which field or line they are on
//   documents Yjs updates for text that several people edit at the same time
// The server authorizes every room like reading the object, so nothing arrives here that
// the API would not also give.
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import * as Y from "yjs";
import { Awareness, applyAwarenessUpdate, encodeAwarenessUpdate, removeAwarenessStates } from "y-protocols/awareness";
import { queryClient, type Person } from "./api";

export interface Peer {
  conn: string;
  user: Person;
  state: Record<string, any>;
  seen: number;
}
type Listener = (msg: any) => void;
export type LiveStatus = "idle" | "connecting" | "open" | "offline";

export function toBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}
export function fromBase64(text: string): Uint8Array {
  const s = atob(text);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

class Room {
  refs = 0;
  listeners = new Set<Listener>();
  peers = new Map<string, Peer>();
  write = false;
  joined = false;
  denied = false;
  state: Record<string, any> | null = null;
  constructor(readonly name: string) {}
  /** Rooms about one object share presence; the catch-all rooms do not. */
  get shared() {
    return this.name.includes(":") && !this.name.startsWith("user:");
  }
}

const HEARTBEAT_MS = 30_000;
const PEER_EXPIRY_MS = 100_000;

class Live {
  private ws: WebSocket | null = null;
  private rooms = new Map<string, Room>();
  private retry = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private beat: ReturnType<typeof setInterval> | undefined;
  private watchers = new Set<() => void>();
  private version = 0;
  status: LiveStatus = "idle";
  conn = "";
  /** False when the server has the real-time connection switched off: the pages still work, by asking. */
  available = true;
  private stopped = false;

  subscribe = (cb: () => void) => {
    this.watchers.add(cb);
    return () => this.watchers.delete(cb);
  };
  snapshot = () => this.version;
  private changed() {
    this.version++;
    this.watchers.forEach((w) => w());
  }

  room(name: string): Room | undefined {
    return this.rooms.get(name);
  }

  private connect() {
    if (this.ws || this.stopped || !this.available || typeof WebSocket === "undefined") return;
    this.status = "connecting";
    this.changed();
    let ws: WebSocket;
    try {
      ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/v1/ws`);
    } catch {
      this.status = "offline";
      return;
    }
    this.ws = ws;
    let opened = false;
    ws.onmessage = (e) => {
      opened = true;
      let m: any;
      try {
        m = JSON.parse(String(e.data));
      } catch {
        return;
      }
      this.handle(m);
    };
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      this.conn = "";
      this.status = "offline";
      for (const r of this.rooms.values()) {
        r.joined = false;
        r.peers.clear();
        r.listeners.forEach((l) => l({ t: "offline", room: r.name }));
      }
      this.changed();
      // A connection that never opened most likely means real time is switched off (404) or
      // the session ended (401). Keep trying, slowly: the pages work without it.
      if (!opened) this.retry = Math.max(this.retry, 4);
      if (!this.stopped && this.rooms.size > 0) {
        const delay = Math.min(30_000, 1000 * 2 ** this.retry) * (0.7 + Math.random() * 0.6);
        this.retry = Math.min(this.retry + 1, 6);
        clearTimeout(this.timer);
        this.timer = setTimeout(() => this.connect(), delay);
      }
    };
    ws.onerror = () => undefined;
    if (!this.beat) {
      this.beat = setInterval(() => this.heartbeat(), HEARTBEAT_MS);
    }
  }

  private heartbeat() {
    const now = Date.now();
    let dropped = false;
    for (const r of this.rooms.values()) {
      if (r.joined && r.shared) this.send({ t: "presence", room: r.name, state: r.state ?? {} });
      for (const [id, p] of r.peers) {
        if (now - p.seen > PEER_EXPIRY_MS) {
          r.peers.delete(id);
          r.listeners.forEach((l) => l({ t: "left", room: r.name, conn: id }));
          dropped = true;
        }
      }
    }
    if (dropped) this.changed();
  }

  private handle(m: any) {
    const room = typeof m.room === "string" ? this.rooms.get(m.room) : undefined;
    switch (m.t) {
      case "hello":
        this.conn = m.conn;
        this.status = "open";
        this.retry = 0;
        for (const r of this.rooms.values()) this.send({ t: "sub", room: r.name });
        this.changed();
        return;
      case "joined":
        if (!room) return;
        room.joined = true;
        room.denied = false;
        room.write = !!m.write;
        room.peers = new Map((m.peers ?? []).map((p: any) => [p.conn, { ...p, state: p.state ?? {}, seen: Date.now() }]));
        if (room.state && room.shared) this.send({ t: "presence", room: room.name, state: room.state });
        break;
      case "denied":
        if (!room) return;
        room.joined = false;
        room.denied = true;
        room.peers.clear();
        break;
      case "peer":
        if (!room || m.conn === this.conn) return;
        room.peers.set(m.conn, { conn: m.conn, user: m.user, state: m.state ?? {}, seen: Date.now() });
        this.changed();
        return;
      case "left":
        if (!room) return;
        room.peers.delete(m.conn);
        break;
      case "who":
        // Another server node asks everyone to say again where they are.
        if (room?.joined && room.shared) this.send({ t: "presence", room: room.name, state: room.state ?? {} });
        return;
      case "notify":
        void queryClient.invalidateQueries({ queryKey: ["/api/v1/notifications"] });
        window.dispatchEvent(new CustomEvent("gantry:notify", { detail: m }));
        return;
      default:
        break;
    }
    room?.listeners.forEach((l) => l(m));
    if (m.t === "joined" || m.t === "denied" || m.t === "left") this.changed();
  }

  send(msg: Record<string, unknown>): boolean {
    if (this.ws?.readyState !== WebSocket.OPEN || !this.conn) return false;
    this.ws.send(JSON.stringify(msg));
    return true;
  }

  /** Enter a room. The returned function leaves it again. */
  join(name: string, listener?: Listener): () => void {
    let r = this.rooms.get(name);
    if (!r) {
      r = new Room(name);
      this.rooms.set(name, r);
    }
    const room = r;
    room.refs++;
    if (listener) room.listeners.add(listener);
    if (room.refs === 1) this.send({ t: "sub", room: name });
    else if (room.joined && listener) queueMicrotask(() => listener({ t: "joined", room: name, write: room.write, peers: [...room.peers.values()] }));
    this.stopped = false;
    this.connect();
    return () => {
      if (listener) room.listeners.delete(listener);
      room.refs--;
      if (room.refs <= 0) {
        this.rooms.delete(name);
        this.send({ t: "unsub", room: name });
        this.changed();
      }
    };
  }

  /** Say where in the room this tab is working. `null` clears it. */
  presence(name: string, state: Record<string, any> | null) {
    const r = this.rooms.get(name);
    if (!r) return;
    r.state = state;
    if (r.joined && r.shared) this.send({ t: "presence", room: name, state: state ?? {} });
  }

  /** On sign-out: close and forget everything. */
  stop() {
    this.stopped = true;
    clearTimeout(this.timer);
    clearInterval(this.beat);
    this.beat = undefined;
    this.rooms.clear();
    this.ws?.close();
    this.ws = null;
    this.status = "idle";
    this.changed();
  }
}

export const live = new Live();

export function useLiveStatus(): LiveStatus {
  useSyncExternalStore(live.subscribe, live.snapshot);
  return live.status;
}

/** Join a room for as long as the component is mounted. */
export function useRoom(name: string | null | undefined, onMessage?: Listener): { peers: Peer[]; write: boolean; joined: boolean; denied: boolean } {
  const handler = useRef(onMessage);
  handler.current = onMessage;
  useEffect(() => {
    if (!name) return;
    return live.join(name, (m) => handler.current?.(m));
  }, [name]);
  const version = useSyncExternalStore(live.subscribe, live.snapshot);
  return useMemo(() => {
    const r = name ? live.room(name) : undefined;
    // One entry per person: someone with the page open in two tabs is still one person.
    const seen = new Set<string>();
    const peers = [...(r?.peers.values() ?? [])].filter((p) => (seen.has(p.user.id + JSON.stringify(p.state)) ? false : (seen.add(p.user.id + JSON.stringify(p.state)), true)));
    return { peers, write: r?.write ?? false, joined: r?.joined ?? false, denied: r?.denied ?? false };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name, version]);
}

/** Tell the others in a room which field or part this tab is on. */
export function usePresence(name: string | null | undefined, state: Record<string, any> | null) {
  const text = JSON.stringify(state);
  useEffect(() => {
    if (!name) return;
    live.presence(name, text === "null" ? null : JSON.parse(text));
    return () => live.presence(name, null);
  }, [name, text]);
}

let refreshTimer: ReturnType<typeof setTimeout> | undefined;
function refreshSoon() {
  clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => void queryClient.invalidateQueries(), 250);
}

/** Refetch what is on screen when something in these rooms changes. */
export function useLiveRefresh(rooms: (string | null | undefined)[], onEvent?: (kind: string, data: any) => void) {
  const key = rooms.filter(Boolean).join("|");
  const cb = useRef(onEvent);
  cb.current = onEvent;
  useEffect(() => {
    const leave = key
      .split("|")
      .filter(Boolean)
      .map((room) =>
        live.join(room, (m) => {
          if (m.t !== "event") return;
          cb.current?.(m.kind, m.data);
          refreshSoon();
        }),
      );
    return () => leave.forEach((l) => l());
  }, [key]);
}

// ───────────────────────── shared documents ─────────────────────────

const CURSOR_COLORS = ["#1a56c4", "#b3261b", "#1c7a4b", "#8f5400", "#6b3fb5", "#0c6f72", "#b0307a", "#4d5b6a"];
export function colorFor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return CURSOR_COLORS[h % CURSOR_COLORS.length];
}

export interface SharedOptions {
  /** `text` for Markdown source, `rich` for the simple editor's document. */
  mode: "text" | "rich";
  me: Person;
  /** Called when this tab is the one asked to load the saved content into the empty document. */
  seed: (doc: Y.Doc) => void;
  /**
   * The working copy is gone: the saved content was replaced by another route, or someone
   * discarded the unsaved changes. The editor must start over from the saved content.
   * `mine` is true when this tab asked for the discard.
   */
  onReset?: (mine: boolean) => void;
  onState?: () => void;
}

/**
 * A Yjs document kept in step with everyone else who has the same thing open.
 * The server stores and forwards updates; it never reads them.
 */
export class SharedDoc {
  readonly ydoc = new Y.Doc();
  readonly awareness = new Awareness(this.ydoc);
  synced = false;
  canWrite = false;
  error: string | null = null;
  private leave: () => void;
  private seeding: Uint8Array[] | null = null;
  private waiting: ReturnType<typeof setTimeout> | undefined;
  private destroyed = false;

  constructor(readonly room: string, readonly doc: string, private opts: SharedOptions) {
    this.awareness.setLocalStateField("user", { name: opts.me.name, color: colorFor(opts.me.id), colorLight: `${colorFor(opts.me.id)}33`, kind: opts.me.kind, id: opts.me.id });
    this.ydoc.on("update", this.onLocalUpdate);
    this.awareness.on("update", this.onAwareness);
    this.leave = live.join(room, this.onMessage);
  }

  private onLocalUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin === this) return;
    if (this.seeding) {
      this.seeding.push(update);
      return;
    }
    if (!this.canWrite) return;
    live.send({ t: "y", room: this.room, doc: this.doc, u: toBase64(update) });
  };

  private onAwareness = ({ added, updated, removed }: { added: number[]; updated: number[]; removed: number[] }, origin: unknown) => {
    if (origin === this) return;
    const changed = [...added, ...updated, ...removed];
    if (!changed.includes(this.ydoc.clientID)) return;
    live.send({ t: "ya", room: this.room, doc: this.doc, u: toBase64(encodeAwarenessUpdate(this.awareness, [this.ydoc.clientID])) });
  };

  private onMessage = (m: any) => {
    if (this.destroyed) return;
    if (m.t === "joined") {
      this.canWrite = !!m.write;
      this.awareness.setLocalStateField("conn", live.conn);
      live.send({ t: "ysync", room: this.room, doc: this.doc, mode: this.opts.mode });
      return;
    }
    if (m.t === "offline") {
      this.synced = false;
      this.opts.onState?.();
      return;
    }
    if (m.t === "left") {
      const gone = [...this.awareness.getStates().entries()].filter(([, s]) => s.conn === m.conn).map(([id]) => id);
      if (gone.length) removeAwarenessStates(this.awareness, gone, this);
      return;
    }
    if (m.doc !== this.doc) return;
    switch (m.t) {
      case "ystate": {
        clearTimeout(this.waiting);
        for (const u of m.updates as string[]) Y.applyUpdate(this.ydoc, fromBase64(u), this);
        if (m.seed && this.canWrite) {
          // Everything the seeding writes goes out as one update, marked as the load.
          this.seeding = [];
          try {
            this.opts.seed(this.ydoc);
          } finally {
            const parts = this.seeding;
            this.seeding = null;
            if (parts.length) live.send({ t: "y", room: this.room, doc: this.doc, u: toBase64(Y.mergeUpdates(parts)), seed: true });
          }
          this.synced = true;
        } else if (m.updates.length === 0) {
          // Someone else is loading the saved content. If they vanish, ask again.
          this.waiting = setTimeout(() => live.send({ t: "ysync", room: this.room, doc: this.doc, mode: this.opts.mode }), 11_000);
        } else {
          this.synced = true;
          // A long edit history is folded into one update by whoever opens the document next.
          if (this.canWrite && m.updates.length > 60) live.send({ t: "ycompact", room: this.room, doc: this.doc, upto: m.last, u: toBase64(Y.encodeStateAsUpdate(this.ydoc)) });
        }
        // Say where this tab's cursor is to those who were here first.
        live.send({ t: "ya", room: this.room, doc: this.doc, u: toBase64(encodeAwarenessUpdate(this.awareness, [this.ydoc.clientID])) });
        this.opts.onState?.();
        break;
      }
      case "y":
        clearTimeout(this.waiting);
        Y.applyUpdate(this.ydoc, fromBase64(m.u), this);
        if (!this.synced) {
          this.synced = true;
          this.opts.onState?.();
        }
        break;
      case "ya":
        try {
          applyAwarenessUpdate(this.awareness, fromBase64(m.u), this);
        } catch {
          /* a malformed cursor update from another tab is not worth interrupting anyone for */
        }
        break;
      case "yreset":
        this.opts.onReset?.(!!m.from && m.from === live.conn);
        break;
      case "error":
        if (m.of === "y" || m.of === "ysync") {
          this.error = m.detail ?? "The shared editing session reported a problem.";
          this.opts.onState?.();
        }
        break;
      default:
        break;
    }
  };

  /**
   * Throw away everything typed and not saved, for everyone who has this open. The next
   * editor starts again from the saved content. The server records who did it.
   */
  discard() {
    live.send({ t: "ydiscard", room: this.room, doc: this.doc });
  }

  destroy() {
    this.destroyed = true;
    clearTimeout(this.waiting);
    removeAwarenessStates(this.awareness, [this.ydoc.clientID], "leave");
    this.ydoc.off("update", this.onLocalUpdate);
    this.awareness.off("update", this.onAwareness);
    this.leave();
    this.awareness.destroy();
    this.ydoc.destroy();
  }
}

/**
 * Open a shared document while `on` is true. `ready` turns true once it is in step with the
 * others. If the live connection cannot be reached, `alone` turns true instead and the
 * caller edits by itself: saving still works, only the others' typing is not seen.
 */
export function useSharedDoc(room: string, doc: string, on: boolean, me: Person, mode: "text" | "rich", seed: (d: Y.Doc) => void, onReset?: (mine: boolean) => void): { shared: SharedDoc | null; ready: boolean; alone: boolean } {
  const [shared, setShared] = useState<SharedDoc | null>(null);
  const [, bump] = useState(0);
  const [alone, setAlone] = useState(false);
  const fns = useRef({ seed, onReset });
  fns.current = { seed, onReset };
  useEffect(() => {
    if (!on) return;
    setAlone(false);
    const s = new SharedDoc(room, doc, { mode, me, seed: (d) => fns.current.seed(d), onReset: (mine) => fns.current.onReset?.(mine), onState: () => bump((n) => n + 1) });
    setShared(s);
    const giveUp = setTimeout(() => {
      if (!s.synced) setAlone(true);
    }, 3000);
    return () => {
      clearTimeout(giveUp);
      s.destroy();
      setShared(null);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [room, doc, on, mode, me.id]);
  const ready = !!shared && shared.synced && !alone;
  // Not sharing (a new page, or the person chose to continue alone) is ready at once:
  // there is nobody to wait for.
  return { shared: on && ready ? shared : null, ready: !on || ready || alone, alone: on && alone };
}
