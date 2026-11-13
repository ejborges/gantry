import { expect, test } from "@playwright/test";
import { ADMIN, Api, MEMBER, signInAs, uniq } from "./helpers";

const iso = (offset: number) => new Date(Date.now() + offset * 86_400_000).toISOString().slice(0, 10);

test.describe("planning", () => {
  test("a bar moved on the Gantt chart changes the work item, and the change is in its history", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const key = `G${uniq()}`;
    await api.ok("POST", "/api/v1/projects", { key, name: `Gantt ${key}`, template: "basic" });
    const item = await api.ok("POST", `/api/v1/projects/${key}/items`, { title: "Pour the foundation", start_date: iso(1), due_date: iso(4) });
    await signInAs(page, ADMIN);
    await page.goto(`/projects/${key}?view=gantt`);
    const bar = page.getByRole("button", { name: new RegExp(`^${item.key} Pour the foundation`) });
    await expect(bar).toBeVisible();
    // Keyboard: the arrow keys move the whole bar by a day; with Shift only the due date moves.
    await bar.focus();
    await page.keyboard.press("ArrowRight");
    await expect.poll(async () => (await api.ok("GET", `/api/v1/items/${item.key}`)).due_date).toBe(iso(5));
    expect((await api.ok("GET", `/api/v1/items/${item.key}`)).start_date).toBe(iso(2));
    await bar.focus();
    await page.keyboard.press("Shift+ArrowRight");
    await expect.poll(async () => (await api.ok("GET", `/api/v1/items/${item.key}`)).due_date).toBe(iso(6));
    expect((await api.ok("GET", `/api/v1/items/${item.key}`)).start_date).toBe(iso(2));
    // Pointer: dragging the bar by one week on the week scale moves both dates by seven days.
    const box = (await bar.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 14 * 7, box.y + box.height / 2, { steps: 6 });
    await page.mouse.up();
    await expect.poll(async () => (await api.ok("GET", `/api/v1/items/${item.key}`)).start_date).toBe(iso(9));
    // The work item's history names the fields, the person and the values.
    await page.goto(`/i/${item.key}`);
    await page.getByRole("tab", { name: /History/ }).click();
    const history = page.getByRole("tabpanel").or(page.locator(".history"));
    await expect(history.getByText(/due date/i).first()).toBeVisible();
    await expect(history.getByText("Morgan Reyes").first()).toBeVisible();
    await api.dispose();
  });

  test("a saved chart carries its owner's lines, spans and groups, and only the owner changes them", async ({ page, browser, baseURL }) => {
    await signInAs(page, ADMIN);
    await page.goto("/planning/gantt");
    await page.getByRole("link", { name: /Reliability plan/ }).click();
    await expect(page.getByLabel("Name of the chart")).toHaveValue("Reliability plan");
    const legend = page.getByRole("list", { name: "Lines, spans and groups on this chart" });
    await expect(legend).toContainText("Site B shutdown");
    await expect(legend).toContainText("Reset fix");
    // The owner adds a marker line; it is saved and listed.
    await page.getByRole("button", { name: /Lines, spans and groups/ }).click();
    await page.getByRole("button", { name: "Marker line" }).click();
    const label = `Audit ${uniq()}`;
    await page.getByLabel("Label").last().fill(label);
    await expect(legend).toContainText(label);
    const id = page.url().split("/").pop()!;
    const api = await Api.as(baseURL!, ADMIN);
    await expect.poll(async () => JSON.stringify((await api.ok("GET", `/api/v1/gantt/${id}`)).chart.annotations)).toContain(label);
    // Someone else sees the shared chart with the cues, and has no controls to change them.
    const ctx = await browser.newContext();
    const other = await ctx.newPage();
    await signInAs(other, MEMBER);
    await other.goto(`/planning/gantt/${id}`);
    await expect(other.getByRole("heading", { name: "Reliability plan" })).toBeVisible();
    await expect(other.getByRole("list", { name: "Lines, spans and groups on this chart" })).toContainText(label);
    await expect(other.getByRole("button", { name: /Lines, spans and groups/ })).toHaveCount(0);
    await expect(other.getByRole("button", { name: "Save a copy of my own" })).toBeVisible();
    const member = await Api.as(baseURL!, MEMBER);
    expect((await member.call("PATCH", `/api/v1/gantt/${id}`, { annotations: [] })).status()).toBe(403);
    await member.dispose();
    await api.dispose();
    await ctx.close();
  });

  test("the hierarchy graph shows status and who has each node, and opens details in place", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/planning/graph");
    const program = page.getByRole("button", { name: /Program\s*Plant modernization/ });
    await expect(program).toBeVisible();
    await expect(program).toContainText("Active");
    // Projects start folded on a large graph; unfolding one shows its work.
    await page.getByRole("button", { name: /Show the \d+ beneath PUMP/ }).click();
    const agentWork = page.getByRole("button", { name: /PUMP-9/ });
    await expect(agentWork).toBeVisible();
    // The work an AI agent has is marked as such, in words as well as with the robot mark.
    await expect(agentWork.getByRole("img", { name: /Triage agent \(AI agent\)/ })).toBeVisible();
    await agentWork.click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toContainText("Display flickers below 5 °C");
    await expect(dialog).toContainText("AI agent");
    await dialog.getByRole("link", { name: "Open PUMP-9" }).click();
    await expect(page).toHaveURL(/\/i\/PUMP-9/);
  });

  test("the graph follows the work as it changes, without a reload", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    await signInAs(page, ADMIN);
    await page.goto("/projects/PUMP?view=graph");
    await expect(page.getByRole("button", { name: /PUMP-8/ })).toBeVisible();
    const title = `Check the relay board ${uniq()}`;
    const made = await api.ok("POST", "/api/v1/projects/PUMP/items", { title });
    await expect(page.getByRole("button", { name: new RegExp(made.key) })).toContainText(title, { timeout: 15_000 });
    await api.ok("DELETE", `/api/v1/items/${made.key}`);
    await expect(page.getByRole("button", { name: new RegExp(`${made.key}\\b`) })).toHaveCount(0, { timeout: 15_000 });
    await api.dispose();
  });

  test("levels above projects can be renamed, and a program reports on what is beneath it", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/planning/levels");
    await expect(page.getByLabel("Name of level 1").first()).toHaveValue("Program");
    await page.goto("/planning");
    await page.getByRole("link", { name: "Plant modernization" }).click();
    await expect(page.getByRole("heading", { level: 1 })).toContainText("Plant modernization");
    await page.getByRole("tab", { name: "Report" }).click();
    await expect(page.getByText("Who is doing the work")).toBeVisible();
    const people = page.getByRole("table").filter({ hasText: "Finished, 30 days" });
    await expect(people.getByRole("row").filter({ hasText: "Triage agent" })).toContainText("AI agent");
    await expect(page.getByRole("table", { name: "Work items by project" })).toContainText("Pump controller firmware");
  });

  test("a project designs its own workflow in the visual editor", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const key = `F${uniq()}`;
    await api.ok("POST", "/api/v1/projects", { key, name: `Flow ${key}`, template: "basic" });
    await signInAs(page, ADMIN);
    await page.goto(`/projects/${key}?view=settings`);
    await page.getByRole("button", { name: "Design its own" }).first().click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByText(/Saving gives this project its own copy/)).toBeVisible();
    // Add a status, then allow a move into it, using the lists (everything on the canvas is there too).
    await dialog.getByLabel("Name of a new status").fill("Waiting for parts");
    await dialog.getByRole("button", { name: "Add status" }).click();
    await dialog.getByText(/The same workflow as lists/).click();
    await expect(dialog.getByLabel("Name of Waiting for parts")).toHaveValue("Waiting for parts");
    await dialog.getByLabel("A new move from").selectOption({ label: "Any status" });
    await dialog.getByLabel("A new move to").selectOption({ label: "Waiting for parts" });
    await dialog.getByRole("button", { name: "Allow this move" }).click();
    await dialog.getByRole("button", { name: "Save workflow" }).click();
    await expect(page.getByText("Own workflow").first()).toBeVisible();
    const detail = await api.ok("GET", `/api/v1/projects/${key}`);
    const changed = detail.types.find((t: any) => t.workflow.own);
    expect(changed.workflow.statuses.map((s: any) => s.name)).toContain("Waiting for parts");
    expect(changed.workflow.transitions.some((t: any) => t.to === "waiting_for_parts" && t.from.includes("*"))).toBe(true);
    // A new item of that type can now make the move.
    const item = await api.ok("POST", `/api/v1/projects/${key}/items`, { title: "Needs a part", type: changed.key });
    await page.goto(`/i/${item.key}`);
    await page.getByRole("group", { name: "Change status" }).getByRole("button", { name: "Waiting for parts" }).click();
    await expect.poll(async () => (await api.ok("GET", `/api/v1/items/${item.key}`)).status).toBe("waiting_for_parts");
    await api.dispose();
  });
});

