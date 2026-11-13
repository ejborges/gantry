import { expect, test, type Page } from "@playwright/test";
import { ADMIN, Api, MEMBER, signInAs, uniq } from "./helpers";

// Text that would run script, load a remote resource or forge a form if it reached the page unescaped.
const PAYLOADS = [
  `<script>window.__pwned = 'script'</script>`,
  `<img src=x onerror="window.__pwned = 'img'">`,
  `<svg onload="window.__pwned = 'svg'"></svg>`,
  `"><iframe srcdoc="<script>parent.__pwned = 'iframe'</script>"></iframe>`,
  `[link](javascript:window.__pwned='href')`,
  `<a href="javascript:window.__pwned='a'">x</a>`,
  `<details open ontoggle="window.__pwned = 'toggle'">x</details>`,
];
const HOSTILE = PAYLOADS.join("\n\n");

async function expectClean(page: Page, where: string) {
  await page.waitForLoadState("networkidle");
  expect(await page.evaluate(() => (window as any).__pwned), `script ran on ${where}`).toBeUndefined();
  expect(await page.locator("main script, main iframe, main [onerror], main [onload], main [ontoggle]").count(), `active content on ${where}`).toBe(0);
  expect(await page.locator('main a[href^="javascript:" i]').count(), `javascript: link on ${where}`).toBe(0);
}

test.describe("content from users never runs in the page", () => {
  test("hostile text in titles, descriptions, comments, labels and pages is shown as text", async ({ page, baseURL }) => {
    const dialogs: string[] = [];
    page.on("dialog", (d) => (dialogs.push(d.message()), void d.dismiss()));
    const api = await Api.as(baseURL!, MEMBER);
    const key = `X${uniq()}`;
    await api.ok("POST", "/api/v1/projects", { key, name: `<img src=x onerror="window.__pwned='project'"> ${key}`, template: "basic", description: HOSTILE });
    const item = await api.ok("POST", `/api/v1/projects/${key}/items`, { title: `<script>window.__pwned='title'</script><b>bold?</b>`, description: HOSTILE, labels: [`"><img src=x onerror=alert(1)>`] });
    await api.ok("POST", `/api/v1/items/${item.key}/comments`, { body: HOSTILE });
    await api.ok("POST", "/api/v1/spaces", { key, name: `<svg onload="window.__pwned='space'"> ${key}` });
    const doc = await api.ok("POST", `/api/v1/spaces/${key}/pages`, { title: `<img src=x onerror="window.__pwned='pagetitle'">`, content: HOSTILE, status: "published" });

    await signInAs(page, MEMBER);
    for (const [path, where] of [
      [`/i/${item.key}`, "the item page"],
      [`/projects/${key}?view=board`, "the board"],
      [`/projects/${key}?view=list`, "the list"],
      [`/projects`, "the project list"],
      [`/docs/${key}/${doc.id}`, "the document"],
      [`/docs/${key}/${doc.id}/history`, "the page history"],
      [`/docs`, "the space list"],
      [`/search?q=${encodeURIComponent("pwned")}`, "search results"],
      [`/`, "My work"],
    ] as const) {
      await page.goto(path);
      await expectClean(page, where);
    }
    // The title is displayed literally, angle brackets and all.
    await page.goto(`/i/${item.key}`);
    await expect(page.getByLabel("Title")).toHaveValue(/<script>window\.__pwned='title'<\/script>/);
    // Both editors load the same text without running it.
    await page.goto(`/docs/${key}/${doc.id}/edit`);
    await expectClean(page, "the page editor");
    expect(dialogs, "no alert, confirm or prompt opened").toEqual([]);
    await api.dispose();
  });

  test("the administration console shows hostile names and audit details as text", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const email = `x-${uniq().toLowerCase()}@example.org`;
    await api.ok("POST", "/api/v1/admin/users", { email, name: `<img src=x onerror="window.__pwned='username'">`, password: "correct horse battery staple", role: "member" });
    await page.request.post("/api/v1/auth/login", { data: { email: `<script>window.__pwned='login'</script>@example.org`, password: "x" } });
    await signInAs(page, ADMIN);
    for (const path of ["/admin/users", "/admin/audit", "/admin/security", "/admin/overview"]) {
      await page.goto(path);
      await expectClean(page, path);
    }
    await api.dispose();
  });
});

