import { expect, test } from "@playwright/test";
import { ADMIN, Api, MEMBER, OTHER_MEMBER, signInAs, uniq } from "./helpers";

test.describe("projects and work items", () => {
  test("create a project from a template, add an item and move it across the board", async ({ page }) => {
    const key = `E${uniq()}`;
    await signInAs(page, MEMBER);
    await page.goto("/projects?new=1");
    const dialog = page.getByRole("dialog", { name: "New project" });
    await dialog.getByLabel("Name").fill(`Valve retrofit ${key}`);
    await dialog.getByLabel("Key").fill(key);
    await dialog.getByRole("button", { name: /Software delivery/ }).click();
    await dialog.getByRole("button", { name: "Create project" }).click();
    await expect(page).toHaveURL(new RegExp(`/projects/${key}`));
    await expect(page.getByRole("heading", { name: `Valve retrofit ${key}` })).toBeVisible();

    await page.getByRole("button", { name: "New item" }).click();
    const item = page.getByRole("dialog", { name: /New/ });
    await item.getByLabel("Type").selectOption({ label: "Story" });
    await item.getByLabel("Title").fill("Operators can silence an alarm");
    await item.getByLabel("Priority").selectOption({ label: "High" });
    await item.getByRole("button", { name: /Create/ }).click();
    await expect(item).toBeHidden(); // created: leaving the page sooner could cancel the request

    // The software workflow starts in Backlog; the detailed board shows every status.
    await page.goto(`/projects/${key}?view=board`);
    await page.getByLabel("Board").selectOption({ label: /Software delivery workflow/ }).catch(() => undefined);
    const card = page.getByRole("link", { name: /Operators can silence an alarm/ });
    await expect(card).toBeVisible();
    // Without a pointer: a card reached with the Tab key offers a list of where it can go.
    const mover = page.getByLabel(`Move ${key}-1`);
    const width = () => mover.evaluate((el) => el.getBoundingClientRect().width);
    expect(await width(), "out of sight until the keyboard gets there").toBeLessThanOrEqual(1);
    await page.getByLabel("Filter cards").focus();
    for (let i = 0; i < 12 && !(await card.evaluate((el) => el === document.activeElement)); i++) await page.keyboard.press("Tab");
    await expect(card).toBeFocused();
    expect(await width()).toBeGreaterThan(60);
    await page.keyboard.press("Tab");
    await expect(mover).toBeFocused();
    expect(await width()).toBeGreaterThan(60);
    await mover.selectOption({ label: "Plan" });
    // Backlog and To do share the "Not started" column here; the card says which it is in.
    await expect(page.getByRole("listitem", { name: "Not started, 1 item" }).getByRole("link", { name: /Operators can silence an alarm/ })).toContainText("To do");
    await card.click();
    await expect(page).toHaveURL(new RegExp(`/i/${key}-\\d+`));

    const status = page.getByRole("group", { name: "Change status" });
    await status.getByRole("button", { name: "Start" }).click();
    await expect(page.locator("header .badge.in_progress")).toHaveText(/In progress/);

    // The history records who did what.
    await page.getByRole("tab", { name: /History/ }).click();
    await expect(page.getByText(/Ada Okafor/).first()).toBeVisible();
    await expect(page.getByText(/In progress/).first()).toBeVisible();
  });

  test("a workflow rule blocks a step until the required field is filled in", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, MEMBER);
    const key = `R${uniq()}`;
    await api.ok("POST", "/api/v1/projects", { key, name: `Change control ${key}`, template: "engineering-change" });
    const change = await api.ok("POST", `/api/v1/projects/${key}/items`, { type: "change_request", title: "Raise relief valve set point" });
    await signInAs(page, MEMBER);
    await page.goto(`/i/${change.key}`);
    const submit = page.getByRole("group", { name: "Change status" }).getByRole("button", { name: "Submit for review" });
    await expect(submit).toHaveAttribute("title", /Justification/);
    await submit.click();
    // The server explains what is missing and the item stays where it was.
    await expect(page.getByText(/Justification/i).filter({ hasText: /required|needs|before/i }).first()).toBeVisible();
    await expect(page.locator("header .badge").first()).toHaveText(/Draft/);

    await page.getByRole("button", { name: "Edit details" }).click();
    await page.getByRole("textbox", { name: "Justification" }).fill("Nuisance trips at 9.2 bar during normal operation.");
    await page.getByRole("button", { name: "Save details" }).or(page.getByRole("button", { name: "Save", exact: true })).click();
    await submit.click();
    await expect(page.locator("header .badge").first()).toHaveText(/In review/);
    await api.dispose();
  });

  test("edit a title, write a formatted description and comment, and link two items", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, MEMBER);
    const key = `W${uniq()}`;
    await api.ok("POST", "/api/v1/projects", { key, name: `Workshop ${key}`, template: "basic" });
    const a = await api.ok("POST", `/api/v1/projects/${key}/items`, { title: "Order the gasket set" });
    const b = await api.ok("POST", `/api/v1/projects/${key}/items`, { title: "Fit the gasket set" });
    await signInAs(page, MEMBER);
    await page.goto(`/i/${a.key}`);

    const title = page.getByLabel("Title");
    await title.fill("Order the gasket set from the approved supplier");
    await title.blur();
    await expect.poll(async () => (await api.ok("GET", `/api/v1/items/${a.key}`)).title).toBe("Order the gasket set from the approved supplier");

    await page.getByRole("button", { name: "Edit", exact: true }).click();
    await page.getByRole("textbox", { name: "Description (Markdown)" }).fill("Part **GK-204**.\n\n- [x] quote received\n- [ ] purchase order raised\n");
    await page.getByRole("button", { name: "Save description" }).click();
    await expect(page.locator(".prose strong").first()).toHaveText("GK-204");
    const boxes = page.locator('.prose input[type="checkbox"]');
    await expect(boxes).toHaveCount(2);
    await expect(boxes.nth(0)).toBeChecked();
    await expect(boxes.nth(1)).not.toBeChecked();

    await page.getByRole("textbox", { name: "Message (Markdown)" }).fill("Supplier confirmed delivery on *Friday*.");
    await page.getByRole("button", { name: "Post message" }).click();
    await expect(page.locator(".comment em")).toHaveText("Friday");

    await page.getByLabel("Relation").selectOption("blocks");
    await page.getByPlaceholder("Item key, e.g. OPS-12").fill(b.key);
    await page.getByRole("button", { name: "Add link" }).click();
    await expect(page.getByRole("link", { name: new RegExp(`${b.key}.*Fit the gasket set`) })).toBeVisible();
    // From the other end the same link reads the other way round.
    await page.goto(`/i/${b.key}`);
    await expect(page.getByText("blocked by")).toBeVisible();
    await api.dispose();
  });

  test("a private project is invisible to people who are not members", async ({ page, baseURL }) => {
    const owner = await Api.as(baseURL!, MEMBER);
    const key = `P${uniq()}`;
    await owner.ok("POST", "/api/v1/projects", { key, name: `Confidential ${key}`, template: "basic", access: "private" });
    const secret = await owner.ok("POST", `/api/v1/projects/${key}/items`, { title: `Severance terms ${key}` });
    await signInAs(page, OTHER_MEMBER);
    await page.goto("/projects");
    await expect(page.getByText(`Confidential ${key}`)).toHaveCount(0);
    await page.goto(`/i/${secret.key}`);
    await expect(page.getByText(`Severance terms ${key}`)).toHaveCount(0);
    await expect(page.getByText(/does not exist, or you cannot see it/i).first()).toBeVisible();
    await page.goto(`/search?q=${encodeURIComponent(`Severance ${key}`)}`);
    await expect(page.getByText(`Severance terms ${key}`)).toHaveCount(0);
    expect((await page.request.get(`/api/v1/items/${secret.key}`)).status()).toBe(404);
    expect((await page.request.get(`/api/v1/projects/${key}`)).status()).toBe(404);
    await owner.dispose();
  });

  test("search finds work and pages the user may see", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/search?q=controller%20resets");
    await expect(page.getByRole("link", { name: /Controller resets under sustained load/ }).first()).toBeVisible();
    // The command palette does the same from anywhere.
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "My work" })).toBeVisible();
    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: "Search and go to" });
    await palette.getByLabel("Search").fill("capacitor");
    await expect(palette.getByRole("option", { name: /Replace capacitor C14/ })).toBeVisible();
  });
});
