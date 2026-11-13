// Builds the web manual: docs/manual/src/**/*.md -> docs/manual/_site/**/*.html
//
//   npm run manual          (from web/)
//
// The site is plain static HTML with one stylesheet and one small script, so it works when
// served by Gantry under /manual/ (strict content security policy), from any web server, or
// opened from disk. Reference tables are generated from docs/manual/reference.json, which is
// the server's own catalog (`npm run manual:data` refreshes it). The build fails on a
// broken internal link or a missing image, so the manual cannot ship with dead ends.
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, posix, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import MarkdownIt from "markdown-it";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../docs/manual");
const src = join(root, "src");
const out = join(root, "_site");
const nav = JSON.parse(readFileSync(join(root, "nav.json"), "utf8"));
const ref = JSON.parse(readFileSync(join(root, "reference.json"), "utf8"));

const nasa = JSON.parse(readFileSync(resolve(root, "../../addons/nasa/pack.json"), "utf8"));
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const slug = (s) => s.toLowerCase().replace(/<[^>]+>/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
const code = (s) => `<code>${esc(s)}</code>`;
const table = (head, rows) => `<div class="table"><table><thead><tr>${head.map((h) => `<th>${esc(h)}</th>`).join("")}</tr></thead><tbody>${rows.map((r) => `<tr>${r.map((c) => `<td>${c}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`;

const alertKinds = (scope) =>
  table(
    ["What is measured", "Meaning", "Settings and their defaults"],
    ref.alert_kinds.filter((k) => k.scope === scope).map((k) => [`<strong>${esc(k.name)}</strong><br>${code(k.kind)}`, esc(k.description), k.params.length ? k.params.map((p) => `${esc(p.label)}: ${p.default}`).join("<br>") : "None"]),
  );
// ── generated reference blocks: written in Markdown as {{name}} on a line of its own ──
const generated = {
  version: () => esc(ref.version),
  permissions: () => table(["Permission", "Applies to", "Allows"], ref.permissions.map((p) => [code(p.key), esc(p.scope), esc(p.description)])),
  roles: () =>
    ["global", "project", "space"]
      .map((scope) => {
        const title = { global: "Workspace-wide roles", project: "Project roles", space: "Space roles" }[scope];
        const rows = ref.roles.filter((r) => r.scope === scope).map((r) => [`<strong>${esc(r.name)}</strong><br>${code(r.key)}`, esc(r.description), r.permissions.includes("*") ? "Every permission" : r.permissions.map(code).join(" ")]);
        return `<h3 id="${slug(title)}">${title}</h3>${table(["Role", "For", "Permissions"], rows)}`;
      })
      .join("\n"),
  routes: () => {
    const tags = [...new Set(ref.routes.map((r) => r.tag))];
    return tags
      .map((tag) => `<h3 id="${slug(tag)}">${esc(tag)}</h3>${table(["Method", "Path", "What it does"], ref.routes.filter((r) => r.tag === tag).map((r) => [`<span class="method ${r.method.toLowerCase()}">${r.method}</span>`, code(r.path) + (r.public ? ' <span class="tag">no sign-in</span>' : ""), esc(r.summary)]))}`)
      .join("\n");
  },
  events: () => table(["Event", "Sent when"], ref.events.map((e) => [code(e.kind), esc(e.description)])),
  integrations: () => table(["Kind", "Direction", "What it does", "Settings", "Secrets"], ref.integrations.map((k) => [`<strong>${esc(k.name)}</strong><br>${code(k.kind)}`, esc(k.direction), esc(k.description), k.settings.map((s) => esc(s.label) + (s.required ? " (required)" : "")).join("<br>") || "None", k.secrets.map((s) => esc(s.label) + (s.required ? " (required)" : "")).join("<br>") || "None"])),
  mcp_tools: () => table(["Tool", "What it does", "Arguments"], ref.mcp_tools.map((t) => [code(t.name), esc(t.description), Object.entries(t.inputSchema?.properties ?? {}).map(([k, v]) => `${code(k)}${(t.inputSchema.required ?? []).includes(k) ? " (required)" : ""}${v.description ? `: ${esc(v.description)}` : ""}`).join("<br>") || "None"])),
  default_config: () => `<pre><code>${esc(ref.default_config)}</code></pre>`,
  status_codes: () => table(["Code", "Heading", "What it means", "What to do"], ref.status_texts.map((t) => [`<a href="/status/${t.code}">${t.code}</a>`, `<strong>${esc(t.title)}</strong>`, esc(t.meaning), esc(t.action)])),
  notification_categories: () =>
    table(
      ["Kind", "Sent when", "In the application", "By email", "Who sees it"],
      ref.notification_categories.map((c) => [`<strong>${esc(c.name)}</strong><br>${code(c.key)}`, esc(c.description), c.in_app ? "On" : "Off", c.email ? "On" : "Off", c.permission ? `Accounts with ${code(c.permission)}` : "Everyone"]),
    ),
  alert_kinds_system: () => alertKinds("system"),
  alert_kinds_application: () => alertKinds("application"),
  viz_templates: () => table(["Name", "Shows", "Drawn with"], ref.visualizations.templates.map((t) => [`<strong>${esc(t.name)}</strong><br>${code(t.key)}`, esc(t.description), esc(ref.visualizations.libraries.find((l) => l.key === t.library)?.name ?? t.library)])),
  core_item_types: () => table(["Type", "Workflow", "Purpose"], ref.core.item_types.map((t) => [`<strong>${esc(t.name)}</strong><br>${code(t.key)}`, esc(ref.core.workflows.find((w) => w.key === t.workflow)?.name ?? t.workflow), esc(t.description)])),
  core_workflows: () => table(["Workflow", "Statuses, in order", "Rules"], ref.core.workflows.map((w) => [`<strong>${esc(w.name)}</strong><br>${code(w.key)}`, w.statuses.map((s) => esc(s.name)).join(" → "), w.transitions.filter((t) => t.requires?.length).map((t) => `“${esc(t.name)}” needs ${t.requires.map((f) => esc(ref.core.fields.find((x) => x.key === f)?.name ?? f)).join(", ")}`).join("<br>") || "Any step from any status"])),
  core_templates: () => table(["Template", "Discipline", "Starts with"], ref.core.project_templates.map((t) => [`<strong>${esc(t.name)}</strong><br>${code(t.key)}`, esc(t.domain), esc(t.description)])),
  // The NASA add-on's own tables come straight from its pack file.
  nasa_types: () => {
    const fieldName = (k) => [...nasa.fields, ...ref.core.fields].find((f) => f.key === k)?.name ?? k;
    const wfName = (k) => [...nasa.workflows, ...ref.core.workflows].find((w) => w.key === k)?.name ?? k;
    return table(["Type", "Purpose", "Information it carries", "Workflow"], nasa.item_types.map((t) => [`<strong>${esc(t.name)}</strong>`, esc(t.description), (t.fields ?? []).map((f) => esc(fieldName(f.key)) + (f.required ? " (required)" : "")).join(", "), esc(wfName(t.workflow))]));
  },
  nasa_workflows: () => {
    const fieldName = (k) => [...nasa.fields, ...ref.core.fields].find((f) => f.key === k)?.name ?? k;
    return table(["Workflow", "Statuses", "Steps that need information first"], nasa.workflows.map((w) => [`<strong>${esc(w.name)}</strong>`, w.statuses.map((x) => esc(x.name)).join(" → "), w.transitions.filter((t) => t.requires?.length).map((t) => `“${esc(t.name)}” needs ${t.requires.map((f) => esc(fieldName(f))).join(" and ")}`).join("<br>") || "None"]));
  },
  nasa_fields: () => table(["Field", "Kind", "Choices"], nasa.fields.map((f) => [esc(f.name), esc({ select: "One choice", textarea: "Long text", text: "Short text", date: "Date", number: "Number" }[f.type] ?? f.type), (f.options ?? []).map(esc).join(", ")])),
  nasa_page_templates: () => table(["Template", "Use"], nasa.page_templates.map((t) => [`<strong>${esc(t.name)}</strong>`, esc(t.description)])),
  nasa_template_items: () => {
    const t = nasa.project_templates[0];
    const rows = t.items.map((i) => [`<strong>${esc(i.title)}</strong>`, (i.children ?? []).map((c) => esc(c.title)).join("<br>")]);
    return table(["Created in the project", "With these items beneath it"], rows);
  },
  nasa_template_name: () => esc(nasa.project_templates[0].name),
  core_page_templates: () => table(["Template", "Group", "Use it for"], ref.core.page_templates.map((t) => [esc(t.name), esc(t.category), esc(t.description)])),
};

const md = new MarkdownIt({ html: true, linkify: false, typographer: false });
md.renderer.rules.heading_open = (tokens, i, _o, env) => {
  const text = tokens[i + 1].content;
  const level = Number(tokens[i].tag.slice(1));
  let id = slug(text);
  while (env.ids.has(id)) id += "-2";
  env.ids.add(id);
  if (level === 2 || level === 3) env.toc.push({ level, id, text: md.renderInline(text).replace(/<[^>]+>/g, "") });
  return `<${tokens[i].tag} id="${id}">`;
};
const defaultFence = md.renderer.rules.fence;
md.renderer.rules.fence = (tokens, i, o, env, self) => `<div class="code">${defaultFence(tokens, i, o, env, self)}</div>`;
md.renderer.rules.table_open = () => '<div class="table"><table>';
md.renderer.rules.table_close = () => "</table></div>";

function pages(dir = src, list = []) {
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) pages(p, list);
    else if (name.endsWith(".md")) list.push(relative(src, p).split("\\").join("/"));
  }
  return list;
}
function frontMatter(text, file) {
  const m = /^---\n([\s\S]*?)\n---\n/.exec(text);
  if (!m) throw new Error(`${file}: missing front matter (title, summary)`);
  const meta = Object.fromEntries(m[1].split("\n").filter(Boolean).map((l) => [l.slice(0, l.indexOf(":")).trim(), l.slice(l.indexOf(":") + 1).trim()]));
  if (!meta.title) throw new Error(`${file}: front matter needs a title`);
  return { meta, body: text.slice(m[0].length) };
}

const all = pages();
const docs = new Map();
for (const file of all) {
  const { meta, body } = frontMatter(readFileSync(join(src, file), "utf8"), file);
  docs.set(file, { file, url: file.replace(/\.md$/, ".html"), meta, body });
}
// Every page must be in the navigation, and every navigation entry must exist.
const ordered = nav.flatMap((s) => s.pages.map((p) => ({ ...s, page: p })));
for (const o of ordered) if (!docs.has(o.page)) throw new Error(`nav.json lists ${o.page}, which does not exist`);
for (const file of all) if (!ordered.some((o) => o.page === file)) throw new Error(`${file} is not listed in nav.json`);

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
cpSync(join(root, "assets"), join(out, "assets"), { recursive: true });
if (existsSync(join(src, "img"))) cpSync(join(src, "img"), join(out, "img"), { recursive: true });

const problems = [];
const search = [];
const rel = (from, to) => posix.relative(posix.dirname(from), to) || posix.basename(to);

for (const [index, o] of ordered.entries()) {
  const doc = docs.get(o.page);
  const env = { ids: new Set(), toc: [] };
  const source = doc.body.replace(/^\{\{(\w+)\}\}$/gm, (_m, name) => {
    if (!generated[name]) throw new Error(`${doc.file}: unknown block {{${name}}}`);
    return `\n${generated[name]()}\n`;
  }).replace(/\{\{version\}\}/g, esc(ref.version)).replace(/\{\{nasa_template_name\}\}/g, generated.nasa_template_name());
  let html = md.render(source, env);
  // Callouts: a blockquote starting with **Note**, **Important**, **Warning** or **Tip**.
  html = html.replace(/<blockquote>\s*<p><strong>(Note|Important|Warning|Tip)<\/strong>:?/g, (_m, kind) => `<blockquote class="callout ${kind.toLowerCase()}"><p><strong>${kind}</strong>`);
  // Links: .md -> .html, and verify that every internal target exists.
  html = html.replace(/(href|src)="([^"]+)"/g, (m, attr, target) => {
    if (/^(https?:|mailto:|#|\/)/.test(target)) return m;
    const [path, hash] = target.split("#");
    const resolved = posix.normalize(posix.join(posix.dirname(doc.file), path));
    if (attr === "src") {
      if (!existsSync(join(src, resolved))) problems.push(`${doc.file}: image ${target} not found`);
      return m;
    }
    if (path.endsWith(".md")) {
      if (!docs.has(resolved)) problems.push(`${doc.file}: link to ${target} goes nowhere`);
      return `href="${path.replace(/\.md$/, ".html")}${hash ? `#${hash}` : ""}"`;
    }
    return m;
  });
  const section = nav.find((s) => s.pages.includes(doc.file));
  const prev = ordered[index - 1] && docs.get(ordered[index - 1].page);
  const next = ordered[index + 1] && docs.get(ordered[index + 1].page);
  const base = rel(doc.url, "index.html").replace(/index\.html$/, "");
  const sections = nav.filter((s) => s.title).map((s) => `<a href="${rel(doc.url, docs.get(s.pages[0]).url)}"${s === section ? ' aria-current="true"' : ""}>${esc(s.title)}</a>`).join("");
  const side = section.pages.map((p) => `<li><a href="${rel(doc.url, docs.get(p).url)}"${p === doc.file ? ' aria-current="page"' : ""}>${esc(docs.get(p).meta.nav ?? docs.get(p).meta.title)}</a></li>`).join("");
  const toc = env.toc.filter((t) => t.level === 2).length > 1 ? `<nav class="toc" aria-label="On this page"><h2>On this page</h2><ul>${env.toc.map((t) => `<li class="l${t.level}"><a href="#${t.id}">${esc(t.text)}</a></li>`).join("")}</ul></nav>` : "";
  const page = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(doc.meta.title)} · Gantry manual</title>
