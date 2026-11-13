import { expect, test } from "@playwright/test";
import { ADMIN, Api, CUSTOMER, signInAs, uniq } from "./helpers";

test.describe("service desk", () => {
  test("a customer raises a request, an agent answers, and internal notes stay internal", async ({ browser, baseURL }) => {
    const subject = `Badge reader offline ${uniq()}`;
    const note = `Vendor ticket ${uniq()} opened`;

    // Customer side.
    const customerCtx = await browser.newContext();
    const customer = await customerCtx.newPage();
    await signInAs(customer, CUSTOMER);
    await customer.goto("/portal");
    await customer.getByRole("link", { name: "Raise a request" }).first().click();
    await customer.getByRole("button", { name: /Something is broken/ }).click();
    await customer.getByLabel("Summary").fill(subject);
    await customer.getByLabel("Details").fill("The reader at door 3 shows a red light and does not react to badges.");
    await customer.getByRole("button", { name: "Send request" }).click();
    await expect(customer.getByRole("heading", { name: subject })).toBeVisible();
    const requestKey = customer.url().split("/").pop()!;

    // Agent side: the request is in the queue with its clock, and gets a reply and a private note.
    const agentCtx = await browser.newContext();
    const agent = await agentCtx.newPage();
    await signInAs(agent, ADMIN);
    await agent.goto("/projects/IT?view=queues");
    await agent.getByRole("link", { name: new RegExp(subject) }).click();
    await expect(agent).toHaveURL(new RegExp(`/i/${requestKey}`));
    await expect(agent.getByText("Resolution", { exact: true })).toBeVisible();
    await agent.getByRole("textbox", { name: "Reply (Markdown)" }).fill("We are sending a technician this afternoon.");
    await agent.getByRole("button", { name: "Send reply" }).click();
    await expect(agent.getByText("We are sending a technician this afternoon.")).toBeVisible();
    await agent.getByLabel(/Internal note/).check();
    await agent.getByRole("textbox", { name: "Internal note (Markdown)" }).fill(note);
    await agent.getByRole("button", { name: "Add internal note" }).click();
    await expect(agent.locator(".comment.internal")).toContainText(note);

    // The customer sees the reply and never the note: not on the page, not through the API.
    await customer.reload();
    await expect(customer.getByText("We are sending a technician this afternoon.")).toBeVisible();
    await expect(customer.getByText(note)).toHaveCount(0);
    // The list of requests names each status the way the desk does, not by its internal key.
    await customer.goto("/portal");
    const listed = customer.getByRole("link", { name: new RegExp(subject) });
    await expect(listed).toContainText("Open");
    await expect(listed).not.toContainText("open");
    const viaApi = await (await customer.request.get(`/api/v1/portal/requests/${requestKey}`)).text();
    expect(viaApi).not.toContain(note);
    expect((await customer.request.get(`/api/v1/items/${requestKey}`)).status()).toBe(404);

    // Another customer cannot open it at all.
    const api = await Api.as(baseURL!, ADMIN);
    const email = `visitor-${uniq().toLowerCase()}@example.org`;
    await api.ok("POST", "/api/v1/admin/users", { email, name: "Visiting Customer", password: "correct horse battery staple", role: "customer" });
    const other = await Api.as(baseURL!, email);
    expect((await other.call("GET", `/api/v1/portal/requests/${requestKey}`)).status()).toBe(404);
    await other.dispose();
    await api.dispose();
    await customerCtx.close();
    await agentCtx.close();
  });
});
