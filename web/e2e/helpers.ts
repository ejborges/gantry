import { expect, request as playwrightRequest, type APIRequestContext, type Page } from "@playwright/test";

export const PASSWORD = "correct horse battery staple";
export const ADMIN = "admin@example.org";
export const MEMBER = "ada@example.org";
export const OTHER_MEMBER = "ben@example.org";
export const CUSTOMER = "pat@example.org";

/** A short unique suffix so tests can run repeatedly against the same server. */
export const uniq = () => Math.random().toString(36).slice(2, 7).toUpperCase().replace(/[0-9]/g, "X");

export async function signIn(page: Page, email: string, password = PASSWORD) {
  await page.goto("/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
}

export async function signInAs(page: Page, email: string) {
  await signIn(page, email);
  await expect(page).not.toHaveURL(/\/login/);
  await expect(page.getByRole("button", { name: /Account menu/ })).toBeVisible();
}

/** An API client signed in as `email`: used to arrange data quickly, exactly as a script would. */
export class Api {
  private constructor(private ctx: APIRequestContext, private csrf: string, public me: any) {}

  static async as(baseURL: string, email: string): Promise<Api> {
    const ctx = await playwrightRequest.newContext({ baseURL });
    const login = await ctx.post("/api/v1/auth/login", { data: { email, password: PASSWORD } });
    expect(login.status(), `sign in as ${email}`).toBe(200);
    const me = await (await ctx.get("/api/v1/auth/me")).json();
    return new Api(ctx, me.csrf, me);
  }

  async call(method: string, path: string, data?: unknown) {
    return this.ctx.fetch(path, { method, data, headers: method === "GET" ? {} : { "X-Gantry-CSRF": this.csrf } });
  }
  async ok(method: string, path: string, data?: unknown) {
    const r = await this.call(method, path, data);
    expect(r.status(), `${method} ${path}: ${await r.text()}`).toBeLessThan(300);
    return r.status() === 204 ? null : r.json();
  }
  dispose() {
    return this.ctx.dispose();
  }
}

/** The page must never grow wider than the screen: a sideways-scrolling page is a layout bug. */
export async function expectNoSidewaysScroll(page: Page) {
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow, "page scrolls sideways").toBeLessThanOrEqual(1);
}