<meta name="description" content="${esc(doc.meta.summary ?? "")}">
<meta name="color-scheme" content="light dark">
<link rel="icon" href="${base}assets/favicon.svg" type="image/svg+xml">
<meta property="og:type" content="article">
<meta property="og:site_name" content="Gantry manual">
<meta property="og:title" content="${esc(doc.meta.title)}">
<meta property="og:description" content="${esc(doc.meta.summary ?? "Guides for the people who use Gantry, administer it, and run its servers.")}">
<meta property="og:image" content="${base}assets/og.png">
<meta name="twitter:card" content="summary_large_image">
<script src="${base}assets/theme.js"></script>
<link rel="stylesheet" href="${base}assets/manual.css">
<script src="${base}assets/manual.js" defer></script>
</head>
<body data-base="${base}">
<a class="skip" href="#content">Skip to content</a>
<header class="top">
  <a class="brand" href="${base}index.html">Gantry <span>manual</span></a>
  <nav class="sections" aria-label="Guides">${sections}</nav>
  <div class="search"><input type="search" id="q" placeholder="Search the manual" aria-label="Search the manual" autocomplete="off"><ul id="results" hidden></ul></div>
  <div class="theme" role="group" aria-label="Theme" hidden><button type="button" data-choice="system" aria-pressed="true" title="Follow this device">Auto</button><button type="button" data-choice="light" aria-pressed="false">Light</button><button type="button" data-choice="dark" aria-pressed="false">Dark</button></div>
  <a class="app" href="/">Open Gantry</a>
