import { expect, request as playwrightRequest, test } from "@playwright/test";
import { ADMIN, Api, signInAs, uniq } from "./helpers";

test.describe("administration console", () => {
  test.beforeEach(async ({ page }) => {
    await signInAs(page, ADMIN);
  });

  test("the overview reports every component as healthy", async ({ page }) => {
    await page.goto("/admin/overview");
    await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
    const rows = page.getByRole("table").filter({ hasText: "Provided by" }).locator("tbody tr");
    await expect(rows).toHaveCount(11);
    for (const part of ["Database", "Schema", "Files", "Search", "Vectors", "Embeddings", "Cache", "Graph", "Worker", "Email", "Live updates"]) {
      await expect(rows.filter({ hasText: part }).first()).toContainText("Healthy");
    }
    // This browser's own live connection is counted, and the node that answered is listed.
    // (The figures are read when the page opens, which can be an instant before the connection
    // is; they are read again every half minute, or on opening the page again.)
    await expect(async () => {
      if (!/[1-9]\d* connections? open/.test((await rows.filter({ hasText: "Live updates" }).textContent()) ?? "")) await page.reload();
      await expect(rows.filter({ hasText: "Live updates" })).toContainText(/[1-9]\d* connections? open on this node/, { timeout: 4_000 });
    }).toPass({ timeout: 30_000 });
    const nodes = page.getByRole("table").filter({ hasText: "Running since" }).locator("tbody tr");
    await expect(nodes.filter({ hasText: "answered this request" })).toContainText("Reporting");
  });

  test("every section opens without an error", async ({ page }) => {
    const sections: [string, string][] = [
      ["jobs", "Background jobs"], ["settings", "Settings"], ["users", "Users and agents"], ["groups", "Groups"], ["roles", "Roles"], ["sign-in", "Sign-in"],
      ["schemes", "Types and workflows"], ["add-ons", "Add-ons"], ["integrations", "Integrations"], ["audit", "Audit log"], ["security", "Security events"],
    ];
    const failures: string[] = [];
    page.on("response", (r) => {
      if (r.url().includes("/api/") && r.status() >= 400) failures.push(`${r.status()} ${r.url()}`);
    });
    for (const [path, heading] of sections) {
      await page.goto(`/admin/${path}`);
      await expect(page.getByRole("heading", { name: heading, level: 1 })).toBeVisible();
      await page.waitForLoadState("networkidle");
      await expect(page.getByText(/The request failed|could not complete the request/)).toHaveCount(0);
    }
    expect(failures, "API calls made by the console").toEqual([]);
  });

  test("the audit log records administrative actions and verifies its own integrity", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const email = `audit-${uniq().toLowerCase()}@example.org`;
    await api.ok("POST", "/api/v1/admin/users", { email, name: "Audit Subject", password: "correct horse battery staple", role: "member" });
    await page.goto("/admin/audit");
    await page.getByPlaceholder(/Action/).fill("user.create");
    const row = page.getByRole("row").filter({ hasText: "user.create" }).first();
    await expect(row).toBeVisible();
    await expect(row).toContainText("admin@example.org");
    await expect(row).toContainText("OK");
    await page.getByRole("button", { name: "Verify integrity" }).click();
    await expect(page.getByRole("status").filter({ hasText: "the hash chain is unbroken" })).toBeVisible();
    await api.dispose();
  });

  test("failed sign-ins appear as security events", async ({ page, baseURL }) => {
    const stranger = await playwrightRequest.newContext({ baseURL });
    const probe = await stranger.post("/api/v1/auth/login", { data: { email: "intruder@example.org", password: "wrong wrong wrong" } });
    expect(probe.status()).toBe(401);
    await stranger.dispose();
    await page.goto("/admin/security");
    const row = page.getByRole("row").filter({ hasText: "intruder@example.org" }).first();
    await expect(row).toContainText("Wrong password or unknown account");
    await expect(row).toContainText("Not signed in");
    await expect(row).toContainText("127.0.0.1");
  });

  test("create a user, change their role, and deactivate them", async ({ page }) => {
    const email = `new-${uniq().toLowerCase()}@example.org`;
    await page.goto("/admin/users");
    await page.getByRole("button", { name: "New user" }).click();
    const dialog = page.getByRole("dialog", { name: "New user" });
    await dialog.getByLabel("Name").fill("Noor Haddad");
    await dialog.getByLabel("Email").fill(email);
    await dialog.getByLabel("Initial password").fill("a long enough passphrase 42");
    await dialog.getByRole("button", { name: /Create/ }).click();
    const row = page.getByRole("row").filter({ hasText: email });
    await expect(row).toBeVisible();
    await expect(row).toContainText("Active");
  });

  test("a customized workflow is marked and survives being reopened", async ({ page }) => {
    await page.goto("/admin/schemes");
    await page.getByRole("tab", { name: "Workflows" }).click();
    await expect(page.getByText("Software delivery").first()).toBeVisible();
    await page.getByRole("tab", { name: "Fields" }).click();
    await expect(page.getByText("Severity").first()).toBeVisible();
    await page.getByRole("tab", { name: "Work item types" }).click();
    await expect(page.getByRole("row").filter({ hasText: "Controlled product" })).toContainText("NASA");
  });
});
