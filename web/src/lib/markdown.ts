// Markdown is the single stored format for descriptions, messages and pages.
//
//  render()         Markdown -> sanitized HTML for display
//  enhance()        fills in what needs code to draw: diagrams, formulas, embedded content
//  renderForEditor() / toMarkdown()   the simple editor's round trip
//
// The dialect is the common ground of three that people already write:
//   GitHub  tables, task lists, strikethrough, autolinks, footnotes, alerts, emoji, a safe subset of HTML
//   GitLab  math ($`a^2`$ and ```math), ```mermaid, [[_TOC_]], {+added+} {-removed-}, >>> block quotes,
//           audio and video through image syntax, image sizes ({width=300})
//   Obsidian  [[wikilinks]], ![[embeds]], ==highlights==, callouts (> [!note]), %%comments%%, #tags,
//           image sizes (![alt|300](…)), properties (front matter)
// and adds references to work: OPS-12, @handle, [[2026-03-31]].
//
// Everything shown from Markdown passes through DOMPurify; nothing else may set innerHTML.
// Scripts, styles and frames never reach the page: content that needs them runs in the
// sandbox (lib/sandbox.ts).
import MarkdownIt from "markdown-it";
import type Token from "markdown-it/lib/token.mjs";
import footnote from "markdown-it-footnote";
// @ts-expect-error the package ships no type declarations
import mark from "markdown-it-mark";
import { full as emoji } from "markdown-it-emoji";
import DOMPurify from "dompurify";
import TurndownService from "turndown";
// @ts-expect-error the package ships no type declarations
import { gfm } from "turndown-plugin-gfm";
import type { Attachment, Refs } from "../api";
import { day } from "./format";
import { diagramSvg, parseDiagram } from "./diagram";
import { mountSandbox, type SandboxHandle } from "./sandbox";

export interface RenderContext {
  /** What the server resolved for this reader. A reference that is not in here stays plain text. */
  refs?: Refs;
  /** Files that ![[name]] can show, and whose kind decides between a picture and a player. */
  attachments?: Attachment[];
}

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

// ───────────────────────── shared rules ─────────────────────────

/** GitHub-style task lists: "- [ ] text" and "- [x] text". */
function taskLists(md: MarkdownIt) {
  md.core.ruler.after("inline", "task-lists", (state) => {
    const tokens = state.tokens;
    for (let i = 2; i < tokens.length; i++) {
      if (tokens[i].type !== "inline" || tokens[i - 1].type !== "paragraph_open" || tokens[i - 2].type !== "list_item_open") continue;
      const m = /^\[([ xX])\]\s+/.exec(tokens[i].content);
      if (!m) continue;
      const checked = m[1] !== " ";
      tokens[i].content = tokens[i].content.slice(m[0].length);
      const first = tokens[i].children?.[0];
      if (first && first.type === "text") first.content = first.content.slice(m[0].length);
      tokens[i - 2].attrSet("data-type", "taskItem");
      tokens[i - 2].attrSet("data-checked", String(checked));
      for (let j = i - 3; j >= 0; j--) {
        if (tokens[j].type === "bullet_list_open" && tokens[j].level === tokens[i - 2].level - 1) {
          tokens[j].attrSet("data-type", "taskList");
          break;
        }
      }
    }
  });
}

// The simple editor's renderer: plain CommonMark with tables and task lists. Everything else
// stays literal text, which is what lets a page round-trip through the simple editor unchanged.
const basic = new MarkdownIt({ html: false, linkify: true, breaks: false });
taskLists(basic);
const basicFence = basic.renderer.rules.fence!;
basic.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const t = tokens[idx];
  if (t.info.trim() === "mermaid") return `<pre class="mermaid-src" data-mermaid="1"><code>${esc(t.content)}</code></pre>`;
  return basicFence(tokens, idx, options, env, self);
};

// ───────────────────────── the full dialect ─────────────────────────

const md = new MarkdownIt({ html: true, linkify: true, breaks: false });
// The plugins' type declarations name a different build of markdown-it's own types.
type Plugin = (md: MarkdownIt) => void;
md.use(footnote as unknown as Plugin).use(mark).use(emoji as unknown as Plugin);
taskLists(md);

