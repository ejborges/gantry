// Thin client for the Gantry REST API. Sessions are cookie based; every unsafe request
// carries the per-session CSRF token the server hands out with /auth/me.
import { QueryClient, useQuery, type UseQueryOptions } from "@tanstack/react-query";

export class ApiError extends Error {
  /** The server's identifier for the failed request; administrators find it in the logs. */
  requestId?: string;
  constructor(public status: number, message: string, public code?: string) {
    super(message);
  }
}

let csrf: string | null = null;
export function setCsrf(token: string | null) {
  csrf = token;
}

async function parse(resp: Response) {
  if (resp.status === 204) return null;
  const text = await resp.text();
  let data: any = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!resp.ok) {
    if (resp.status === 401 && !location.pathname.startsWith("/login")) {
      window.dispatchEvent(new Event("gantry:signed-out"));
    }
    const detail = data && typeof data === "object" ? data.detail : null;
    // The server answers "not found" both for things that do not exist and for things the
    // caller may not see; say so in plain words instead of echoing the bare status text.
    const generic = !detail || detail === "not found" || detail === "forbidden" || detail === "internal error";
    const requestId = resp.headers.get("x-request-id") ?? undefined;
    // For failures that are the server's fault, show the reference an administrator can look up.
    const reference = resp.status >= 500 && requestId ? ` Reference: ${requestId}.` : "";
    const err = new ApiError(resp.status, (generic ? messageFor(resp.status) : detail) + reference, data?.code);
    err.requestId = requestId;
    throw err;
  }
  return data;
}

function messageFor(status: number) {
  if (status === 401) return "Your session has ended. Sign in again.";
  if (status === 403) return "You do not have permission to do that.";
  if (status === 404) return "That does not exist, or you cannot see it.";
  if (status === 413) return "That file is too large.";
  if (status === 429) return "Too many requests. Wait a minute and try again.";
  if (status >= 500) return "The server could not complete the request. Try again; if it keeps failing, tell an administrator.";
  return `The request failed (${status}).`;
}

export async function api<T = any>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (method !== "GET" && csrf) headers["X-Gantry-CSRF"] = csrf;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const resp = await fetch(path, { method, headers, credentials: "same-origin", body: body === undefined ? undefined : JSON.stringify(body) });
  return parse(resp);
}

export async function upload(path: string, file: File, method = "POST") {
  const headers: Record<string, string> = { "Content-Type": file.type || "application/octet-stream" };
  if (csrf) headers["X-Gantry-CSRF"] = csrf;
  const sep = path.includes("?") ? "&" : "?";
  const resp = await fetch(`${path}${sep}filename=${encodeURIComponent(file.name)}`, { method, headers, credentials: "same-origin", body: file });
  return parse(resp);
}

/**
 * A read made on behalf of custom content running in the sandbox. Only GET, only the API,
 * and marked so that the server refuses anything else even if this check were bypassed.
 */
export async function sandboxGet(path: string, query?: Record<string, unknown> | null): Promise<unknown> {
  if (typeof path !== "string" || !path.startsWith("/api/v1/") || path.includes("..") || path.includes("//") || /[\\#]/.test(path)) throw new ApiError(400, "Custom content can read only addresses under /api/v1/.");
  const [bare, own] = path.split("?");
  const params = new URLSearchParams(own ?? "");
  for (const [k, v] of Object.entries(query ?? {})) {
    if (v === undefined || v === null) continue;
    params.set(k, typeof v === "object" ? JSON.stringify(v) : String(v));
  }
  const text = params.toString();
  const resp = await fetch(text ? `${bare}?${text}` : bare, { method: "GET", headers: { Accept: "application/json", "X-Gantry-Sandbox": "1" }, credentials: "same-origin" });
  return parse(resp);
}

export const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 15_000, retry: (n, e) => !(e instanceof ApiError && e.status < 500) && n < 2, refetchOnWindowFocus: true } },
});

/** GET with caching. The path is the cache key. Pass `null` to skip. */
export function useGet<T = any>(path: string | null, options?: Partial<UseQueryOptions<T, ApiError>>) {
  return useQuery<T, ApiError>({ queryKey: [path], queryFn: () => api<T>("GET", path!), enabled: path !== null, ...options });
}

/** After any change, refresh everything on screen. Simple, and correct for a tool where one edit touches many views. */
export function refresh() {
  return queryClient.invalidateQueries();
}

export function qs(params: Record<string, string | number | boolean | undefined | null>) {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== null && v !== "") u.set(k, String(v));
  const s = u.toString();
  return s ? `?${s}` : "";
}

export interface Person {
  id: string;
  name: string;
  /** `human` or `agent` (an AI agent account). */
  kind?: string;
  email?: string;
  handle?: string;
  active?: boolean;
  job_title?: string;
  /** Address of the picture: an uploaded one, or the generated identicon. */
  avatar?: string;
  presence?: "available" | "away" | "busy" | "out_of_office" | string;
  status_message?: string;
  org_name?: string;
  org_code?: string;
}
export interface FieldDef {
  key: string;
  name: string;
  type: string;
  options: string[];
  description?: string;
  required?: boolean;
}
export interface Status {
  key: string;
  name: string;
  category: "todo" | "in_progress" | "done";
}
export interface Transition {
  name: string;
  from: string[];
  to: string;
  requires?: string[];
}
export interface ItemType {
  key: string;
  name: string;
  icon: string;
  color: string;
  description: string;
  fields: FieldDef[];
  workflow: { key: string; name: string; initial: string; statuses: Status[]; transitions: Transition[] };
}
export interface Item {
  id: string;
  key: string;
  project: string;
  number: number;
  type: string;
  title: string;
  status: string;
  status_category: "todo" | "in_progress" | "done";
  priority: number;
  assignee: Person | null;
  reporter: Person | null;
  labels: string[];
  due_date: string | null;
  start_date: string | null;
  estimate: number | null;
  cycle_id: string | null;
  rank: number;
  created_at: string;
  updated_at: string;
  sla: null | { response_due: string | null; first_response_at: string | null; resolution_due: string | null; response_breached: boolean; resolution_breached: boolean };
  [extra: string]: any;
}
export interface Me {
  user: Person & { id: string; name: string; email: string; kind: string; theme: "system" | "light" | "dark"; timezone: string; status_until: string | null; has_avatar: boolean; has_password: boolean; description?: string };
  csrf: string | null;
  permissions: string[];
  unread_notifications: number;
  features: { semantic_search: boolean; local_login: boolean; email: boolean };
  site: { name: string; announcement: string; timezone: string; support: { name: string; email: string; url: string; text: string } };
  profile_rules: { user_editable: Record<string, boolean>; avatar_max_kb: number; avatar_max_px: number; avatar_suggested_px: number };
  version: string;
}

/** What the server resolved in a text: only what the reader may see is in here. */
export interface Refs {
  items?: Record<string, { key: string; title: string; url: string; status_category?: string; status_name?: string; type?: string }>;
  users?: Record<string, Person>;
  pages?: Record<string, { id: string; title: string; url: string }>;
  projects?: Record<string, { key: string; name: string; url: string }>;
}
export interface Attachment {
  id: string;
  filename: string;
  content_type: string;
  size: number;
  created_at?: string;
  uploaded_by?: Person | null;
}
