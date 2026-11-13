import { expect, test } from "@playwright/test";
import { ADMIN, Api, MEMBER, PASSWORD, signInAs, uniq } from "./helpers";

test.describe("people, AI agents and teams", () => {
  test("an AI agent is marked as one wherever it is named", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    await signInAs(page, ADMIN);
    // The directory, with a filter for the kind of account.
    await page.goto("/people");
    const card = page.getByRole("link", { name: /Triage agent/ });
    await expect(card).toContainText("AI agent");
    await page.getByRole("group", { name: "Kind of account" }).getByRole("button", { name: /People/ }).click();
    await expect(page.getByRole("link", { name: /Triage agent/ })).toHaveCount(0);
    await expect(page.getByRole("link", { name: /Ada Okafor/ })).toBeVisible();
    // Assignee on a work item, the assignee picker, and the profile.
    await page.goto("/i/PUMP-9");
    await expect(page.locator('[data-field="assignee"]')).toContainText("AI agent");
    await expect(page.getByLabel("Assignee").locator("option", { hasText: "Triage agent (AI agent)" })).toHaveCount(1);
    await expect(page.locator('[data-field="assignee"]').getByRole("img", { name: /Triage agent \(AI agent\)/ })).toBeVisible();
    // A message the agent writes carries the mark; a mention of it does too.
    const agents = await api.ok("GET", "/api/v1/users?kind=agent");
    const agent = agents.find((a: any) => a.name === "Triage agent");
    const note = `Triage note ${uniq()}`;
    const token = await api.ok("POST", `/api/v1/admin/users/${agent.id}/tokens`, { name: `e2e ${uniq()}`, scopes: ["read", "write"] });
    const posted = await page.request.post(`/api/v1/items/PUMP-9/comments`, { headers: { Authorization: `Bearer ${token.token}`, Cookie: "" }, data: { body: note } });
    expect(posted.status(), await posted.text()).toBeLessThan(300);
    await api.ok("POST", "/api/v1/items/PUMP-9/comments", { body: `@${agent.handle} thank you` });
    await page.reload();
    const message = page.getByRole("article", { name: "Message from Triage agent" }).filter({ hasText: note });
    await expect(message).toContainText("AI agent");
    await expect(page.locator("a.mention.agent").first()).toHaveAttribute("title", "Triage agent (AI agent)");
    await page.goto(`/people/${agent.id}`);
    await expect(page.getByRole("heading", { level: 1 })).toContainText("Triage agent");
    await expect(page.getByText("AI agent").first()).toBeVisible();
    await api.dispose();
  });

  test("a new AI agent can do nothing until an administrator writes its rules", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const name = `Planner bot ${uniq()}`;
    await signInAs(page, ADMIN);
    await page.goto("/admin/users");
    await page.getByRole("button", { name: /New AI agent|Add AI agent|New agent/ }).click();
    await page.getByRole("dialog").getByLabel("Name").fill(name);
    await page.getByRole("dialog").getByRole("button", { name: /Create/ }).click();
    // Creating an agent leads straight to its rules, which allow nothing.
    await expect(page.getByRole("heading", { name: new RegExp(`Rules for ${name}`) })).toBeVisible();
    await expect(page.getByRole("checkbox", { checked: true })).toHaveCount(0);
    const id = page.url().split("/").pop()!;
    const token = await api.ok("POST", `/api/v1/admin/users/${id}/tokens`, { name: "e2e", scopes: ["read", "write"] });
    const asAgent = (path: string, init: { method?: string; data?: unknown } = {}) => page.request.fetch(path, { method: init.method ?? "GET", data: init.data, headers: { Authorization: `Bearer ${token.token}`, Cookie: "" } });
    // It can sign in to the API, and that is all: lists come back empty, everything else is refused.
    expect(await (await asAgent("/api/v1/projects")).json()).toEqual([]);
    expect((await (await asAgent("/api/v1/items?limit=5")).json()).total).toBe(0);
    expect((await asAgent("/api/v1/items/PUMP-2")).status()).toBe(404);
    expect((await asAgent("/api/v1/users")).status()).toBe(403);
    expect((await asAgent("/api/v1/projects", { method: "POST", data: { key: "BOT", name: "Bot project", template: "basic" } })).status()).toBe(403);
    // Allow reading, in one project only.
    await page.getByRole("button", { name: "Read", exact: true }).click();
    await page.getByRole("group", { name: "Projects" }).getByLabel(/PUMP/).check();
    await page.getByRole("button", { name: "Save rules" }).click();
    await expect(page.getByText(/Rules saved|saved/i).first()).toBeVisible();
    await api.ok("POST", "/api/v1/projects/PUMP/members", { user_id: id, role: "project_member" });
    await api.ok("POST", "/api/v1/projects/ORB/members", { user_id: id, role: "project_member" });
    expect((await asAgent("/api/v1/items/PUMP-2")).status()).toBe(200);
    expect((await asAgent("/api/v1/items/ORB-1")).status()).toBe(404); // a role there, but the rules do not include that project
    expect((await asAgent("/api/v1/items/PUMP-2/comments", { method: "POST", data: { body: "May I?" } })).status()).toBe(403);
    expect((await asAgent("/api/v1/items/PUMP-2", { method: "PATCH", data: { title: "Changed by the agent" } })).status()).toBe(403);
    // The refusals are on record.
    await page.goto("/admin/security");
    await expect(page.getByRole("table")).toContainText(name);
    await api.dispose();
  });

  test("a profile has a picture or a pattern, organization details, and a status others see", async ({ page, baseURL }) => {
    const email = `quinn-${uniq().toLowerCase()}@example.org`;
    const admin = await Api.as(baseURL!, ADMIN);
    const made = await admin.ok("POST", "/api/v1/admin/users", { email, name: "Quinn Abara", password: PASSWORD, role: "member" });
    await signInAs(page, email);
    await page.goto("/profile");
    // Without an upload, the picture is a pattern generated for the account.
    const picture = page.getByRole("img", { name: "Quinn Abara" }).first().locator("img");
    await expect(picture).toHaveAttribute("src", new RegExp(`/api/v1/users/${made.id}/avatar`));
    expect((await page.request.get(`/api/v1/users/${made.id}/avatar`)).headers()["content-type"]).toContain("image/svg+xml");
    await expect(page.getByText(/A square picture of about \d+ × \d+ pixels works best/)).toBeVisible();

    // A picture that is too large is refused with the limit in the message; a small one is taken.
    const large = await page.screenshot({ fullPage: false });
    await page.locator('input[type="file"]').first().setInputFiles({ name: "wide.png", mimeType: "image/png", buffer: large });
    await expect(page.getByText(/at most \d+ pixels|too large|larger than/i).last()).toBeVisible();
    const small = await page.getByRole("button", { name: "Upload a picture" }).screenshot();
    await page.locator('input[type="file"]').first().setInputFiles({ name: "me.png", mimeType: "image/png", buffer: small });
    await expect(page.getByRole("button", { name: "Remove" })).toBeVisible();
    expect((await page.request.get(`/api/v1/users/${made.id}/avatar`)).headers()["content-type"]).toContain("image/png");

    // Organization and job title, where the administrator lets people set them.
    await page.getByLabel("Job title").fill("Reliability engineer");
    await page.getByLabel("Organization", { exact: true }).fill("Northfield Controls");
    await expect(page.getByLabel("Organization code")).toBeDisabled(); // set by an administrator here
    await page.getByRole("button", { name: "Save profile" }).click();
    await expect.poll(async () => (await admin.ok("GET", `/api/v1/users/${made.id}`)).job_title).toBe("Reliability engineer");
    await admin.ok("PATCH", `/api/v1/admin/users/${made.id}`, { org_code: "NFC-REL" });

    // Status: away, with a message. Others see it in the directory.
    await page.getByLabel("I am").selectOption("away");
    await page.getByLabel("Message", { exact: true }).fill("At the test stand until two");
    await page.getByRole("button", { name: "Set status" }).click();
    await expect.poll(async () => (await admin.ok("GET", `/api/v1/users/${made.id}`)).presence).toBe("away");
    const other = await page.context().browser()!.newContext();
    const viewer = await other.newPage();
    await signInAs(viewer, MEMBER);
    await viewer.goto(`/people/${made.id}`);
    await expect(viewer.getByText("At the test stand until two")).toBeVisible();
    await expect(viewer.getByText("Away").first()).toBeVisible();
    await expect(viewer.getByText("NFC-REL")).toBeVisible();
    await expect(viewer.getByText("Reliability engineer").first()).toBeVisible();
    await other.close();
    await admin.dispose();
  });

  test("a team is its positions: a lead adds one, fills it, and the history and the org chart follow", async ({ page, baseURL }) => {
    const name = `Commissioning ${uniq()}`;
    await signInAs(page, ADMIN);
    await page.goto("/teams");
    await page.getByRole("button", { name: "New team" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("Name").fill(name);
    await dialog.getByLabel("Part of").selectOption({ label: "Engineering" });
    await dialog.getByRole("button", { name: "Create team" }).click();
    await expect(page.getByRole("heading", { name: new RegExp(name), level: 1 })).toBeVisible();
    // Whoever creates a team leads it.
    await expect(page.locator(".position.lead")).toContainText("Morgan Reyes");

    await page.getByRole("button", { name: "Add position" }).click();
    await page.getByRole("dialog").getByLabel("Title").fill("Site engineer");
    await page.getByRole("dialog").getByRole("button", { name: /Add position|Save/ }).click();
    const position = page.locator(".position").filter({ hasText: "Site engineer" });
    await expect(position).toContainText("Open: nobody is in this position.");
    // Filled by a person, counted from a date in the past, and by an AI agent as well.
    await position.getByRole("button", { name: "Add someone" }).click();
    await page.getByRole("dialog").getByLabel("Who").selectOption({ label: "Chen Wei" });
    await page.getByRole("dialog").getByLabel("In this position since").fill("2025-03-01");
    await page.getByRole("dialog").getByRole("button", { name: /Add|Save/ }).last().click();
    await expect(position).toContainText("Chen Wei");
    await expect(position).toContainText("since Mar 1, 2025");
    await position.getByRole("button", { name: "Add someone" }).click();
    await page.getByRole("dialog").getByLabel("Who").selectOption({ label: "Triage agent (AI agent)" });
    await page.getByRole("dialog").getByRole("button", { name: /Add|Save/ }).last().click();
    await expect(position.getByText("AI agent")).toBeVisible();

    // The person's profile carries the position and how long they have held it.
    await position.getByRole("link", { name: /Chen Wei/ }).click();
    await expect(page.getByRole("heading", { level: 1 })).toContainText("Chen Wei");
    const held = page.locator("main").getByText("Site engineer").first();
    await expect(held).toBeVisible();
    await expect(page.locator("main")).toContainText(name);
    await expect(page.locator("main")).toContainText(/1 year/);

    // The org chart shows the new team beneath its parent.
    await page.goto("/teams/chart");
    const chart = page.getByRole("region", { name: "Org chart" });
    await expect(chart.getByRole("link", { name }).first()).toBeVisible();
    await expect(chart).toContainText("Site engineer");

    // Someone who does not lead the team can look, not change.
    const member = await Api.as(baseURL!, MEMBER);
    const teams = await member.ok("GET", "/api/v1/teams");
    const team = (teams.teams ?? teams).find((t: any) => t.name === name);
    expect((await member.call("POST", `/api/v1/teams/${team.id}/positions`, { title: "Shadow lead", is_lead: true })).status()).toBe(403);
    await member.dispose();
  });
});