test.describe("browser protections", () => {
  test("every response carries a strict content security policy and anti-framing headers", async ({ page }) => {
    for (const path of ["/login", "/", "/api/v1/auth/providers", "/manual/"]) {
      const r = await page.request.get(path);
      const h = r.headers();
      expect(h["content-security-policy"], path).toContain("default-src 'self'");
      expect(h["content-security-policy"], path).toContain("frame-ancestors 'none'");
      expect(h["content-security-policy"], path).not.toContain("'unsafe-eval'");
      expect(h["content-security-policy"], path).toMatch(/script-src 'self'(;|$| (?!'unsafe-inline'))/);
      expect(h["x-content-type-options"], path).toBe("nosniff");
      expect(h["x-frame-options"], path).toBe("DENY");
      expect(h["referrer-policy"], path).toBe("same-origin");
    }
    expect((await page.request.get("/api/v1/auth/providers")).headers()["cache-control"]).toBe("no-store");
  });

  test("the policy stops inline script even if markup were injected", async ({ page }) => {
    const violations: string[] = [];
    await signInAs(page, MEMBER);
    await page.exposeFunction("__report", (d: string) => violations.push(d));
    await page.evaluate(() => {
      document.addEventListener("securitypolicyviolation", (e) => (window as any).__report(e.violatedDirective));
      const s = document.createElement("script");
      s.textContent = "window.__pwned = 'inline'";
      document.body.appendChild(s);
      const img = document.createElement("img");
      img.setAttribute("onerror", "window.__pwned = 'handler'");
      img.src = "data:,broken";
      document.body.appendChild(img);
    });
    await page.waitForTimeout(300);
    expect(await page.evaluate(() => (window as any).__pwned)).toBeUndefined();
    expect(violations.join(" ")).toMatch(/script-src/);
  });

  test("a request that changes data without the session's CSRF token is refused", async ({ page }) => {
    await signInAs(page, MEMBER);
    const before = await (await page.request.get("/api/v1/items/PUMP-2")).json();
    // The browser sends the session cookie, as it would for a forged cross-site form.
    const forged = await page.request.patch("/api/v1/items/PUMP-2", { data: { title: "Forged" } });
    expect(forged.status()).toBe(403);
    const crossSite = await page.request.patch("/api/v1/items/PUMP-2", { data: { title: "Forged" }, headers: { Origin: "https://evil.example", "X-Gantry-CSRF": "guess" } });
    expect(crossSite.status()).toBe(403);
    expect((await (await page.request.get("/api/v1/items/PUMP-2")).json()).title).toBe(before.title);
  });

  test("a signed-in session cannot be used from another site's page", async ({ page }) => {
    await signInAs(page, MEMBER);
    const r = await page.request.get("/api/v1/auth/me", { headers: { Origin: "https://evil.example" } });
    expect(r.headers()["access-control-allow-origin"]).toBeUndefined();
    expect(r.headers()["access-control-allow-credentials"]).toBeUndefined();
  });

  test("an uploaded HTML file is downloaded, never displayed as a page", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, MEMBER);
    const key = `U${uniq()}`;
    await api.ok("POST", "/api/v1/projects", { key, name: `Uploads ${key}`, template: "basic" });
    const item = await api.ok("POST", `/api/v1/projects/${key}/items`, { title: "Evidence" });
    await signInAs(page, MEMBER);
    const csrf = (await (await page.request.get("/api/v1/auth/me")).json()).csrf;
    const up = await page.request.post(`/api/v1/items/${item.key}/attachments?filename=${encodeURIComponent("report.html")}`, { headers: { "X-Gantry-CSRF": csrf, "Content-Type": "text/html" }, data: "<script>window.__pwned='upload'</script>" });
    expect(up.status()).toBe(201);
    const file = await page.request.get(`/api/v1/attachments/${(await up.json()).id}`);
    const h = file.headers();
    expect(h["content-disposition"]).toMatch(/^attachment/);
    expect(h["content-type"]).not.toContain("text/html");
    expect(h["x-content-type-options"]).toBe("nosniff");
    await api.dispose();
  });
});