const MEDIA = { video: /\.(mp4|m4v|webm|ogv|mov)(\?|#|$)/i, audio: /\.(mp3|m4a|ogg|oga|wav|flac|opus)(\?|#|$)/i };
const ATTACHMENT_URL = /^\/api\/v1\/attachments\/([0-9a-f-]{36})$/i;

function kindOf(src: string, alt: string, ctx: RenderContext): "image" | "video" | "audio" {
  const id = ATTACHMENT_URL.exec(src)?.[1];
  const type = id ? ctx.attachments?.find((a) => a.id === id)?.content_type : undefined;
  if (type?.startsWith("video/")) return "video";
  if (type?.startsWith("audio/")) return "audio";
  if (type?.startsWith("image/")) return "image";
  for (const text of [src, alt]) {
    if (MEDIA.video.test(text)) return "video";
    if (MEDIA.audio.test(text)) return "audio";
  }
  return "image";
}

const dimension = (v: string | null | undefined) => (v && /^\d{1,4}%?$/.test(v) ? v : null);

/** A bare file name that is one of the attached files stands for that file. */
function attached(target: string, ctx: RenderContext): string {
  if (!target || /^[a-z][a-z0-9+.-]*:|^[/#?]/i.test(target)) return target;
  let name = target;
  try {
    name = decodeURIComponent(target);
  } catch {
    /* keep it as written */
  }
  const file = ctx.attachments?.find((a) => a.filename.toLowerCase() === name.toLowerCase());
  return file ? `/api/v1/attachments/${file.id}` : target;
}

/** A picture, or for sound and film a player with the browser's own controls. */
function media(src: string, label: string, width: string | null, height: string | null, ctx: RenderContext): string {
  const kind = kindOf(src, label, ctx);
  const size = `${width ? ` width="${esc(width)}"` : ""}${height ? ` height="${esc(height)}"` : ""}`;
  if (kind === "video") return `<video class="media" controls preload="metadata" src="${esc(src)}"${size} aria-label="${esc(label || "Video")}"></video>`;
  if (kind === "audio") return `<audio class="media" controls preload="metadata" src="${esc(src)}" aria-label="${esc(label || "Audio")}"></audio>`;
  return `<img src="${esc(src)}" alt="${esc(label)}"${size} loading="lazy">`;
}

md.renderer.rules.image = (tokens, idx, options, env, self) => {
  const t = tokens[idx];
  let alt = self.renderInlineAsText(t.children ?? [], options, env);
  let width = dimension(t.attrGet("width")), height = dimension(t.attrGet("height"));
  // Obsidian: ![alt|300](…) and ![alt|300x200](…)
  const m = /^(.*?)\|\s*(\d{1,4})(?:\s*x\s*(\d{1,4}))?\s*$/.exec(alt);
  if (m) {
    alt = m[1].trim();
    width = m[2];
    height = m[3] ?? height;
  }
  return media(attached(t.attrGet("src") ?? "", env ?? {}), alt, width, height, env ?? {});
};

// [text](report.pdf) links to the attached file of that name.
const linkOpen = md.renderer.rules.link_open ?? ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));
md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  const href = tokens[idx].attrGet("href");
  if (href) tokens[idx].attrSet("href", attached(href, env ?? {}));
  return linkOpen(tokens, idx, options, env, self);
};

// GitLab: ![alt](…){width=300 height=200}
md.core.ruler.after("inline", "image-attributes", (state) => {
  for (const block of state.tokens) {
    const kids = block.children;
    if (block.type !== "inline" || !kids) continue;
    for (let i = 0; i < kids.length - 1; i++) {
      if (kids[i].type !== "image" || kids[i + 1].type !== "text") continue;
      const m = /^\{([^}\n]{1,80})\}/.exec(kids[i + 1].content);
      if (!m) continue;
      const w = /width\s*=\s*"?(\d{1,4}%?)/.exec(m[1])?.[1], h = /height\s*=\s*"?(\d{1,4}%?)/.exec(m[1])?.[1];
      if (!w && !h) continue;
      if (w) kids[i].attrSet("width", w.replace(/px$/, ""));
      if (h) kids[i].attrSet("height", h.replace(/px$/, ""));
      kids[i + 1].content = kids[i + 1].content.slice(m[0].length);
    }
  }
});

// Headings get ids so a table of contents and links to a section can point at them.
md.core.ruler.after("inline", "heading-ids", (state) => {
  const used = new Map<string, number>();
  state.tokens.forEach((t, i) => {
    if (t.type !== "heading_open") return;
    const text = state.tokens[i + 1]?.children?.filter((c) => c.type === "text" || c.type === "code_inline").map((c) => c.content).join("") ?? "";
    const base = text.toLowerCase().trim().replace(/[^\p{L}\p{N}\s-]/gu, "").replace(/\s+/g, "-").slice(0, 80) || "section";
    const n = used.get(base) ?? 0;
    used.set(base, n + 1);
    t.attrSet("id", n ? `${base}-${n}` : base);
  });
});

