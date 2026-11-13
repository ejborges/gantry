import { expect, test, type Page } from "@playwright/test";
import { ADMIN, Api, MEMBER, signInAs, uniq } from "./helpers";

/** The frame a tile's content runs in. */
const frameOf = (page: Page, tile: string) => page.getByRole("region", { name: tile }).frameLocator("iframe");

test.describe("dashboards and visualizations", () => {
  test("build a dashboard from built-in tiles, arrange it with the keyboard, and share it", async ({ page, browser, baseURL }) => {
    const name = `Flow ${uniq()}`;
    await signInAs(page, ADMIN);
    await page.goto("/analytics");
    await page.getByRole("button", { name: "New dashboard" }).click();
    await page.getByRole("dialog").getByLabel("Name").fill(name);
    await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
    await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();

    for (const tile of ["Key figures", "Work by status", "Due next"]) {
      await page.getByRole("button", { name: "Add a tile" }).first().click();
      await page.getByRole("dialog").getByRole("button", { name: new RegExp(`^${tile}`) }).click();
      await expect(page.getByRole("region", { name: tile })).toBeVisible();
    }
    // Each tile draws inside a sandboxed frame, from data read as the person looking at it.
    await expect(frameOf(page, "Key figures").getByText("Unassigned")).toBeVisible({ timeout: 20_000 });
    await expect(frameOf(page, "Work by status").getByRole("img", { name: /Work by status: \d+ / })).toBeVisible({ timeout: 20_000 });
    await expect(frameOf(page, "Due next").getByRole("columnheader", { name: "Assignee" })).toBeVisible({ timeout: 20_000 });
    const sandbox = await page.getByRole("region", { name: "Key figures" }).locator("iframe").getAttribute("sandbox");
    expect(sandbox).toBe("allow-scripts"); // no allow-same-origin: the frame cannot read the page or the session

    // Keyboard: move a tile one column to the right; it is saved and nothing overlaps.
    const id = page.url().split("/").pop()!.split("?")[0];
    const api = await Api.as(baseURL!, ADMIN);
    const before = (await api.ok("GET", `/api/v1/dashboards/${id}`)).layout.find((t: any) => t.ref === "status-donut");
    const grip = page.getByRole("button", { name: /^Move Work by status/ });
    await grip.focus();
    await page.keyboard.press("Shift+ArrowLeft"); // one column narrower
    await page.keyboard.press("ArrowDown");
    await expect.poll(async () => (await api.ok("GET", `/api/v1/dashboards/${id}`)).layout.find((t: any) => t.ref === "status-donut").w).toBe(before.w - 1);
    const layout = (await api.ok("GET", `/api/v1/dashboards/${id}`)).layout;
    const overlap = layout.some((a: any) => layout.some((b: any) => a.id !== b.id && a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h));
    expect(overlap).toBe(false);
    // The tile kept its frame through the move: content is not fetched again for a change of place.
    await expect(frameOf(page, "Work by status").getByRole("img", { name: /Work by status/ })).toBeVisible();

    // Shared: another member can open it, sees their own figures, and cannot change it.
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("dialog").getByLabel("Who can see it").selectOption("workspace");
    await page.getByRole("dialog").getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.getByText("Shared").first()).toBeVisible();
    const ctx = await browser.newContext();
    const other = await ctx.newPage();
    await signInAs(other, MEMBER);
    await other.goto(`/analytics/d/${id}`);
    await expect(other.getByRole("heading", { name, level: 1 })).toBeVisible();
    await expect(frameOf(other, "Key figures").getByText("Unassigned")).toBeVisible({ timeout: 20_000 });
    await expect(other.getByRole("button", { name: "Edit", exact: true })).toHaveCount(0);
    const member = await Api.as(baseURL!, MEMBER);
    expect((await member.call("PUT", `/api/v1/dashboards/${id}/tiles/x1`, { kind: "template", ref: "kpi-tiles" })).status()).toBe(403);
    await member.dispose();
    await api.dispose();
    await ctx.close();
  });

  test("write a visualization of your own: the preview runs the script, errors are shown, and it can be embedded", async ({ page, baseURL }) => {
    const name = `Open by type ${uniq()}`;
    await signInAs(page, ADMIN);
    await page.goto("/analytics/library");
    await page.getByRole("button", { name: "New visualization" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("Name").fill(name);
    await dialog.getByLabel("Written with").selectOption("d3");
    await dialog.getByRole("button", { name: "Create" }).click();
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue(name);
    const preview = page.locator(".viz-preview").frameLocator("iframe");
    await expect(preview.getByRole("img", { name: /Open work by priority/ })).toBeVisible({ timeout: 20_000 });

    // Replace the script. The preview follows what is typed.
    const editor = page.getByRole("textbox", { name: /^Script of / });
    await editor.fill("const s = await gantry.api.get('/api/v1/analytics/summary', {});\nconst p = gantry.root.appendChild(document.createElement('p'));\np.id = 'answer';\np.textContent = 'Open now: ' + s.open;\n");
    await expect(preview.locator("#answer")).toContainText(/Open now: \d+/, { timeout: 20_000 });

    // A script cannot change anything: the page that brokers its requests only reads.
    await editor.fill("try { await fetch('/api/v1/auth/me'); gantry.root.textContent = 'reached the network'; } catch (e) { gantry.root.textContent = 'no network'; }\n");
    await expect(preview.getByText("no network")).toBeVisible({ timeout: 20_000 });
    await editor.fill("const top = window.parent.document; gantry.root.textContent = 'read the page';\n");
    await expect(page.locator(".viz-preview").getByRole("alert")).toBeVisible({ timeout: 20_000 });
    await expect(preview.getByText("read the page")).toHaveCount(0);

    // A mistake is reported next to the preview, in the frame and outside it.
    await editor.fill("this is not JavaScript\n");
    await expect(page.locator(".viz-preview").getByRole("alert")).toContainText(/\S/);

    await editor.fill("gantry.root.textContent = 'Saved ' + (gantry.params.note || 'nothing');\n");
    await page.getByLabel("Name of a new parameter").fill("note");
    await page.getByRole("button", { name: "Add", exact: true }).click();
    await page.getByLabel("Default value of note").fill("this");
    await expect(preview.getByText("Saved this")).toBeVisible({ timeout: 20_000 });
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.getByText("Saved", { exact: true }).last()).toBeVisible();

    // The same visualization inside a document, by pasting its line.
    const id = page.url().split("/").pop()!;
    const api = await Api.as(baseURL!, ADMIN);
    expect((await api.ok("GET", `/api/v1/visualizations/${id}`)).source).toContain("Saved ");
    const key = `V${uniq()}`;
    await api.ok("POST", "/api/v1/spaces", { key, name: `Charts ${key}` });
    const doc = await api.ok("POST", `/api/v1/spaces/${key}/pages`, { title: "With a chart", editor: "advanced", status: "published", content: `Figure:\n\n![[viz:${id}|200]]\n` });
    await page.goto(`/docs/${key}/${doc.id}`);
    await expect(page.locator(".prose").frameLocator("iframe").getByText("Saved this")).toBeVisible({ timeout: 20_000 });
    await api.dispose();
  });

  test("a built-in visualization shows how it is written and can be copied to change", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/analytics/library");
    await page.getByRole("link", { name: /Workload by assignee/ }).click();
    await expect(page.getByRole("heading", { name: /Workload by assignee/, level: 1 })).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Source of Workload by assignee" })).toContainText("gantry.api.get");
    await page.getByText("How a script gets its data, and what it may do").click();
    await expect(page.getByRole("cell", { name: "/api/v1/analytics/workload" })).toBeVisible();
    await page.getByRole("button", { name: "Make a copy to change" }).click();
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue("My workload by assignee");
    await expect(page.getByRole("button", { name: "Save a copy" })).toBeVisible();
  });
});
