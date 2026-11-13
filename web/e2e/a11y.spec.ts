import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { ADMIN, signInAs } from "./helpers";

// Automated accessibility checks (axe-core, WCAG 2.1 A and AA) on every kind of page, in
// both themes. Automated checks find only part of what matters; the keyboard and screen
// reader behavior of each feature is tested where the feature is tested.
const PAGES: [string, string][] = [
  ["/", "My work"],
  ["/projects/PUMP?view=board", "Pump controller firmware"],
  ["/projects/PUMP?view=list", "Pump controller firmware"],
  ["/projects/PUMP?view=gantt", "Pump controller firmware"],
  ["/projects/PUMP?view=graph", "Pump controller firmware"],
  ["/projects/PUMP?view=report", "Pump controller firmware"],
  ["/projects/PUMP?view=settings", "Pump controller firmware"],
  ["/i/PUMP-2", "Controller resets under sustained load"],
  ["/planning", "Planning"],
  ["/planning/graph", "Planning"],
  ["/planning/gantt", "Planning"],
  ["/planning/levels", "Planning"],
  ["/search?view=find", "Search"],
  ["/analytics", "Analytics"],
  ["/analytics/library", "Analytics"],
  ["/analytics/t/throughput-bars", "Created and resolved"],
  ["/people", "People"],
  ["/teams", "Teams"],
  ["/teams/chart", "Org chart"],
  ["/docs/ENG", "Engineering handbook"],
  ["/portal", "Help desk"],
  ["/profile", "Profile"],
  ["/about", "About Gantry"],
  ["/about/cookies", "Cookies"],
  ["/about/licenses", "Licenses"],
  ["/status", "When something goes wrong"],
  ["/no/such/page", "This does not exist, or you cannot see it"],
  ["/admin/overview", "Overview"],
  ["/admin/users", "Users and agents"],
  ["/admin/settings", "Settings"],
  ["/admin/notifications", "Notifications and email"],
  ["/admin/alerts", "Alerts"],
  ["/admin/schemes", "Types and workflows"],
];

async function check(page: Page, where: string) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    // Third-party canvases and the sandboxed frames draw their own content; their pages are
    // checked by the libraries' own options (aria labels are asserted in analytics.spec.ts).
    .exclude("iframe")
    .analyze();
  const problems = results.violations.map((v) => `${v.id} (${v.impact}): ${v.help}\n    ${v.nodes.slice(0, 4).map((n) => n.target.join(" ")).join("\n    ")}`);
  expect(problems, `${where}\n${problems.join("\n")}`).toEqual([]);
}

for (const theme of ["light", "dark"] as const) {
  test.describe(`accessibility, ${theme} theme`, () => {
    test.use({ colorScheme: theme });

    test("the sign-in page, the public statements and the error page pass", async ({ page }) => {
      for (const path of ["/login", "/legal/privacy", "/status/403"]) {
        await page.goto(path);
        await expect(page.locator("h1").first()).toBeVisible();
        await expect(page.locator("h1")).toHaveCount(1);
        await expect(page).toHaveTitle(/.+ · Gantry$/);
        await check(page, `${path} (${theme})`);
      }
    });

    test("every kind of page passes", async ({ page }) => {
      test.setTimeout(240_000);
      await signInAs(page, ADMIN);
      for (const [path, heading] of PAGES) {
        await page.goto(path);
        await expect(page.getByRole("heading", { name: heading, level: 1 }).first()).toBeVisible();
        // The browser tab is named after the page, never after the one before it.
        await expect(page).toHaveTitle(new RegExp(`${heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}.* · Gantry$`));
        await page.waitForLoadState("networkidle");
        await check(page, `${path} (${theme})`);
      }
    });
  });
}

test.describe("using the keyboard alone", () => {
  test("the first Tab offers a way past the navigation, and menus work with the arrow keys", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/projects");
    await expect(page.getByRole("heading", { name: "Projects", level: 1 })).toBeVisible();
    await page.keyboard.press("Tab");
    const skip = page.getByRole("link", { name: "Skip to content" });
    await expect(skip).toBeFocused();
    // ... and can be seen while it has focus.
    await expect(skip).toBeInViewport();
    await page.keyboard.press("Enter");
    await expect(page.locator("#main")).toBeFocused();

    // The account menu: opened from its button, walked with the arrows, closed with Escape,
    // which puts focus back on the button.
    const account = page.getByRole("button", { name: /Account menu/ });
    await account.focus();
    await page.keyboard.press("Enter");
    const menu = page.getByRole("menu");
    await expect(menu.getByRole("menuitem").first()).toBeFocused();
    await page.keyboard.press("End");
    await expect(menu.getByRole("menuitem", { name: "Sign out" })).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(menu.getByRole("menuitem").first()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(menu).toBeHidden();
    await expect(account).toBeFocused();
  });

  test("Tab indents in the editor, and Escape then Tab leaves it", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/i/PUMP-2");
    const box = page.getByRole("textbox", { name: "Message (Markdown)" });
    await box.click();
    await page.keyboard.type("- one");
    await page.keyboard.press("Tab");
    await expect(box).toBeFocused();
    // The way out is announced with the box.
    await expect(box).toHaveAccessibleDescription(/press Escape, then Tab/);
    await page.keyboard.press("Escape");
    await page.keyboard.press("Tab");
    await expect(box).not.toBeFocused();
    await expect(page.getByRole("button", { name: "Post message" })).toBeFocused();
  });

  test("a date is chosen from the calendar with the arrow keys", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/i/PUMP-2");
    const box = page.getByRole("textbox", { name: "Message (Markdown)" });
    await box.click();
    await page.getByRole("button", { name: "Insert a date" }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("dialog", { name: "Insert a date" })).toBeVisible();
    const day = () => page.evaluate(() => (document.activeElement?.getAttribute("role") === "gridcell" ? document.activeElement.getAttribute("data-day") : null));
    for (let i = 0; i < 5 && !(await day()); i++) await page.keyboard.press("Tab");
    const from = await day();
    expect(from, "Tab reaches the day the calendar opened on").toMatch(/^\d{4}-\d{2}-\d{2}$/);
    await page.keyboard.press("ArrowDown"); // one week on
    await page.keyboard.press("ArrowRight"); // and a day
    const to = await day();
    expect((Date.parse(`${to}T12:00:00Z`) - Date.parse(`${from}T12:00:00Z`)) / 86_400_000).toBe(8);
    await page.keyboard.press("Enter");
    await expect(box).toContainText(`[[${to}]]`);
  });

  test("a dialog takes focus, keeps Tab inside, and gives focus back when it closes", async ({ page }) => {
    await signInAs(page, ADMIN);
    await page.goto("/projects/PUMP?view=list");
    const opener = page.getByRole("button", { name: "New item" });
    await opener.focus();
    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog", { name: /New/ });
    await expect(dialog).toBeVisible();
    expect(await page.evaluate(() => !!document.activeElement?.closest('[role="dialog"]'))).toBe(true);
    for (let i = 0; i < 40; i++) {
      await page.keyboard.press(i % 7 === 6 ? "Shift+Tab" : "Tab");
      expect(await page.evaluate(() => !!document.activeElement?.closest('[role="dialog"]')), `after ${i + 1} presses`).toBe(true);
    }
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(opener).toBeFocused();
  });
});