const CALLOUTS: Record<string, string> = {
  note: "Note", info: "Info", tip: "Tip", hint: "Tip", important: "Important", success: "Success", question: "Question", warning: "Warning", caution: "Caution",
  attention: "Warning", danger: "Danger", error: "Error", bug: "Bug", example: "Example", quote: "Quote", abstract: "Summary", summary: "Summary", todo: "To do",
};
// GitHub alerts and Obsidian callouts: a block quote that begins with [!note].
md.core.ruler.after("inline", "callouts", (state) => {
  const tokens = state.tokens;
  for (let i = 0; i < tokens.length - 2; i++) {
    if (tokens[i].type !== "blockquote_open" || tokens[i + 1].type !== "paragraph_open" || tokens[i + 2].type !== "inline") continue;
    const inline = tokens[i + 2];
    const first = inline.children?.[0];
    const m = first?.type === "text" ? /^\[!([a-zA-Z]+)\][+-]?[ \t]*/.exec(first.content) : null;
    if (!m || !first || !inline.children) continue;
    const type = m[1].toLowerCase();
    const known = CALLOUTS[type] ? type : "note";
    tokens[i].attrSet("class", `callout callout-${known}`);
    first.content = first.content.slice(m[0].length);
    // The rest of the first line is the title; without one, the kind names itself.
    const brk = inline.children.findIndex((c) => c.type === "softbreak" || c.type === "hardbreak");
    const titleTokens = inline.children.slice(0, brk < 0 ? inline.children.length : brk);
    const rest = brk < 0 ? [] : inline.children.slice(brk + 1);
    const open = new state.Token("html_inline", "", 0), close = new state.Token("html_inline", "", 0);
    const empty = titleTokens.every((c) => c.type === "text" && !c.content.trim());
    open.content = `<span class="callout-title">${empty ? esc(CALLOUTS[known]) : ""}`;
    close.content = rest.length ? "</span><br>" : "</span>";
    inline.children = [open, ...(empty ? [] : titleTokens), close, ...rest];
  }
});

function unresolved(text: string): string {
  return `<span class="unresolved" title="This does not exist, or you cannot see it">${esc(text)}</span>`;
}

function personLink(p: NonNullable<Refs["users"]>[string], handle: string): string {
  const agent = p.kind === "agent";
  return `<a class="mention${agent ? " agent" : ""}" href="/people/${esc(p.id)}" title="${esc(agent ? `${p.name} (AI agent)` : p.name)}" data-kind="${agent ? "agent" : "human"}">@${esc(p.name || handle)}</a>`;
}

function itemLink(i: NonNullable<Refs["items"]>[string], label?: string): string {
  return `<a class="ref ref-item" href="${esc(i.url)}" title="${esc(i.title)}" data-status="${esc(i.status_category ?? "")}">${esc(label ?? i.key)}</a>`;
}

/** [[target]], [[target|label]] and the embedding form ![[target|size]]. */
function wikilink(inner: string, embed: boolean, ctx: RenderContext): string {
  const cut = inner.indexOf("|");
  const target = (cut < 0 ? inner : inner.slice(0, cut)).trim();
  const label = cut < 0 ? "" : inner.slice(cut + 1).trim();
  const refs = ctx.refs ?? {};
  if (!embed && /^_?TOC_?$/i.test(target)) return `<nav class="toc" data-toc="1" aria-label="Contents"></nav>`;
  if (/^\d{4}-\d{2}-\d{2}$/.test(target) && !Number.isNaN(Date.parse(target))) return `<time class="chip" datetime="${target}">${esc(label || day(target))}</time>`;
  const viz = /^viz:([0-9a-f-]{36})$/i.exec(target);
  if (viz) {
    const h = /^\d{2,4}$/.test(label) ? label : "360";
    return embed ? `<div class="embed" data-viz="${viz[1]}" data-height="${h}"></div>` : `<a class="ref" href="/analytics/v/${viz[1]}">${esc(label || "Visualization")}</a>`;
  }
  const file = ctx.attachments?.find((a) => a.filename.toLowerCase() === target.toLowerCase());
  if (file) {
    const src = `/api/v1/attachments/${file.id}`;
    if (!embed) return `<a class="ref ref-file" href="${src}">${esc(label || file.filename)}</a>`;
    if (/^(image|video|audio)\//.test(file.content_type)) {
      const m = /^(\d{1,4})(?:\s*x\s*(\d{1,4}))?$/.exec(label);
      return media(src, m ? file.filename : label || file.filename, m?.[1] ?? null, m?.[2] ?? null, ctx);
    }
    return `<a class="ref ref-file" href="${src}">${esc(file.filename)}</a>`;
  }
  const item = refs.items?.[target.toUpperCase()];
  if (item) return embed ? `<a class="ref ref-item card" href="${esc(item.url)}" data-status="${esc(item.status_category ?? "")}"><span class="key">${esc(item.key)}</span> ${esc(item.title)}</a>` : itemLink(item, label || `${item.key} ${item.title}`);
  const project = refs.projects?.[target.toUpperCase()];
  if (project) return `<a class="ref ref-project" href="${esc(project.url)}">${esc(label || project.name)}</a>`;
  const page = refs.pages?.[target] ?? Object.entries(refs.pages ?? {}).find(([k]) => k.toLowerCase() === target.toLowerCase())?.[1];
  if (page) return `<a class="ref ref-page" href="${esc(page.url)}">${esc(label || page.title)}</a>`;
  return unresolved(label || target);
}