test.describe("finding work by its fields", () => {
  test("criteria narrow the list, columns sort it, and an item opens beside it", async ({ page, viewport }) => {
    await signInAs(page, ADMIN);
    await page.goto("/search");
    await expect(page.getByRole("tab", { name: "Find work" })).toHaveAttribute("aria-selected", "true");
    await page.getByLabel("Add a condition on a field").selectOption({ label: "Project" });
    await page.getByLabel("Add to Project").selectOption({ label: "PUMP · Pump controller firmware" });
    await page.getByLabel("Add a condition on a field").selectOption({ label: "Assignee" });
    await page.getByLabel("Add to Assignee").selectOption({ label: "Triage agent (AI agent)" });
    await page.locator(".find-form").getByRole("button", { name: "Search", exact: true }).click();
    const results = page.locator(".find-results tbody tr");
    await expect(results).toHaveCount(1);
    await expect(results.first()).toContainText("PUMP-9");
    await expect(page.getByRole("status").filter({ hasText: "1 match" })).toBeVisible();

    // Widen: anything in PUMP with a due date; sort by the Key column, both ways.
    await page.getByRole("button", { name: "Remove the line about Assignee" }).click();
    await page.getByLabel("Add a condition on a field").selectOption({ label: "Due date" });
    await page.getByLabel("How Due date is compared").selectOption({ label: "is not empty" });
    await page.locator(".find-form").getByRole("button", { name: "Search", exact: true }).click();
    await expect(results).toHaveCount(10);
    await page.getByRole("button", { name: "Key", exact: true }).click();
    await expect(results.first()).toContainText("PUMP-1");
    await expect(page.getByRole("columnheader", { name: /Key/ })).toHaveAttribute("aria-sort", "ascending");
    await page.getByRole("button", { name: "Key", exact: true }).click();
    await expect(results.first()).toContainText("PUMP-10");

    // Open one beside the list: the address stays on the search, the list keeps its place.
    await page.getByRole("button", { name: /Controller resets under sustained load: open beside the list/ }).click();
    const pane = page.getByRole("complementary", { name: /^Work item / });
    await expect(pane).toHaveAccessibleName("Work item PUMP-2");
    await expect(pane.getByRole("heading", { name: "Description" })).toBeVisible();
    await expect(page).toHaveURL(/\/search\?.*open=PUMP-2/);
    if (viewport && viewport.width > 1100) await expect(results.filter({ hasText: "PUMP-2" }).first()).toHaveAttribute("aria-selected", "true");
    await pane.getByRole("button", { name: "Next in the list" }).click();
    await expect(page).toHaveURL(/open=PUMP-1\b/);
    await expect(pane).toHaveAccessibleName("Work item PUMP-1");
    await pane.getByRole("button", { name: "Close and return to the list" }).click();
    await expect(results).toHaveCount(10);
  });

  test("a search can be saved, shared and opened again; others see only what they may", async ({ page, baseURL }) => {
    const name = `Late in pump ${uniq()}`;
    await signInAs(page, ADMIN);
    await page.goto("/search?view=find");
    await page.getByLabel("Add a condition on a field").selectOption({ label: "Overdue" });
    await page.getByRole("button", { name: "Save as…" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("Name").fill(name);
    await dialog.getByLabel("Share with the workspace").check();
    await dialog.getByRole("button", { name: "Save", exact: true }).click();
    await expect(page.getByText(`Saved search ${name}`)).toBeVisible();
    await expect(page).toHaveURL(/saved=/);
    const saved = new URL(page.url()).searchParams.get("saved")!;
    // A link to the saved search alone runs it.
    await page.goto(`/search?saved=${saved}`);
    await expect(page.locator(".find-results tbody tr").first()).toBeVisible();
    await expect(page.locator(".find-results")).toContainText("PUMP-2");
    // Another member finds the shared search in their list, marked as someone else's.
    const api = await Api.as(baseURL!, MEMBER);
    const list = await api.ok("GET", "/api/v1/searches");
    expect(list.some((s: any) => s.id === saved && !s.mine)).toBe(true);
    await api.dispose();
  });
});
