// Draws the icons and the link-preview image from the mark in public/favicon.svg.
// A maintainer runs this when the mark changes (`node scripts/make-icons.mjs`); the results
// are committed, so building the application does not need a browser.
import { chromium } from "playwright";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const pub = resolve(dirname(fileURLToPath(import.meta.url)), "..", "public");
const manual = resolve(pub, "..", "..", "docs", "manual", "assets");
const mark = (stroke) => `<path d="M8 25V9h16v16M8 9l16 8M24 9 8 17M8 17h16" fill="none" stroke="#f2b705" stroke-width="${stroke}" stroke-linecap="round" stroke-linejoin="round"/>`;
const icon = (radius, pad = 0) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${-pad} ${-pad} ${32 + pad * 2} ${32 + pad * 2}"><rect x="${-pad}" y="${-pad}" width="${32 + pad * 2}" height="${32 + pad * 2}" rx="${radius}" fill="#16202b"/>${mark(2.2)}</svg>`;

const browser = await chromium.launch();
async function shot(html, width, height, file) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
  await page.setContent(`<!doctype html><style>html,body{margin:0;background:transparent}svg{display:block;width:${width}px;height:${height}px}</style>${html}`);
  const png = await page.screenshot({ omitBackground: true, clip: { x: 0, y: 0, width, height } });
  await page.close();
  if (file) writeFileSync(join(pub, file), png);
  return png;
}

await shot(icon(0), 180, 180, "apple-touch-icon.png"); // iOS rounds the corners itself
await shot(icon(7), 192, 192, "icon-192.png");
await shot(icon(7), 512, 512, "icon-512.png");
await shot(icon(0, 5), 512, 512, "icon-maskable-512.png"); // the mark stays inside the safe zone

// favicon.ico for browsers that ask for it by name: one 32-pixel PNG in an ICO container.
const png32 = await shot(icon(7), 32, 32, null);
const ico = Buffer.alloc(22);
ico.writeUInt16LE(0, 0); ico.writeUInt16LE(1, 2); ico.writeUInt16LE(1, 4);
ico.writeUInt8(32, 6); ico.writeUInt8(32, 7); ico.writeUInt8(0, 8); ico.writeUInt8(0, 9);
ico.writeUInt16LE(1, 10); ico.writeUInt16LE(32, 12); ico.writeUInt32LE(png32.length, 14); ico.writeUInt32LE(22, 18);
writeFileSync(join(pub, "favicon.ico"), Buffer.concat([ico, png32]));

// The image chat applications and social sites show beside a link: 1200 by 630.
const og = (title, line) => `
<div style="width:1200px;height:630px;box-sizing:border-box;padding:84px 96px;background:#16202b;color:#e7ecf2;font-family:'IBM Plex Sans',system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;display:flex;flex-direction:column;justify-content:space-between">
  <div style="display:flex;align-items:center;gap:28px">
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="4 5 24 24" style="width:112px;height:112px">${mark(2)}</svg>
    <div style="font-size:92px;font-weight:600;letter-spacing:-1px">${title}</div>
  </div>
  <div>
    <div style="font-size:44px;line-height:1.25;font-weight:500;max-width:940px">${line}</div>
    <div style="margin-top:34px;width:150px;height:8px;border-radius:4px;background:#f2b705"></div>
  </div>
</div>`;
async function ogShot(html, file) {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });
  await page.setContent(`<!doctype html><style>html,body{margin:0}</style>${html}`);
  writeFileSync(file, await page.screenshot({ clip: { x: 0, y: 0, width: 1200, height: 630 } }));
  await page.close();
}
await ogShot(og("Gantry", "Projects, issues, a service desk and documentation, for teams and their AI agents."), join(pub, "og.png"));
await ogShot(og("Gantry manual", "Guides for the people who use it, the people who administer it, and the people who run its servers."), join(manual, "og.png"));
await browser.close();

writeFileSync(
  join(pub, "site.webmanifest"),
  JSON.stringify(
    {
      name: "Gantry", short_name: "Gantry", description: "Projects, issues, a service desk and documentation.", start_url: "/", scope: "/", display: "standalone",
      background_color: "#16202b", theme_color: "#16202b",
      icons: [
        { src: "/icon-192.png", sizes: "192x192", type: "image/png" },
        { src: "/icon-512.png", sizes: "512x512", type: "image/png" },
        { src: "/icon-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
        { src: "/favicon.svg", sizes: "any", type: "image/svg+xml" },
      ],
    },
    null,
    2,
  ) + "\n",
);
console.log("icons written to public/ and docs/manual/assets/");