md.inline.ruler.before("link", "wikilink", (state, silent) => {
  const src = state.src, pos = state.pos;
  const embed = src.charCodeAt(pos) === 0x21 && src.startsWith("[[", pos + 1);
  if (!embed && !src.startsWith("[[", pos)) return false;
  const start = pos + (embed ? 3 : 2);
  const end = src.indexOf("]]", start);
  if (end <= start || end - start > 300) return false;
  const inner = src.slice(start, end);
  if (inner.includes("\n") || inner.includes("[")) return false;
  if (!silent) {
    const t = state.push("wikilink", "", 0);
    t.content = inner;
    t.meta = { embed };
  }
  state.pos = end + 2;
  return true;
});
md.renderer.rules.wikilink = (tokens, idx, _options, env) => wikilink(tokens[idx].content, !!tokens[idx].meta?.embed, env ?? {});

// Math: $…$, $$…$$ and GitLab's $`…`$.
md.inline.ruler.after("escape", "math", (state, silent) => {
  const src = state.src, start = state.pos;
  if (src.charCodeAt(start) !== 0x24) return false;
  let inner: string, end: number, block = false;
  if (src[start + 1] === "`") {
    const close = src.indexOf("`$", start + 2);
    if (close < 0) return false;
    inner = src.slice(start + 2, close);
    end = close + 2;
  } else if (src[start + 1] === "$") {
    const close = src.indexOf("$$", start + 2);
    if (close < 0) return false;
    inner = src.slice(start + 2, close);
    end = close + 2;
    block = true;
  } else {
    let close = start + 1;
    for (;;) {
      close = src.indexOf("$", close);
      if (close < 0) return false;
      if (src[close - 1] !== "\\") break;
      close++;
    }
    inner = src.slice(start + 1, close);
    end = close + 1;
    // "$5 and $6" is money, not a formula.
    if (!inner || /^\s|\s$/.test(inner) || inner.includes("\n") || /\d/.test(src[end] ?? "")) return false;
  }
  if (!inner.trim() || inner.length > 4000) return false;
  if (!silent) {
    const t = state.push("math", "", 0);
    t.content = inner.trim();
    t.meta = { block };
  }
  state.pos = end;
  return true;
});
md.renderer.rules.math = (tokens, idx) => `<span class="math${tokens[idx].meta?.block ? " math-block" : ""}" data-math="${tokens[idx].meta?.block ? "block" : "inline"}">${esc(tokens[idx].content)}</span>`;

// GitLab inline diff: {+added+} and {-removed-}.
md.inline.ruler.before("emphasis", "inline-diff", (state, silent) => {
  const src = state.src, pos = state.pos;
  if (src.charCodeAt(pos) !== 0x7b) return false;
  const sign = src[pos + 1];
  if (sign !== "+" && sign !== "-") return false;
  const end = src.indexOf(`${sign}}`, pos + 2);
  if (end < 0 || src.slice(pos + 2, end).includes("\n")) return false;
  if (!silent) {
    const tag = sign === "+" ? "ins" : "del";
    state.push("inline_diff_open", tag, 1).attrSet("class", sign === "+" ? "diff-add" : "diff-del");
    state.push("text", "", 0).content = src.slice(pos + 2, end).trim();
    state.push("inline_diff_close", tag, -1);
  }
  state.pos = end + 2;
  return true;
});

