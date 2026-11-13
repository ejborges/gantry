import { expect, test } from "@playwright/test";
import { ADMIN, CUSTOMER, MEMBER, signIn, signInAs } from "./helpers";

test.describe("signing in and out", () => {
  test("a visitor is sent to the sign-in page and sees nothing else", async ({ page }) => {
    for (const path of ["/", "/projects/PUMP", "/i/PUMP-2", "/admin/audit", "/docs/ENG"]) {
      await page.goto(path);
      await expect(page).toHaveURL(/\/login/);
      await expect(page.getByRole("heading", { name: /Sign in to/ })).toBeVisible();
    }
    await expect(page.getByText("Controller resets")).toHaveCount(0);
  });

  test("a wrong password is refused without saying which part was wrong", async ({ page }) => {
    await signIn(page, MEMBER, "not the password");
    const alert = page.getByRole("alert");
    await expect(alert).toBeVisible();
    const known = await alert.innerText();
    await signIn(page, "nobody-here@example.org", "not the password");
    await expect(page.getByRole("alert")).toHaveText(known);
    await expect(page).toHaveURL(/\/login/);
  });

  test("signing in opens My work, and signing out ends the session", async ({ page, context }) => {
    await signInAs(page, MEMBER);
    await expect(page.getByRole("heading", { name: "My work" })).toBeVisible();
    const cookie = (await context.cookies()).find((c) => c.name.includes("gantry"));
    expect(cookie, "session cookie").toBeTruthy();
    expect(cookie!.httpOnly, "the session cookie is hidden from scripts").toBe(true);
    expect(cookie!.sameSite).toBe("Lax");
    expect(await page.evaluate(() => document.cookie)).not.toContain(cookie!.value);

    await page.getByRole("button", { name: /Account menu/ }).click();
    await page.getByRole("menuitem", { name: "Sign out" }).click();
    await expect(page).toHaveURL(/\/login/);
    await page.goto("/projects");
    await expect(page).toHaveURL(/\/login/);
    expect((await page.request.get("/api/v1/auth/me")).status()).toBe(401);
  });

  test("the address asked for before signing in is where you land afterward", async ({ page }) => {
    await page.goto("/i/PUMP-2");
    await expect(page).toHaveURL(/\/login/);
    await page.getByLabel("Email").fill(MEMBER);
    await page.getByLabel("Password").fill("correct horse battery staple");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page).toHaveURL(/\/i\/PUMP-2/);
  });
});

test.describe("what each kind of user is shown", () => {
  test("a member has no administration entry and cannot open its pages", async ({ page }) => {
    await signInAs(page, MEMBER);
    await expect(page.getByLabel("Main navigation").getByRole("link", { name: "Administration" })).toHaveCount(0);
    await page.goto("/admin/users");
    await expect(page.getByRole("heading", { name: "Users and agents" })).toHaveCount(0);
    for (const path of ["/api/v1/admin/users", "/api/v1/admin/audit", "/api/v1/admin/settings", "/api/v1/admin/security-events"]) {
      expect((await page.request.get(path)).status(), path).toBe(403);
    }
  });

  test("a customer sees only the help desk", async ({ page }) => {
    await signInAs(page, CUSTOMER);
    await expect(page).toHaveURL(/\/portal/);
    const nav = page.getByLabel("Main navigation");
    await expect(nav.getByRole("link", { name: "Help desk" })).toBeVisible();
    for (const hidden of ["Projects", "Docs", "Search", "Administration"]) await expect(nav.getByRole("link", { name: hidden })).toHaveCount(0);
    // Internal work stays invisible however the address is reached.
    expect((await page.request.get("/api/v1/items/PUMP-2")).status()).toBe(404);
    const projects: { key: string }[] = await (await page.request.get("/api/v1/projects")).json();
    expect(projects.map((p) => p.key)).not.toContain("PUMP");
    await page.goto("/i/PUMP-2");
    await expect(page.getByText("Controller resets under sustained load")).toHaveCount(0);
  });

  test("an administrator reaches the console", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.getByLabel("Main navigation").getByRole("link", { name: "Administration" }).click();
    await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  });
});
