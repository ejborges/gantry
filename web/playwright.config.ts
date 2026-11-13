import { defineConfig, devices } from "@playwright/test";
import { createRequire } from "node:module";
const { baseUrl } = createRequire(import.meta.url)("./scripts/base-url.cjs") as { baseUrl: () => string };

// Browser tests run against a live server with the sample data (dev/run-local.sh, or dev/e2e.sh
// which starts one for you). Chromium runs everywhere; WebKit (Safari's engine) is added with
// GANTRY_E2E_WEBKIT=1 on machines where `npx playwright install webkit` has been run.
const webkit = !!process.env.GANTRY_E2E_WEBKIT;

export default defineConfig({
  testDir: "e2e",
  timeout: 40_000,
  expect: { timeout: 8_000 },
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"]],
  use: {
    baseURL: baseUrl(), // GANTRY_E2E_URL, the Vagrant machine's forwarded port, or this machine: see scripts/base-url.cjs
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "tablet", grep: /@responsive/, use: { ...devices["Desktop Chrome"], viewport: { width: 820, height: 1180 }, hasTouch: true } },
    { name: "phone", grep: /@responsive/, use: { ...devices["Pixel 7"] } },
    ...(webkit
      ? [
          { name: "safari", use: { ...devices["Desktop Safari"] } },
          { name: "iphone", grep: /@responsive/, use: { ...devices["iPhone 14"] } },
        ]
      : []),
  ],
});