const IN_TEXT = /(^|[^\w@/.-])@([a-z0-9](?:[a-z0-9._-]{0,38}[a-z0-9])?)|\b([A-Z][A-Z0-9]{1,9}-\d{1,9})\b|(^|[\s(])#([A-Za-z][\w/-]{1,60})/gi;
const KEY = /^[A-Z][A-Z0-9]{1,9}-\d{1,9}$/;
const HEX = /^([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;

// Mentions, work item keys and tags inside ordinary text (not inside links or code).
md.core.ruler.after("inline", "references", (state) => {
  const refs: Refs = state.env?.refs ?? {};
  for (const block of state.tokens) {
    if (block.type !== "inline" || !block.children) continue;
    let depth = 0;
    const out: Token[] = [];
    for (const t of block.children) {
      if (t.type === "link_open") depth++;
      if (t.type === "link_close") depth--;
      if (t.type !== "text" || depth > 0) {
        out.push(t);
        continue;
      }
      const text = t.content;
      let last = 0;
      IN_TEXT.lastIndex = 0;
      let m: RegExpExecArray | null;
      const push = (type: "text" | "html_inline", content: string) => {
        const tok = new state.Token(type, "", 0);
        tok.content = content;
        out.push(tok);
      };
      while ((m = IN_TEXT.exec(text))) {
        let lead = "", html = "";
        if (m[2] !== undefined) {
          const person = refs.users?.[m[2].toLowerCase()];
          if (!person) continue;
          lead = m[1];
          html = personLink(person, m[2]);
        } else if (m[3] !== undefined) {
          const item = KEY.test(m[3]) ? refs.items?.[m[3]] : undefined;
          if (!item) continue;
          html = itemLink(item);
        } else {
          if (HEX.test(m[5])) continue;
          lead = m[4];
          html = `<span class="tag">#${esc(m[5])}</span>`;
        }
        if (m.index + lead.length > last) push("text", text.slice(last, m.index + lead.length));
        push("html_inline", html);
        last = m.index + m[0].length;
      }
      if (last === 0) out.push(t);
      else if (last < text.length) push("text", text.slice(last));
    }
    block.children = out;
  }
});

// GitLab: a color written as code shows a swatch.
const inlineCode = md.renderer.rules.code_inline!;
md.renderer.rules.code_inline = (tokens, idx, options, env, self) => {
  const c = tokens[idx].content;
  const html = inlineCode(tokens, idx, options, env, self);
  return /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(c) ? `${html}<span class="swatch" data-color="${esc(c)}"></span>` : html;
};

const fence = md.renderer.rules.fence!;
md.renderer.rules.fence = (tokens, idx, options, env, self) => {
  const t = tokens[idx];
  const [kind, ...rest] = t.info.trim().split(/\s+/);
  const height = /(?:^|\s)height=(\d{2,4})/.exec(t.info)?.[1] ?? "";
  switch (kind) {
    case "mermaid":
      return `<pre class="mermaid-src" data-mermaid="1"><code>${esc(t.content)}</code></pre>`;
    case "math":
      return `<div class="math math-block" data-math="block">${esc(t.content.trim())}</div>`;
    case "diagram":
      return `<div class="embed-diagram" data-diagram="1">${esc(t.content)}</div>`;
    // Markup with its own styles and scripts. It is shown inside the sandbox, never in this page.
    case "embed":
      return `<div class="embed" data-embed="html" data-height="${height}">${esc(t.content)}</div>`;
    // A chart script for one of the bundled libraries.
    case "viz": {
      const library = ["chartjs", "echarts", "plotly", "d3"].includes(rest[0]) ? rest[0] : "chartjs";
      return `<div class="embed" data-embed="script" data-library="${library}" data-height="${height || "360"}">${esc(t.content)}</div>`;
    }
    default:
      return fence(tokens, idx, options, env, self);
  }
};

/** What has to happen before parsing: properties, comments, >>> quotes, [TOC]. Code is left alone. */
function prepare(source: string): { text: string; front: string | null } {
  const lines = (source ?? "").replace(/\r\n?/g, "\n").split("\n");
  let front: string | null = null;
  if (lines[0] === "---") {
    const end = lines.indexOf("---", 1);
    if (end > 0 && end <= 80) {
      front = lines.slice(1, end).join("\n");
      lines.splice(0, end + 1);
    }
  }
  const out: string[] = [];
  let fenceMark: string | null = null, quote = false, comment = false;
  for (let line of lines) {
    const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (f && (!fenceMark || line.trim().startsWith(fenceMark))) {
      fenceMark = fenceMark ? null : f[1];
      out.push(quote ? `> ${line}` : line);
      continue;
    }
    if (fenceMark) {
      out.push(quote ? `> ${line}` : line);
      continue;
    }
    if (line.trim() === ">>>") {
      quote = !quote;
      out.push("");
      continue;
    }
    // Obsidian comments: %%hidden%%, also across lines.
    if (line.includes("%%") || comment) {
      const parts = line.split("%%");
      let kept = "";
      parts.forEach((part, i) => {
        if (!comment) kept += part;
        if (i < parts.length - 1) comment = !comment;
      });
      line = kept;
    }
    if (/^\s*\[TOC\]\s*$/i.test(line)) line = "[[_TOC_]]";
    out.push(quote ? `> ${line}` : line);
  }
  return { text: out.join("\n"), front };
}

// ───────────────────────── sanitizing ─────────────────────────

// Classes are how the application's own parts are styled. Content may use the ones this
// module produces, and none of the application's (a comment cannot dress up as a button).
const CLASS_OK = /^(language-[\w+-]+|callout|callout-[a-z]+|callout-title|mention|agent|ref|ref-[a-z]+|card|key|tag|chip|math|math-block|mermaid-src|footnote[\w-]*|footnotes(-sep|-list)?|toc|embed|embed-diagram|media|unresolved|swatch|frontmatter|diff-add|diff-del)$/;
let ownRender = false;

DOMPurify.addHook("uponSanitizeAttribute", (_node, data) => {
  if (!ownRender || data.attrName !== "class") return;
  data.attrValue = data.attrValue.split(/\s+/).filter((c) => CLASS_OK.test(c)).join(" ");
  if (!data.attrValue) data.keepAttr = false;
});

DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    const href = node.getAttribute("href") ?? "";
    // Links leaving the application open in a new tab without handing over the opener.
    if (/^https?:\/\//i.test(href) && !href.startsWith(location.origin)) {
      node.setAttribute("target", "_blank");
      node.setAttribute("rel", "noopener noreferrer");
    }
    // Ids in content are prefixed (see SANITIZE_NAMED_PROPS); links to them must follow.
    if (ownRender && href.startsWith("#") && !href.startsWith("#user-content-")) node.setAttribute("href", `#user-content-${href.slice(1)}`);
  }
  if (!ownRender) return;
  // The only form control in content is the check box of a task list, and it is for show.
  if (node.tagName === "INPUT") {
    if (node.getAttribute("type") !== "checkbox") node.remove();
    else {
      node.setAttribute("disabled", "");
      // Named for screen readers: the text of the task follows right after the box.
      node.setAttribute("aria-label", (node as HTMLInputElement).hasAttribute("checked") ? "Task, done" : "Task, not done");
    }
    return;
  }
  // Pictures and recordings come from this server (or are inline data). The page's policy
  // would refuse anything else; a link is more use than a broken picture, and fetches nothing.
  if (["IMG", "VIDEO", "AUDIO", "SOURCE"].includes(node.tagName)) {
    const src = node.getAttribute("src") ?? "";
    if (/^(https?:)?\/\//i.test(src) && !src.startsWith(`${location.origin}/`)) {
      const a = document.createElement("a");
      a.href = src;
      a.target = "_blank";
      a.rel = "noopener noreferrer";
      a.textContent = node.getAttribute("alt") || node.getAttribute("aria-label") || src;
      node.replaceWith(a);
    }
  }
});

