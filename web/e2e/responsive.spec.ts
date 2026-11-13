import { expect, test } from "@playwright/test";
import { ADMIN, expectNoSidewaysScroll, signInAs } from "./helpers";

// Tagged @responsive: these also run on the tablet and phone projects (and on WebKit when enabled).
const PAGES: [string, string][] = [
  ["/", "My work"],
  ["/projects", "Projects"],
  ["/projects/PUMP?view=board", "Pump controller firmware"],
  ["/projects/PUMP?view=list", "Pump controller firmware"],
  ["/projects/IT?view=queues", "IT help"],
  ["/projects/ORB?view=board", "Orbiter demonstration mission"],
  ["/docs", "Docs"],
  ["/portal", "Help desk"],
  ["/profile", "Profile"],
  ["/admin/overview", "Overview"],
  ["/admin/users", "Users and agents"],
  ["/admin/schemes", "Types and workflows"],
  ["/admin/audit", "Audit log"],
  ["/projects/PUMP?view=gantt", "Pump controller firmware"],
  ["/projects/PUMP?view=graph", "Pump controller firmware"],
  ["/projects/PUMP?view=report", "Pump controller firmware"],
  ["/i/PUMP-2", "Controller resets under sustained load"],
  ["/planning", "Planning"],
  ["/planning/graph", "Planning"],
  ["/planning/gantt", "Planning"],
  ["/planning/levels", "Planning"],
  ["/search?view=find&open=PUMP-2", "Search"],
  ["/analytics", "Analytics"],
  ["/analytics/library", "Analytics"],
  ["/analytics/t/kpi-tiles", "Key figures"],
  ["/people", "People"],
  ["/teams", "Teams"],
  ["/teams/chart", "Org chart"],
  ["/about", "About Gantry"],
  ["/about/licenses", "Licenses"],
  ["/status/404", "This does not exist, or you cannot see it"],
  ["/admin/notifications", "Notifications and email"],
  ["/admin/alerts", "Alerts"],
];

test.describe("layout on every screen size @responsive", () => {
  test("pages fit the screen and keep their primary navigation reachable", async ({ page, viewport }) => {
    await signInAs(page, ADMIN);
    for (const [path, heading] of PAGES) {
      await page.goto(path);
      await expect(page.getByRole("heading", { name: heading, level: 1 }).first()).toBeVisible();
      await page.waitForLoadState("networkidle");
      await expectNoSidewaysScroll(page);
    }
    if (viewport && viewport.width <= 760) {
      // Phones: a bottom tab bar replaces the side rail, and a drawer holds the rest.
      const tabs = page.getByRole("navigation", { name: "Sections", exact: true });
      await expect(tabs).toBeVisible();
      await tabs.getByRole("link", { name: "Projects" }).click();
      await expect(page).toHaveURL(/\/projects/);
      await tabs.getByRole("button", { name: "More" }).click();
      await expect(page.getByLabel("Main navigation").getByRole("link", { name: "Administration" })).toBeVisible();
    } else {
      await expect(page.getByLabel("Main navigation").getByRole("link", { name: "Projects" })).toBeVisible();
      await expect(page.getByRole("navigation", { name: "Sections", exact: true })).toBeHidden();
    }
  });

  test("work item tables keep status and priority in view at every width", async ({ page, viewport }) => {
    test.skip(!viewport || viewport.width <= 760, "phones show these tables as a list");
    await signInAs(page, ADMIN);
    await page.goto("/");
    const panel = page.locator("section.panel").filter({ hasText: "Assigned to you" });
    await expect(panel.getByRole("row").nth(1)).toBeVisible();
    // Nothing is cut off: the table fits its panel without scrolling sideways.
    const hidden = await panel.locator(".table-wrap").evaluate((el) => el.scrollWidth - el.clientWidth);
    expect(hidden, "table wider than its panel").toBeLessThanOrEqual(1);
    for (const column of ["Status", "Priority"]) await expect(panel.getByRole("columnheader", { name: column })).toBeInViewport({ ratio: 1 });
  });

  test("an item page shows its title in full and its status near the top", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/i/PUMP-2");
    const title = page.getByLabel("Title");
    await expect(title).toHaveValue("Controller resets under sustained load");
    // The field grows with the text instead of cutting it off.
    expect(await title.evaluate((el: HTMLTextAreaElement) => el.scrollHeight - el.clientHeight)).toBeLessThanOrEqual(2);
    expect(await title.evaluate((el: HTMLTextAreaElement) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(2);
    await expect(page.locator("header .badge").filter({ hasText: "In progress" })).toBeInViewport();
    await expectNoSidewaysScroll(page);
  });

  test("touch targets on small screens are large enough to hit", async ({ page, viewport }) => {
    test.skip(!viewport || viewport.width > 760, "phones only");
    await signInAs(page, ADMIN);
    await page.goto("/projects/PUMP?view=board");
    const small = await page.evaluate(() =>
      [...document.querySelectorAll<HTMLElement>("button, a.btn, select, input:not([type=checkbox]), nav.tabbar a")]
        .filter((el) => el.offsetParent !== null)
        .map((el) => ({ el: el.outerHTML.slice(0, 80), h: el.getBoundingClientRect().height }))
        .filter((x) => x.h > 0 && x.h < 32),
    );
    expect(small, "controls shorter than 32 px").toEqual([]);
  });
});

test.describe("themes @responsive", () => {
  for (const scheme of ["light", "dark"] as const) {
    test(`follows a ${scheme} device setting with readable contrast`, async ({ browser }, info) => {
      const ctx = await browser.newContext({ ...info.project.use, colorScheme: scheme });
      const page = await ctx.newPage();
      await signInAs(page, ADMIN);
      await page.goto("/i/PUMP-2");
      await expect(page.locator("html")).toHaveAttribute("data-theme", scheme);
      const { bg, ink } = await page.evaluate(() => {
        const lum = (c: string) => {
          const [r, g, b] = c.match(/[\d.]+/g)!.slice(0, 3).map((v) => {
            const x = Number(v) / 255;
            return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
          });
          return 0.2126 * r + 0.7152 * g + 0.0722 * b;
        };
        return { bg: lum(getComputedStyle(document.body).backgroundColor), ink: lum(getComputedStyle(document.querySelector(".prose") ?? document.body).color) };
      });
      const contrast = (Math.max(bg, ink) + 0.05) / (Math.min(bg, ink) + 0.05);
      expect(contrast, "body text against the page background").toBeGreaterThanOrEqual(7);
      expect(scheme === "dark" ? bg < 0.1 : bg > 0.8, `${scheme} background`).toBe(true);
      await ctx.close();
    });
  }

  test("a chosen theme overrides the device and is remembered", async ({ browser }, info) => {
    const ctx = await browser.newContext({ ...info.project.use, colorScheme: "light" });
    const page = await ctx.newPage();
    await signInAs(page, ADMIN);
    await page.getByRole("button", { name: /Account menu/ }).click();
    await page.getByRole("menuitemradio", { name: "Dark" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.getByRole("button", { name: /Account menu/ }).click();
    await page.getByRole("menuitemradio", { name: "Match device" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await ctx.close();
  });
});
