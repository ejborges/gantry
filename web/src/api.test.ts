import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, api, qs, setCsrf, upload } from "./api";

const respond = (status: number, body?: unknown) => new Response(body === undefined ? null : typeof body === "string" ? body : JSON.stringify(body), { status });

afterEach(() => {
  vi.restoreAllMocks();
  setCsrf(null);
});

describe("qs", () => {
  it("drops empty values and encodes the rest", () => {
    expect(qs({ a: 1, b: "", c: null, d: undefined, e: "x y&z", f: false })).toBe("?a=1&e=x+y%26z&f=false");
    expect(qs({})).toBe("");
  });
});

describe("api", () => {
  it("sends the CSRF token on every request that changes something, and never on reads", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(async () => respond(200, { ok: true }));
    setCsrf("tok123");
    await api("GET", "/api/v1/items");
    await api("POST", "/api/v1/items", { title: "x" });
    await api("PATCH", "/api/v1/items/A-1", { title: "y" });
    await api("DELETE", "/api/v1/items/A-1");
    const headers = fetchMock.mock.calls.map((c) => (c[1] as RequestInit).headers as Record<string, string>);
    expect(headers[0]["X-Gantry-CSRF"]).toBeUndefined();
    for (const h of headers.slice(1)) expect(h["X-Gantry-CSRF"]).toBe("tok123");
    expect(headers[1]["Content-Type"]).toBe("application/json");
    for (const c of fetchMock.mock.calls) expect((c[1] as RequestInit).credentials).toBe("same-origin");
  });

  it("sends the token with uploads and names the file", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(async () => respond(201, { id: "1" }));
    setCsrf("tok123");
    await upload("/api/v1/items/A-1/attachments", new File(["x"], "log 1.txt", { type: "text/plain" }));
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/v1/items/A-1/attachments?filename=log%201.txt");
    expect((init.headers as Record<string, string>)["X-Gantry-CSRF"]).toBe("tok123");
  });

  it("returns null for an empty success", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => respond(204));
    expect(await api("DELETE", "/api/v1/items/A-1")).toBeNull();
  });

  it("raises the server's explanation from a problem document", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => respond(409, { status: 409, code: "conflict", detail: "the page changed since you opened it" }));
    const err = await api("PUT", "/api/v1/pages/1", {}).catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(409);
    expect(err.code).toBe("conflict");
    expect(err.message).toBe("the page changed since you opened it");
  });

  it("explains failures that carry no detail, without exposing internals", async () => {
    const statuses: [number, RegExp][] = [[403, /permission/], [404, /does not exist, or you cannot see it/], [413, /too large/], [429, /Too many requests/], [500, /server could not complete/], [418, /failed \(418\)/]];
    for (const [status, text] of statuses) {
      vi.spyOn(globalThis, "fetch").mockImplementation(async () => respond(status, "<html>upstream stack trace</html>"));
      const err = await api("GET", "/x").catch((e) => e);
      expect(err.message).toMatch(text);
      expect(err.message).not.toContain("stack trace");
    }
  });

  it("quotes the request reference when the server is at fault", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(JSON.stringify({ status: 500, detail: "internal error" }), { status: 500, headers: { "x-request-id": "f1cd1338e1a578d7" } }));
    const err = await api("GET", "/api/v1/items").catch((e) => e);
    expect(err.message).toBe("The server could not complete the request. Try again; if it keeps failing, tell an administrator. Reference: f1cd1338e1a578d7.");
    expect(err.requestId).toBe("f1cd1338e1a578d7");
    // Mistakes by the user get a plain explanation without a reference to chase.
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(JSON.stringify({ status: 400, detail: "title is required" }), { status: 400, headers: { "x-request-id": "abc" } }));
    expect((await api("POST", "/api/v1/items", {}).catch((e) => e)).message).toBe("title is required");
  });

  it("says in plain words that something is missing or hidden", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => respond(404, { status: 404, code: "not_found", detail: "not found" }));
    const err = await api("GET", "/api/v1/items/NOPE-1").catch((e) => e);
    expect(err.message).toBe("That does not exist, or you cannot see it.");
    expect(err.code).toBe("not_found");
  });

  it("announces a lost session so the app can return to the sign-in page", async () => {
    vi.spyOn(globalThis, "fetch").mockImplementation(async () => respond(401, { detail: "authentication required" }));
    const heard = vi.fn();
    window.addEventListener("gantry:signed-out", heard);
    await api("GET", "/api/v1/items").catch(() => null);
    window.removeEventListener("gantry:signed-out", heard);
    expect(heard).toHaveBeenCalledTimes(1);
  });
});