const PURIFY = {
  ALLOWED_TAGS: [
    "h1", "h2", "h3", "h4", "h5", "h6", "p", "br", "hr", "strong", "b", "em", "i", "u", "s", "del", "ins", "mark", "small", "sub", "sup", "kbd", "abbr", "code", "pre", "blockquote",
    "ul", "ol", "li", "dl", "dt", "dd", "a", "img", "video", "audio", "source", "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption", "input", "span", "div", "label",
    "details", "summary", "figure", "figcaption", "time", "nav", "section",
  ],
  ALLOWED_ATTR: [
    "href", "title", "src", "alt", "class", "id", "type", "checked", "disabled", "colspan", "rowspan", "align", "target", "rel", "width", "height", "loading", "controls", "preload",
    "datetime", "open", "start", "lang", "dir",
  ],
  // DOMPurify tests every attribute value against this, not only links: it must accept plain
  // values ("checkbox", class names) as well as http(s) and mailto URLs, and nothing with another scheme.
  ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto):|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i,
  // An id in content cannot shadow one the application relies on.
  SANITIZE_NAMED_PROPS: true,
};

function sanitize(html: string): string {
  ownRender = true;
  try {
    return DOMPurify.sanitize(html, PURIFY) as string;
  } finally {
    ownRender = false;
  }
}

/** Render Markdown to HTML that is safe to insert into the page. */
export function render(source: string, ctx: RenderContext = {}): string {
  const { text, front } = prepare(source);
  let html = md.render(text, { refs: ctx.refs ?? {}, attachments: ctx.attachments ?? [] });
  // Show task items with a (read-only) checkbox.
  html = html.replace(/<li data-type="taskItem" data-checked="(true|false)">/g, (_m, c) => `<li data-type="taskItem" data-checked="${c}"><input type="checkbox" disabled${c === "true" ? " checked" : ""}>`);
  if (front) html = `<details class="frontmatter"><summary>Properties</summary><pre><code>${esc(front)}</code></pre></details>${html}`;
  return sanitize(html);
}

/** HTML for loading into the simple editor: task items in the structure the editor expects. */
export function renderForEditor(source: string): string {
  const html = DOMPurify.sanitize(basic.render(source ?? ""), { ...PURIFY, SANITIZE_NAMED_PROPS: false }) as string;
  return html.replace(/<pre class="mermaid-src" data-mermaid="1"><code>/g, '<pre><code class="language-mermaid">');
}

