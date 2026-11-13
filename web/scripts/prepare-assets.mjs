// Prepares what the web application serves besides its own code. Runs before `vite` and
// before `vite build` (see package.json):
//
//   public/vendor/     the chart libraries that visualization scripts load inside the sandbox
//   public/licenses.json  every third-party package shipped: the web application's npm
//                      packages and the server's Rust crates, with their license texts
//   public/build.json  version and date of this build, shown under About
//
// Nothing here reaches the network. The Rust crates are listed with `cargo metadata
// --offline`; if cargo is not available (a web-only checkout), that part is left out and
// the file says so.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(web, "..");
const pub = join(web, "public");
const modules = join(web, "node_modules");
const pkg = JSON.parse(readFileSync(join(web, "package.json"), "utf8"));

// ── chart libraries ──
const VENDOR = [
  ["chart.js", "dist/chart.umd.min.js", "chartjs/chart.umd.min.js"],
  ["echarts", "dist/echarts.min.js", "echarts/echarts.min.js"],
  ["plotly.js-dist-min", "plotly.min.js", "plotly/plotly.min.js"],
  ["d3", "dist/d3.min.js", "d3/d3.min.js"],
];
for (const [name, from, to] of VENDOR) {
  const src = join(modules, name, from);
  if (!existsSync(src)) throw new Error(`${name} is not installed: run npm ci in web/ first`);
  mkdirSync(dirname(join(pub, "vendor", to)), { recursive: true });
  cpSync(src, join(pub, "vendor", to));
}

// ── licenses ──
const texts = new Map();
function keep(text) {
  const clean = text.replace(/\r\n/g, "\n").trim();
  if (!clean) return null;
  const id = createHash("sha256").update(clean).digest("hex").slice(0, 12);
  texts.set(id, clean);
  return id;
}
function licenseFiles(dir) {
  try {
    return readdirSync(dir)
      .filter((f) => /^(licen[sc]e|copying|notice|unlicense)/i.test(f))
      .sort()
      .slice(0, 4)
      .map((f) => {
        try {
          return keep(readFileSync(join(dir, f), "utf8").slice(0, 60_000));
        } catch {
          return null;
        }
      })
      .filter(Boolean);
  } catch {
    return [];
  }
}
const spdx = (l) => (typeof l === "string" ? l : Array.isArray(l) ? l.map((x) => x.type ?? x).join(" OR ") : (l?.type ?? "See the license text"));
const link = (r) => {
  const url = typeof r === "string" ? r : r?.url;
  if (!url) return "";
  const m = url.replace(/^git\+/, "").replace(/^git:\/\//, "https://").replace(/^git@github\.com:/, "https://github.com/").replace(/^github:/, "https://github.com/").replace(/\.git$/, "");
  return /^https?:\/\//.test(m) ? m : /^[\w.-]+\/[\w.-]+$/.test(m) ? `https://github.com/${m}` : "";
};

// The web application: what package-lock.json says is installed for production.
const lock = JSON.parse(readFileSync(join(web, "package-lock.json"), "utf8"));
const npm = [];
for (const [path, info] of Object.entries(lock.packages ?? {})) {
  if (!path || info.dev || info.devOptional) continue;
  const dir = join(web, path);
  let meta;
  try {
    meta = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  } catch {
    continue; // an optional package for another platform
  }
  npm.push({ name: meta.name, version: meta.version, license: spdx(meta.license ?? meta.licenses ?? info.license), url: link(meta.repository) || meta.homepage || "", texts: licenseFiles(dir) });
}
npm.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));

// The server: every crate that is compiled into the release binaries.
let crates = [];
let cratesNote = "";
try {
  const meta = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", "--offline", "--locked", "--filter-platform", "x86_64-unknown-linux-gnu"], { cwd: repo, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "ignore"] }));
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const members = new Set(meta.workspace_members);
  // Walk normal and build dependencies from the workspace; development-only ones are not shipped.
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const seen = new Set();
  const queue = [...members];
  while (queue.length) {
    const id = queue.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id)?.deps ?? []) if (dep.dep_kinds.some((k) => k.kind !== "dev")) queue.push(dep.pkg);
  }
  crates = [...seen]
    .filter((id) => !members.has(id))
    .map((id) => byId.get(id))
    .map((p) => ({ name: p.name, version: p.version, license: p.license ?? "See the license text", url: p.repository ?? p.homepage ?? "", texts: licenseFiles(dirname(p.manifest_path)) }))
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
} catch {
  cratesNote = "The server's Rust crates could not be listed when this file was built.";
}

const summary = {};
for (const p of [...npm, ...crates]) summary[p.license] = (summary[p.license] ?? 0) + 1;
writeFileSync(
  join(pub, "licenses.json"),
  JSON.stringify({
    generated: new Date().toISOString().slice(0, 10),
    product: { name: "Gantry", version: pkg.version, license: "Apache-2.0" },
    groups: [
      { key: "web", name: "Web application", description: "JavaScript packages delivered to your browser.", packages: npm },
      { key: "server", name: "Server", description: "Rust crates compiled into the server and the MCP bridge.", packages: crates, note: cratesNote || undefined },
    ],
    summary: Object.entries(summary).sort((a, b) => b[1] - a[1]).map(([license, count]) => ({ license, count })),
    texts: Object.fromEntries(texts),
  }),
);

// ── build stamp ──
// SOURCE_DATE_EPOCH keeps the RPM build reproducible.
const when = process.env.SOURCE_DATE_EPOCH ? new Date(Number(process.env.SOURCE_DATE_EPOCH) * 1000) : new Date();
writeFileSync(join(pub, "build.json"), JSON.stringify({ name: "gantry-web", version: pkg.version, date: when.toISOString().slice(0, 10) }));

console.log(`assets: ${VENDOR.length} chart libraries, ${npm.length} npm packages, ${crates.length} crates, ${texts.size} license texts`);
