import { expect, test } from "@playwright/test";
import { ADMIN, Api, MEMBER, signInAs, uniq } from "./helpers";

// Two people at once: what one does, the other sees without reloading.
test.describe("working at the same time", () => {
  test("presence, live discussion, mentions and notifications on a work item", async ({ browser, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const item = await api.ok("POST", "/api/v1/projects/PUMP/items", { title: `Bench test the relay ${uniq()}` });
    const one = await browser.newContext(), two = await browser.newContext();
    const morgan = await one.newPage(), ada = await two.newPage();
    await signInAs(morgan, ADMIN);
    await signInAs(ada, MEMBER);
    await morgan.goto(`/i/${item.key}`);
    await ada.goto(`/i/${item.key}`);
    // Each sees that the other has the item open.
    await expect(morgan.getByRole("status", { name: /Also here: Ada Okafor/ })).toBeVisible({ timeout: 15_000 });
    await expect(ada.getByRole("status", { name: /Also here: Morgan Reyes/ })).toBeVisible({ timeout: 15_000 });

    // Ada starts writing: Morgan is told. She mentions Morgan, links another item and a date.
    const box = ada.getByRole("textbox", { name: "Message (Markdown)" });
    await box.click();
    await ada.keyboard.type("Ready for review, ");
    await expect(morgan.getByText("Ada Okafor is writing…")).toBeVisible({ timeout: 10_000 });
    // "@" offers the people who can open this project; Enter takes the first match.
    await ada.keyboard.type("@mor");
    await expect(ada.getByRole("listbox").getByRole("option", { name: /Morgan Reyes/ })).toHaveAttribute("aria-selected", "true");
    // The list ignores keys for an instant after it opens, so that Enter typed at speed stays a new line.
    await ada.waitForTimeout(200);
    await ada.keyboard.press("Enter");
    await expect(box).toContainText("@morgan");
    await ada.keyboard.type(" see PUMP-2 and PUMP-9999 by ");
    // The date comes from the calendar, not from typing.
    await ada.getByRole("button", { name: "Insert a date" }).click();
    const calendar = ada.getByRole("dialog", { name: "Insert a date" });
    await calendar.getByRole("button", { name: "Next month" }).click();
    await calendar.getByRole("gridcell", { name: /\b15,/ }).click();
    await expect(box).toContainText(/\[\[\d{4}-\d{2}-15\]\]/);
    await ada.getByRole("button", { name: "Post message" }).click();

    // Morgan's page shows the message without a reload, rendered, with what Morgan may see linked.
    const message = morgan.getByRole("article", { name: "Message from Ada Okafor" });
    await expect(message).toContainText("Ready for review", { timeout: 15_000 });
    await expect(message.locator("a.mention")).toHaveText("@Morgan Reyes");
    await expect(message.getByRole("link", { name: "PUMP-2" })).toHaveAttribute("href", "/i/PUMP-2");
    await expect(message.getByRole("link", { name: "PUMP-9999" })).toHaveCount(0); // no such item: plain text
    await expect(message.locator("time.chip")).toHaveAttribute("datetime", /-15$/);
    await expect(morgan.getByText("Ada Okafor is writing…")).toHaveCount(0);
    // Being mentioned raises a notification at once.
    await expect(morgan.getByText(/Ada Okafor mentioned Morgan Reyes in a message on/).first()).toBeVisible({ timeout: 15_000 });
    const inbox = await api.ok("GET", "/api/v1/notifications?limit=5");
    expect(JSON.stringify(inbox)).toContain(item.key);

    // Ada corrects her message, then removes it; both are seen by Morgan as they happen.
    const mine = ada.getByRole("article", { name: "Message from Ada Okafor" });
    await mine.getByRole("button", { name: "Message actions" }).click();
    await ada.getByRole("menuitem", { name: "Edit" }).click();
    const edit = mine.getByRole("textbox", { name: "Message (Markdown)" });
    await edit.fill("Ready for review after all.");
    await mine.getByRole("button", { name: "Save" }).click();
    await expect(message).toContainText("Ready for review after all.", { timeout: 15_000 });
    await expect(message).toContainText("edited");
    await mine.getByRole("button", { name: "Message actions" }).click();
    await ada.getByRole("menuitem", { name: "Remove" }).click();
    await ada.getByRole("dialog").getByRole("button", { name: "Remove message" }).click();
    await expect(morgan.getByText("Ready for review after all.")).toHaveCount(0, { timeout: 15_000 });
    await expect(morgan.getByText(/message was removed/i).first()).toBeVisible();
    // The history keeps the record of all of it.
    await morgan.getByRole("tab", { name: /History/ }).click();
    await expect(morgan.getByText(/removed a message/i).first()).toBeVisible();
    await api.dispose();
    await one.close();
    await two.close();
  });

  test("two people edit one description at the same time and both texts survive", async ({ browser, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const item = await api.ok("POST", "/api/v1/projects/PUMP/items", { title: `Shared text ${uniq()}`, description: "First line.\n" });
    const one = await browser.newContext(), two = await browser.newContext();
    const morgan = await one.newPage(), ada = await two.newPage();
    await signInAs(morgan, ADMIN);
    await signInAs(ada, MEMBER);
    for (const p of [morgan, ada]) {
      await p.goto(`/i/${item.key}`);
      await p.getByRole("button", { name: "Edit", exact: true }).click();
      await expect(p.getByRole("textbox", { name: "Description (Markdown)" })).toContainText("First line.");
    }
    // Each is shown where the other is.
    // (Once at the top of the item, saying which part, and once beside that part.)
    await expect(morgan.getByRole("status", { name: /Also here: Ada Okafor/ }).first()).toContainText(/description/i, { timeout: 15_000 });
    await expect(morgan.getByRole("status", { name: /Also here: Ada Okafor/ })).toHaveCount(2);
    const m = morgan.getByRole("textbox", { name: "Description (Markdown)" }), a = ada.getByRole("textbox", { name: "Description (Markdown)" });
    await m.click();
    await morgan.keyboard.press("Control+Home");
    await morgan.keyboard.type("Morgan was here. ");
    await a.click();
    await ada.keyboard.press("Control+End");
    await ada.keyboard.type("Ada was here.");
    // Both editors converge on the same text, with both additions.
    for (const box of [m, a]) {
      await expect(box).toContainText("Morgan was here.", { timeout: 15_000 });
      await expect(box).toContainText("Ada was here.", { timeout: 15_000 });
    }
    // The other person's cursor carries their name.
    await expect(morgan.locator(".cm-ySelectionInfo", { hasText: "Ada Okafor" }).first()).toBeAttached();
    await ada.getByRole("button", { name: "Save description" }).click();
    await expect.poll(async () => (await api.ok("GET", `/api/v1/items/${item.key}`)).description).toMatch(/Morgan was here\. First line\.\s+Ada was here\./);
    await api.dispose();
    await one.close();
    await two.close();
  });

  test("times are shown in the time zone each person chose", async ({ browser, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    const item = await api.ok("POST", "/api/v1/projects/PUMP/items", { title: `Zones ${uniq()}` });
    await api.ok("POST", `/api/v1/items/${item.key}/comments`, { body: "Posted at a known moment." });
    const created = (await api.ok("GET", `/api/v1/items/${item.key}/comments`))[0].created_at as string;
    const member = await Api.as(baseURL!, MEMBER);
    const stampIn = async (zone: string, deviceZone: string) => {
      await member.ok("PATCH", "/api/v1/auth/me", { timezone: zone });
      const ctx = await browser.newContext({ timezoneId: deviceZone, locale: "en-US" });
      const page = await ctx.newPage();
      await signInAs(page, MEMBER);
      await page.goto(`/i/${item.key}`);
      const text = await page.getByRole("article", { name: "Message from Morgan Reyes" }).locator("time").first().innerText();
      await ctx.close();
      return text;
    };
    const expected = (zone: string) => new Intl.DateTimeFormat("en-US", { timeZone: zone, hour: "numeric", minute: "2-digit" }).format(new Date(created));
    // A chosen zone wins over the device's; "auto" follows the device.
    const tokyo = await stampIn("Asia/Tokyo", "America/Chicago");
    expect(tokyo).toContain(expected("Asia/Tokyo"));
    expect(tokyo).toMatch(/GMT\+9|JST/);
    const auto = await stampIn("auto", "America/Chicago");
    expect(auto).toContain(expected("America/Chicago"));
    expect(auto).toMatch(/C[SD]T/);
    await member.ok("PATCH", "/api/v1/auth/me", { timezone: "auto" });
    await member.dispose();
    await api.dispose();
  });
});