const turndown = new TurndownService({ headingStyle: "atx", codeBlockStyle: "fenced", bulletListMarker: "-", emDelimiter: "*", hr: "---" });
turndown.use(gfm);
turndown.addRule("taskItem", {
  filter: (node) => node.nodeName === "LI" && (node as HTMLElement).getAttribute("data-type") === "taskItem",
  replacement: (content, node) => {
    const checked = (node as HTMLElement).getAttribute("data-checked") === "true";
    return `- [${checked ? "x" : " "}] ${content.trim().replace(/\n{2,}/g, "\n").replace(/\n/g, "\n  ")}\n`;
  },
});
// One space after the marker ("- item", "1. item"), with continuation lines indented to match.
turndown.addRule("compactListItem", {
  filter: (node) => node.nodeName === "LI" && (node as HTMLElement).getAttribute("data-type") !== "taskItem",
  replacement: (content, node) => {
    const parent = node.parentNode as HTMLElement | null;
    let prefix = "- ";
    if (parent?.nodeName === "OL") {
      const start = Number(parent.getAttribute("start") ?? 1) || 1;
      prefix = `${start + Array.prototype.indexOf.call(parent.children, node)}. `;
    }
    const body = content.replace(/^\n+/, "").replace(/\n+$/, "").replace(/\n/g, `\n${" ".repeat(prefix.length)}`);
    return `${prefix}${body}${node.nextSibling ? "\n" : ""}`;
  },
});
turndown.addRule("fencedWithLanguage", {
  filter: (node) => node.nodeName === "PRE" && !!node.firstChild && node.firstChild.nodeName === "CODE",
  replacement: (_content, node) => {
    const code = node.firstChild as HTMLElement;
    const lang = (code.getAttribute("class") ?? "").match(/language-(\S+)/)?.[1] ?? "";
    return `\n\n\`\`\`${lang}\n${(code.textContent ?? "").replace(/\n$/, "")}\n\`\`\`\n\n`;
  },
});
// The editor wraps table cell text in paragraphs; Markdown tables need the bare text.
turndown.addRule("cellParagraph", {
  filter: (node) => node.nodeName === "P" && !!node.parentNode && ["TD", "TH"].includes(node.parentNode.nodeName),
  replacement: (content) => content,
});
// What the full dialect reads as syntax must survive the simple editor as the same characters.
const escapeText = turndown.escape.bind(turndown);
turndown.escape = (text: string) => escapeText(text).replace(/\\\[\\\[([^[\]\n\\]{1,300})\\\]\\\]/g, "[[$1]]");

/** Convert the simple editor's HTML back to Markdown. */
export function toMarkdown(html: string): string {
  return turndown.turndown(html).replace(/\n{3,}/g, "\n\n").trim() + "\n";
}

/** Constructs the simple editor cannot represent; editing such a page there would lose them. */
export function needsAdvancedEditor(source: string): string | null {
  if (/^\s*```mermaid/m.test(source)) return "a diagram";
  if (/^\s*```diagram/m.test(source)) return "a drawn diagram";
  if (/^\s*```(embed|viz)\b/m.test(source)) return "embedded content";
  if (/^\s*```math/m.test(source) || /\$\$[\s\S]+?\$\$/.test(source)) return "formulas";
  if (/^\[\^[^\]]+\]:/m.test(source)) return "footnotes";
  if (/^>\s*\[![a-zA-Z]+\]/m.test(source)) return "callouts";
  if (/^---\n[\s\S]*?\n---\n/.test(source)) return "properties";
  if (/<\/?[a-z][^>]*>/i.test(source.replace(/```[\s\S]*?```/g, "").replace(/`[^`]*`/g, ""))) return "raw HTML";
  return null;
}

// ───────────────────────── drawing ─────────────────────────

let mermaidReady: Promise<typeof import("mermaid").default> | null = null;

/** Replace ```mermaid blocks inside `root` with rendered diagrams. Loaded on first use. */
export async function renderDiagrams(root: HTMLElement) {
  const blocks = Array.from(root.querySelectorAll<HTMLElement>("pre[data-mermaid]"));
  if (blocks.length === 0) return;
  mermaidReady ??= import("mermaid").then((m) => m.default);
  const mermaid = await mermaidReady;
  mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: document.documentElement.dataset.theme === "dark" ? "dark" : "neutral", fontFamily: "IBM Plex Sans, system-ui, sans-serif" });
  let n = 0;
  for (const block of blocks) {
    const holder = document.createElement("div");
    holder.className = "mermaid";
    try {
      const { svg } = await mermaid.render(`mmd-${Date.now()}-${n++}`, block.textContent ?? "");
      holder.innerHTML = DOMPurify.sanitize(svg, { USE_PROFILES: { svg: true, svgFilters: true }, ADD_TAGS: ["foreignObject"] }) as string;
    } catch (e) {
      holder.textContent = `This diagram could not be drawn: ${e instanceof Error ? e.message.split("\n")[0] : "syntax error"}`;
      holder.classList.add("error-text");
    }
    block.replaceWith(holder);
  }
}

