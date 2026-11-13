import { expect, test } from "@playwright/test";
import { ADMIN, Api, MEMBER, signInAs } from "./helpers";

test.describe("when something goes wrong", () => {
  test("an address that does not exist explains itself and says how to get help", async ({ page }) => {
    await signInAs(page, MEMBER);
    await page.goto("/there/is/nothing/here");
    await expect(page.getByRole("heading", { name: "This does not exist, or you cannot see it" })).toBeVisible();
    await expect(page.getByText("Error 404")).toBeVisible();
    await expect(page.getByRole("heading", { name: "What you can do" })).toBeVisible();
    // The support contact an administrator configured.
    const help = page.getByRole("region", { name: "Getting help" });
    await expect(help).toContainText("Operations desk");
    await expect(help.getByRole("link", { name: "ops-desk@example.org" })).toHaveAttribute("href", "mailto:ops-desk@example.org");
    await page.getByRole("link", { name: "Go to my work" }).click();
    await expect(page.getByRole("heading", { name: "My work" })).toBeVisible();
    // Every code has its page.
    await page.goto("/status");
    for (const code of ["400", "401", "403", "404", "409", "413", "422", "429", "500", "502", "503", "504"]) await expect(page.getByRole("link", { name: new RegExp(`^${code}\\b`) })).toBeVisible();
    await page.getByRole("link", { name: /^429/ }).click();
    await expect(page.getByRole("heading", { name: "Too many requests" })).toBeVisible();
    await expect(page.getByText("Wait a minute, then try again.")).toBeVisible();
  });

  test("a failing address opened directly gets a readable page from the server, with the status kept", async ({ page }) => {
    await signInAs(page, MEMBER);
    const response = await page.goto("/api/v1/items/NOPE-404");
    expect(response!.status()).toBe(404);
    expect(response!.headers()["content-type"]).toContain("text/html");
    await expect(page.getByRole("heading", { name: "This does not exist, or you cannot see it" })).toBeVisible();
    await expect(page.getByText("Operations desk")).toBeVisible();
    await expect(page.getByText(/Reference: \w+/)).toBeVisible();
    // Something the account may not do reads differently from something that is not there.
    const forbidden = await page.goto("/api/v1/admin/settings");
    expect(forbidden!.status()).toBe(403);
    await expect(page.getByRole("heading", { name: "You do not have access to this" })).toBeVisible();
    // Signed out: an offer to sign in, and nothing about what is behind the address.
    await page.context().clearCookies();
    const anonymous = await page.goto("/api/v1/items/PUMP-2");
    expect(anonymous!.status()).toBe(401);
    await expect(page.getByRole("link", { name: "Sign in" })).toBeVisible();
    await expect(page.locator("body")).not.toContainText("Controller resets");
  });

  test("a view that fails while drawing shows an explanation instead of a blank page, and navigation still works", async ({ page }) => {
    await signInAs(page, ADMIN);
    // Answer one request with something no page expects.
    await page.route("**/api/v1/projects/PUMP", (route) => route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ project: null, types: 7 }) }));
    await page.goto("/projects/PUMP");
    await expect(page.getByRole("heading", { name: "This page ran into a problem" })).toBeVisible();
    await expect(page.getByText("Your work on the server is not affected.")).toBeVisible();
    await page.unroute("**/api/v1/projects/PUMP");
    await page.getByLabel("Main navigation").getByRole("link", { name: "Planning" }).click();
    await expect(page.getByRole("heading", { name: "Planning", level: 1 })).toBeVisible();
  });

  test("a slow page shows its outline first, then says what it is waiting for", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.clock.install();
    let release: () => void = () => undefined;
    const held = new Promise<void>((r) => (release = r));
    await page.route("**/api/v1/containers", async (route) => {
      await held;
      await route.continue();
    });
    await page.goto("/planning");
    // First the outline of what is coming, with no words to read.
    // (Wait for the page's own outline, not the one shown while the page's code is fetched,
    // so that the clock below is moved for the part that is being timed.)
    const outline = page.locator('[aria-busy="true"][aria-label="Loading the hierarchy"]');
    await expect(outline).toBeVisible();
    await expect(page.getByRole("status").filter({ hasText: "Loading the hierarchy" })).toHaveCount(0);
    // Past ten seconds: a progress bar and a message that says what is being loaded.
    await page.clock.fastForward(11_000);
    const status = page.getByRole("status").filter({ hasText: "Loading the hierarchy…" });
    await expect(status).toBeVisible();
    await expect(status).toContainText("This is taking longer than usual.");
    await expect(status.locator(".progress")).toBeVisible();
    await page.clock.fastForward(55_000);
    await expect(status.getByRole("button", { name: "Reload the page" })).toBeVisible();
    // When the answer arrives, the page takes over.
    release();
    await expect(page.getByRole("link", { name: "Plant modernization" })).toBeVisible();
    await expect(status).toHaveCount(0);
  });
});