</header>
<div class="layout${section.title ? "" : " home"}">
  ${section.title ? `<nav class="side" aria-label="${esc(section.title)}"><h2>${esc(section.title)}</h2><ul>${side}</ul></nav>` : ""}
  <main id="content">
    <h1>${esc(doc.meta.title)}</h1>
    ${doc.meta.summary ? `<p class="lead">${esc(doc.meta.summary)}</p>` : ""}
    ${html}
    <footer class="pager">${prev ? `<a class="prev" href="${rel(doc.url, prev.url)}"><span>Previous</span>${esc(prev.meta.title)}</a>` : "<span></span>"}${next ? `<a class="next" href="${rel(doc.url, next.url)}"><span>Next</span>${esc(next.meta.title)}</a>` : ""}</footer>
    <p class="version">Gantry ${esc(ref.version)}</p>
  </main>
  ${toc}
</div>
</body>
</html>
`;
  mkdirSync(dirname(join(out, doc.url)), { recursive: true });
  writeFileSync(join(out, doc.url), page);
  const text = html.replace(/<pre[\s\S]*?<\/pre>/g, " ").replace(/<[^>]+>/g, " ").replace(/&[a-z]+;/g, " ").replace(/\s+/g, " ").trim();
  search.push({ url: doc.url, title: doc.meta.title, section: section.title ?? "", headings: env.toc.map((t) => t.text), text: text.slice(0, 4000) });
}
// Anchors: every #fragment in an internal link must exist in its target page.
for (const o of ordered) {
  const doc = docs.get(o.page);
  const html = readFileSync(join(out, doc.url), "utf8");
  for (const m of html.matchAll(/href="([^"#:]*)#([^"]+)"/g)) {
    const target = m[1] ? posix.normalize(posix.join(posix.dirname(doc.url), m[1])) : doc.url;
    if (!existsSync(join(out, target))) continue;
    if (!new RegExp(`id="${m[2].replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`).test(readFileSync(join(out, target), "utf8"))) problems.push(`${doc.file}: no section #${m[2]} in ${target}`);
  }
}
writeFileSync(join(out, "search.json"), JSON.stringify(search));
// The icon is the application's; version and date are shown under About in the application.
cpSync(resolve(root, "../../web/public/favicon.svg"), join(out, "assets", "favicon.svg"));
const built = process.env.SOURCE_DATE_EPOCH ? new Date(Number(process.env.SOURCE_DATE_EPOCH) * 1000) : new Date();
writeFileSync(join(out, "build.json"), JSON.stringify({ name: "gantry-manual", version: ref.version, date: built.toISOString().slice(0, 10), pages: ordered.length }));
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`manual: ${ordered.length} pages -> ${relative(process.cwd(), out)}`);