let katexReady: Promise<typeof import("katex").default> | null = null;

async function renderMath(root: HTMLElement) {
  const spots = Array.from(root.querySelectorAll<HTMLElement>("[data-math]"));
  if (spots.length === 0) return;
  katexReady ??= import("katex").then((m) => m.default);
  const katex = await katexReady;
  for (const el of spots) {
    const source = el.textContent ?? "";
    try {
      // MathML only: browsers draw it themselves, and screen readers can read it.
      katex.render(source, el, { displayMode: el.dataset.math === "block", output: "mathml", throwOnError: true, trust: false, strict: "ignore", maxExpand: 500 });
      el.setAttribute("title", source);
    } catch (e) {
      el.classList.add("error-text");
      el.title = e instanceof Error ? e.message : "This formula could not be read.";
    }
    el.removeAttribute("data-math");
  }
}

function renderDrawn(root: HTMLElement) {
  for (const el of Array.from(root.querySelectorAll<HTMLElement>("[data-diagram]"))) {
    try {
      const svg = diagramSvg(parseDiagram(el.textContent ?? ""));
      el.textContent = "";
      el.appendChild(svg);
    } catch {
      el.textContent = "This diagram could not be drawn: its data is not valid.";
      el.classList.add("error-text");
    }
    el.removeAttribute("data-diagram");
  }
}

function contents(root: HTMLElement) {
  for (const nav of Array.from(root.querySelectorAll<HTMLElement>("nav[data-toc]"))) {
    const list = document.createElement("ul");
    for (const h of Array.from(root.querySelectorAll<HTMLElement>("h1[id], h2[id], h3[id], h4[id]"))) {
      const li = document.createElement("li");
      li.style.marginLeft = `${(Number(h.tagName[1]) - 1) * 14}px`;
      const a = document.createElement("a");
      a.href = `#${h.id}`;
      a.textContent = h.textContent;
      li.appendChild(a);
      list.appendChild(li);
    }
    nav.textContent = "";
    if (list.children.length) nav.appendChild(list);
    nav.removeAttribute("data-toc");
  }
  for (const s of Array.from(root.querySelectorAll<HTMLElement>(".swatch[data-color]"))) s.style.background = s.dataset.color ?? "";
}

function note(el: HTMLElement, text: string) {
  el.textContent = text;
  el.classList.add("muted", "small");
}

/** Start the sandboxed frames for ```embed and ```viz blocks and for saved visualizations. */
function mountEmbeds(root: HTMLElement): SandboxHandle[] {
  const handles: SandboxHandle[] = [];
  for (const el of Array.from(root.querySelectorAll<HTMLElement>(".embed[data-embed], .embed[data-viz]"))) {
    const height = Number(el.dataset.height) || 0;
    const report = { error: (message: string) => el.setAttribute("data-error", message) };
    if (el.dataset.embed) {
      const source = el.textContent ?? "";
      const script = el.dataset.embed === "script";
      el.textContent = "";
      if (height) el.style.height = `${Math.min(1200, Math.max(80, height))}px`;
      handles.push(mountSandbox(el, { kind: script ? "script" : "html", source, library: script ? el.dataset.library : "html", mode: height || script ? "tile" : "flow", title: script ? "Chart" : "Embedded content" }, report));
      el.removeAttribute("data-embed");
    } else if (el.dataset.viz) {
      const id = el.dataset.viz;
      el.removeAttribute("data-viz");
      el.style.height = `${Math.min(1200, Math.max(120, height || 360))}px`;
      note(el, "Loading…");
      void fetch(`/api/v1/visualizations/${id}`, { credentials: "same-origin", headers: { Accept: "application/json" } })
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
        .then((v) => {
          if (!el.isConnected) return;
          el.textContent = "";
          el.classList.remove("muted", "small");
          handles.push(mountSandbox(el, { kind: v.library === "html" ? "html" : "script", source: v.source, library: v.library, params: v.params, mode: "tile", title: v.name }, report));
        })
        .catch(() => note(el, "This visualization does not exist, or has not been shared with you."));
    }
  }
  return handles;
}

/**
 * Draw what `render()` left as placeholders. Returns a function that stops what it started;
 * call it before the content is replaced.
 */
export function enhance(root: HTMLElement): () => void {
  contents(root);
  renderDrawn(root);
  void renderMath(root);
  void renderDiagrams(root);
  const frames = mountEmbeds(root);
  return () => frames.forEach((f) => f.destroy());
}