test.describe("what the application says about itself", () => {
  test("the statements can be read before signing in", async ({ page, baseURL }) => {
    const api = await Api.as(baseURL!, ADMIN);
    await api.ok("PUT", "/api/v1/admin/settings/legal", { privacy: "We keep **as little** as the work needs." });
    await page.goto("/login");
    await page.getByRole("navigation", { name: "About this site" }).getByRole("link", { name: "Privacy" }).click();
    await expect(page.getByRole("heading", { name: "Privacy", level: 1 })).toBeVisible();
    // The organization's own statement, then what the software itself does.
    await expect(page.locator(".prose strong")).toHaveText("as little");
    await expect(page.getByRole("heading", { name: "What this application stores about you" })).toBeVisible();
    await page.getByRole("link", { name: "Cookies" }).click();
    await expect(page.getByText(/One cookie, named gantry_session/)).toBeVisible();
    await expect(page.getByText("No advertising, analytics or tracking cookies.")).toBeVisible();
    await page.getByRole("link", { name: "Data retention" }).click();
    await expect(page.getByText(/Your organization has not written its own statement/)).toBeVisible();
    await expect(page.getByText("Notifications in your inbox: 90 days.")).toBeVisible();
    // Versions and components are for people who are signed in.
    await expect(page.getByRole("link", { name: "Licenses" })).toHaveCount(0);
    expect((await page.request.get("/api/v1/about/components")).status()).toBe(401);
    expect((await (await page.request.get("/api/v1/about")).json()).product.version).toBeUndefined();
    await api.ok("PUT", "/api/v1/admin/settings/legal", { privacy: "" });
    await api.dispose();
  });

  test("components, add-ons and the licenses of everything it is built from", async ({ page }) => {
    await signInAs(page, MEMBER);
    await page.getByLabel("Main navigation").getByRole("link", { name: "About" }).click();
    await expect(page.getByRole("heading", { name: /^About / })).toBeVisible();
    const components = page.getByRole("table").filter({ hasText: "What it is" });
    for (const part of ["Gantry server", "Web application", "Manual"]) await expect(components.getByRole("row").filter({ hasText: part })).toContainText(/\d+\.\d+\.\d+/);
    const addons = page.getByRole("table").filter({ hasText: "Installed" });
    await expect(addons.getByRole("row").filter({ hasText: "NASA project management" })).toContainText(/\d+\.\d+\.\d+/);
    await page.getByRole("navigation", { name: "About this application" }).getByRole("link", { name: "Licenses" }).click();
    await expect(page.getByText(/built from [\d,]+ other open-source packages/)).toBeVisible();
    await page.getByLabel("Find a package or a license").fill("axum");
    const row = page.locator(".license-list li").filter({ hasText: /^axum\b/ }).first();
    await expect(row).toContainText("MIT");
    await row.getByRole("button", { name: "License text" }).click();
    await expect(row.locator(".license-text")).toContainText("Permission is hereby granted");
  });

  test("links to the site carry a title, a description and a picture", async ({ page }) => {
    await page.goto("/login");
    const meta = (name: string) => page.locator(`meta[property="${name}"], meta[name="${name}"]`).first().getAttribute("content");
    expect(await meta("og:title")).toBe("Gantry");
    expect(await meta("og:description")).toMatch(/service desk/);
    expect(await meta("twitter:card")).toBe("summary_large_image");
    const image = (await meta("og:image"))!;
    const picture = await page.request.get(image);
    expect(picture.status()).toBe(200);
    expect(picture.headers()["content-type"]).toBe("image/png");
    for (const icon of ["/favicon.svg", "/favicon.ico", "/apple-touch-icon.png", "/icon-192.png", "/icon-512.png", "/site.webmanifest"]) expect((await page.request.get(icon)).status(), icon).toBe(200);
    const manifest = await (await page.request.get("/site.webmanifest")).json();
    expect(manifest.icons.length).toBeGreaterThanOrEqual(3);
    // The manual has its own.
    await page.goto("/manual/");
    expect(await page.locator('link[rel="icon"]').first().getAttribute("href")).toBeTruthy();
    expect(await page.locator('meta[property="og:image"]').getAttribute("content")).toMatch(/og\.png$/);
  });
});
