import { describe, expect, it } from "vitest";
import { needsAdvancedEditor, render, renderForEditor, toMarkdown } from "./markdown";

const dom = (html: string) => {
  const d = document.createElement("div");
  d.innerHTML = html;
  return d;
};

describe("render", () => {
  it("renders headings, emphasis, lists, tables and code", () => {
    const d = dom(render("# Title\n\nSome **bold** and *italic* text.\n\n- one\n- two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```js\nlet x = 1;\n```\n"));
    expect(d.querySelector("h1")?.textContent).toBe("Title");
    expect(d.querySelector("strong")?.textContent).toBe("bold");
    expect(d.querySelectorAll("li")).toHaveLength(2);
    expect(d.querySelectorAll("td")).toHaveLength(2);
    expect(d.querySelector("pre code")?.textContent).toContain("let x = 1;");
  });

  it("shows task lists as read-only checkboxes", () => {
    // Regression: the sanitizer once stripped type="checkbox", which turned these into text inputs.
    const d = dom(render("- [ ] open\n- [x] finished\n"));
    const boxes = d.querySelectorAll<HTMLInputElement>("input");
    expect(boxes).toHaveLength(2);
    for (const b of boxes) {
      expect(b.getAttribute("type")).toBe("checkbox");
      expect(b.disabled).toBe(true);
    }
    expect(boxes[0].checked).toBe(false);
    expect(boxes[1].checked).toBe(true);
    expect(d.querySelector("ul")?.getAttribute("data-type")).toBe("taskList");
    expect(d.textContent).not.toContain("[x]");
  });

  it("opens outside links in a new tab without an opener", () => {
    const a = dom(render("[site](https://example.com/a) and [here](/docs/ENG)")).querySelectorAll("a");
    expect(a[0].getAttribute("target")).toBe("_blank");
    expect(a[0].getAttribute("rel")).toBe("noopener noreferrer");
    expect(a[1].getAttribute("target")).toBeNull();
    expect(a[1].getAttribute("href")).toBe("/docs/ENG");
  });

  it("keeps a diagram as inert source for the viewer to draw", () => {
    const d = dom(render("```mermaid\ngraph TD; A-->B;\n```\n"));
    const pre = d.querySelector("pre[data-mermaid]");
    expect(pre?.textContent).toContain("A-->B");
    expect(d.querySelector("svg")).toBeNull();
  });
});

describe("sanitizing untrusted Markdown", () => {
  const hostile = [
    `<script>window.pwned = 1</script>`,
    `<img src=x onerror="window.pwned = 1">`,
    `<svg onload="window.pwned = 1"></svg>`,
    `<iframe src="https://evil.example"></iframe>`,
    `<a href="javascript:window.pwned=1">click</a>`,
    `[click](javascript:window.pwned=1)`,
    `[click](JaVaScRiPt:window.pwned=1)`,
    `[click](data:text/html,<script>window.pwned=1</script>)`,
    `[click](vbscript:msgbox(1))`,
    `![x](javascript:window.pwned=1)`,
    `<form action="https://evil.example"><input name="password"><button>Go</button></form>`,
    `<style>body { display: none }</style>`,
    `<div style="position:fixed;inset:0" onclick="window.pwned=1">cover</div>`,
    `<object data="x"></object><embed src="x">`,
    `<math><mi xlink:href="javascript:window.pwned=1">x</mi></math>`,
  ];

  it.each(hostile)("neutralises %s", (payload) => {
    const html = render(payload);
    const d = dom(html);
    expect(d.querySelector("script, iframe, object, embed, style, form, svg, math, button")).toBeNull();
    for (const el of d.querySelectorAll("*")) {
      for (const attr of el.getAttributeNames()) {
        expect(attr.startsWith("on"), `${attr} on <${el.tagName.toLowerCase()}>`).toBe(false);
        expect(attr).not.toBe("style");
        expect(el.getAttribute(attr) ?? "").not.toMatch(/^\s*(javascript|data|vbscript):/i);
      }
    }
    expect((window as any).pwned).toBeUndefined();
  });

  it("keeps http, https and mailto links", () => {
    const a = dom(render("[a](http://example.com) [b](https://example.com) [c](mailto:ops@example.com)")).querySelectorAll("a");
    expect([...a].map((x) => x.getAttribute("href"))).toEqual(["http://example.com", "https://example.com", "mailto:ops@example.com"]);
  });

  it("keeps a safe subset of HTML typed into Markdown and drops what could act", () => {
    const d = dom(render("before <b onclick=\"x()\" style=\"position:fixed\" class=\"btn danger\">bold</b> after <kbd>Ctrl</kbd>\n\n<details><summary>More</summary>Hidden</details>\n\n<script>alert(1)</script><style>body{display:none}</style><iframe src=\"https://evil.example\"></iframe><form action=\"https://evil.example\"><input name=\"password\"><button>Sign in</button></form><object data=\"x\"></object><base href=\"https://evil.example\"><meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\"><link rel=\"stylesheet\" href=\"https://evil.example/x.css\">"));
    const b = d.querySelector("b")!;
    expect(b.textContent).toBe("bold");
    expect(b.attributes.length).toBe(0);
    expect(d.querySelector("kbd")?.textContent).toBe("Ctrl");
    expect(d.querySelector("details summary")?.textContent).toBe("More");
    for (const tag of ["script", "style", "iframe", "form", "button", "object", "base", "meta", "link", "input"]) expect(d.querySelector(tag), tag).toBeNull();
    expect(d.innerHTML).not.toContain("alert(1)");
    expect(d.innerHTML).not.toContain("evil.example");
  });
});

describe("the simple editor round trip", () => {
  const cases: [string, string][] = [
    ["heading and paragraph", "## Plan\n\nShip it on **Friday**, not *Thursday*.\n"],
    ["bullet list", "- one\n- two\n- three\n"],
    ["numbered list", "1. first\n2. second\n"],
    ["link", "See [the runbook](https://example.com/runbook).\n"],
    ["quote", "> measure twice\n"],
    ["code block with a language", "```rust\nfn main() {}\n```\n"],
    ["inline code", "Run `gantry doctor` first.\n"],
    ["table", "| Board | Resets |\n| --- | --- |\n| Rev B | 0 |\n| Rev C | 1.5 |\n"],
    ["task list", "- [ ] open\n- [x] finished\n"],
    ["nested list", "- parent\n  - child one\n  - child two\n- sibling\n"],
    ["numbered list with nested bullets", "1. first\n   - detail\n2. second\n"],
  ];
  it.each(cases)("keeps a %s", (_name, source) => {
    const once = toMarkdown(renderForEditor(source));
    expect(once).toBe(source);
    // Saving again without changes must not alter the text.
    expect(toMarkdown(renderForEditor(once))).toBe(once);
  });

  it("hands a diagram to the editor as an ordinary code block and back", () => {
    const source = "```mermaid\ngraph TD; A-->B;\n```\n";
    expect(renderForEditor(source)).toContain('<code class="language-mermaid">');
    expect(toMarkdown(renderForEditor(source))).toBe(source);
  });
});

describe("needsAdvancedEditor", () => {
  it("names what the simple editor would lose", () => {
    expect(needsAdvancedEditor("```mermaid\ngraph TD;\n```")).toBe("a diagram");
    expect(needsAdvancedEditor("Text[^1]\n\n[^1]: note")).toBe("footnotes");
    expect(needsAdvancedEditor("<details>more</details>")).toBe("raw HTML");
  });
  it("allows ordinary pages, including angle brackets inside code", () => {
    expect(needsAdvancedEditor("# Title\n\n- [ ] task\n\n| a |\n|---|\n| b |\n")).toBeNull();
    expect(needsAdvancedEditor("Use `<div>` here\n\n```html\n<p>x</p>\n```\n")).toBeNull();
  });
});

describe("references to work, pages and people", () => {
  const refs = {
    items: { "OPS-12": { key: "OPS-12", title: "Fix the pump", url: "/i/OPS-12", status_category: "done", status_name: "Done" } },
    users: { ada: { id: "u1", name: "Ada Okafor", kind: "human", handle: "ada" }, bot: { id: "u2", name: "Triage agent", kind: "agent", handle: "bot" } },
    pages: { "Release process": { id: "p1", title: "Release process", url: "/docs/ENG/p1" } },
  } as any;

  it("links only what the server resolved for this reader", () => {
    const d = dom(render("See OPS-12 and OPS-99. [[Release process]] and [[Secret plan]].", { refs }));
    const links = [...d.querySelectorAll("a")].map((a) => [a.textContent, a.getAttribute("href")]);
    expect(links).toEqual([["OPS-12", "/i/OPS-12"], ["Release process", "/docs/ENG/p1"]]);
    // What the reader may not see stays text: no link, and nothing that tells whether it exists.
    expect(d.textContent).toContain("OPS-99");
    expect(d.querySelector(".unresolved")?.textContent).toBe("Secret plan");
  });

  it("marks a mentioned AI agent as one", () => {
    const d = dom(render("Ask @ada or @bot, not @nobody.", { refs }));
    const mentions = d.querySelectorAll("a.mention");
    expect(mentions).toHaveLength(2);
    expect(mentions[0].textContent).toBe("@Ada Okafor");
    expect(mentions[1].classList.contains("agent")).toBe(true);
    expect(mentions[1].getAttribute("title")).toBe("Triage agent (AI agent)");
    expect(d.textContent).toContain("@nobody");
  });

  it("does not turn keys inside code into links", () => {
    const d = dom(render("`OPS-12` and\n\n```\nOPS-12 @ada\n```\n", { refs }));
    expect(d.querySelectorAll("a")).toHaveLength(0);
  });

  it("shows a date written as [[2026-03-31]] as a date", () => {
    const t = dom(render("Due [[2026-03-31]], not [[2026-13-45]].")).querySelector("time");
    expect(t?.getAttribute("datetime")).toBe("2026-03-31");
    expect(t?.textContent).toMatch(/2026/);
  });
});

describe("the three Markdown dialects", () => {
  it("renders GitHub alerts and Obsidian callouts alike", () => {
    const d = dom(render("> [!NOTE]\n> Body\n\n> [!warning] Mind the gap\n> Text"));
    const boxes = d.querySelectorAll("blockquote.callout");
    expect(boxes).toHaveLength(2);
    expect(boxes[0].classList.contains("callout-note")).toBe(true);
    expect(boxes[1].querySelector(".callout-title")?.textContent).toBe("Mind the gap");
  });

  it("renders highlights, GitLab additions and removals, tags, and drops Obsidian comments", () => {
    const d = dom(render("==mark== {+added+} {-removed-} %%hidden%% #release"));
    expect(d.querySelector("mark")?.textContent).toBe("mark");
    expect(d.querySelector("ins")?.textContent).toBe("added");
    expect(d.querySelector("del")?.textContent).toBe("removed");
    expect(d.querySelector(".tag")?.textContent).toBe("#release");
    expect(d.textContent).not.toContain("hidden");
  });

  it("marks math, Mermaid diagrams and the table of contents for the page to draw", () => {
    const d = dom(render("[[_TOC_]]\n\n# One\n\nInline $a^2$ and $`b^2`$\n\n```math\nc = d\n```\n\n```mermaid\ngraph TD; A-->B;\n```\n"));
    expect(d.querySelector("nav.toc")).not.toBeNull();
    expect(d.querySelectorAll(".math")).toHaveLength(3);
    expect(d.querySelector(".math-block")?.textContent).toBe("c = d");
    expect(d.querySelector("pre[data-mermaid]")?.textContent).toContain("A-->B");
    expect(d.querySelector("h1")?.id).toBe("user-content-one");
  });

  it("renders footnotes and front matter", () => {
    const d = dom(render("---\ntitle: X\n---\n\nText[^1]\n\n[^1]: Note"));
    expect(d.querySelector("details.frontmatter")?.textContent).toContain("title: X");
    expect(d.querySelector(".footnotes li")?.textContent).toContain("Note");
  });
});

describe("pictures, sound, film and attached files", () => {
  const attachments = [
    { id: "11111111-1111-1111-1111-111111111111", filename: "plot.png", content_type: "image/png", size: 1 },
    { id: "22222222-2222-2222-2222-222222222222", filename: "run.mp4", content_type: "video/mp4", size: 1 },
    { id: "33333333-3333-3333-3333-333333333333", filename: "note.wav", content_type: "audio/wav", size: 1 },
    { id: "44444444-4444-4444-4444-444444444444", filename: "test report.pdf", content_type: "application/pdf", size: 1 },
  ] as any;

  it("scales a picture in each dialect's way", () => {
    const d = dom(render("![plot](plot.png){width=300}\n\n![plot|240](plot.png)\n\n![[plot.png|120]]\n\n![plot|200x100](plot.png)", { attachments }));
    const imgs = [...d.querySelectorAll("img")];
    expect(imgs.map((i) => i.getAttribute("width"))).toEqual(["300", "240", "120", "200"]);
    expect(imgs[3].getAttribute("height")).toBe("100");
    expect(imgs[1].getAttribute("alt")).toBe("plot");
  });

  it("finds an attached file by its name, in every way of writing it", () => {
    const d = dom(render("![a](plot.png) ![[plot.png]] [the report](test%20report.pdf) [elsewhere](other.pdf)", { attachments }));
    expect([...d.querySelectorAll("img")].map((i) => i.getAttribute("src"))).toEqual(["/api/v1/attachments/11111111-1111-1111-1111-111111111111", "/api/v1/attachments/11111111-1111-1111-1111-111111111111"]);
    const links = [...d.querySelectorAll("a")].map((a) => a.getAttribute("href"));
    expect(links).toEqual(["/api/v1/attachments/44444444-4444-4444-4444-444444444444", "other.pdf"]);
  });

  it("plays sound and film with the browser's own player", () => {
    const d = dom(render("![clip](run.mp4)\n\n![[note.wav]]", { attachments }));
    const video = d.querySelector("video")!, audio = d.querySelector("audio")!;
    expect(video.hasAttribute("controls")).toBe(true);
    expect(video.getAttribute("src")).toBe("/api/v1/attachments/22222222-2222-2222-2222-222222222222");
    expect(video.hasAttribute("autoplay")).toBe(false);
    expect(audio.getAttribute("src")).toBe("/api/v1/attachments/33333333-3333-3333-3333-333333333333");
  });

  it("never loads a picture from another site: it becomes a link", () => {
    const d = dom(render("![tracker](https://evil.example/pixel.png) ![also](//evil.example/x.png)"));
    expect(d.querySelector("img")).toBeNull();
    expect(d.querySelector("a")?.getAttribute("rel")).toContain("noopener");
  });
});

describe("custom content", () => {
  it("keeps embedded HTML and chart scripts as inert text for the sandbox", () => {
    const d = dom(render("```embed\n<b>hi</b><script>alert(1)</script>\n```\n\n```viz chartjs\nnew Chart(document.body)\n```\n\n![[viz:75c4160c-5cc2-4c4e-8405-84318d6c2457|300]]"));
    expect(d.querySelector("script")).toBeNull();
    expect(d.querySelector("b")).toBeNull(); // not part of the page: it is text until the sandbox runs it
    const html = d.querySelector('[data-embed="html"]')!;
    expect(html.textContent).toContain("<script>alert(1)</script>");
    const script = d.querySelector('[data-embed="script"]')!;
    expect(script.getAttribute("data-library")).toBe("chartjs");
    const saved = d.querySelector("[data-viz]")!;
    expect(saved.getAttribute("data-viz")).toBe("75c4160c-5cc2-4c4e-8405-84318d6c2457");
    expect(saved.getAttribute("data-height")).toBe("300");
  });

  it("loads only the bundled chart libraries, and takes only a real id for a saved chart", () => {
    const d = dom(render("```viz https://evil.example/lib.js\nx\n```\n\n![[viz:../../etc/passwd|300]]\n\n[[viz:75c4160c-5cc2-4c4e-8405-84318d6c2457|The chart]]"));
    expect(d.querySelector('[data-embed="script"]')?.getAttribute("data-library")).toBe("chartjs");
    expect(d.querySelector("[data-viz]")).toBeNull();
    expect(d.querySelector("a.ref")?.getAttribute("href")).toBe("/analytics/v/75c4160c-5cc2-4c4e-8405-84318d6c2457");
  });

  it("keeps a drawn diagram as data for the page to draw", () => {
    const d = dom(render('```diagram\n{"nodes":[{"id":"a","x":0,"y":0,"w":100,"h":40,"shape":"rect","text":"<b>A</b>"}],"edges":[]}\n```'));
    const el = d.querySelector("[data-diagram]")!;
    expect(el.querySelector("b")).toBeNull();
    expect(JSON.parse(el.textContent!).nodes[0].text).toBe("<b>A</b>");
  });

  it("drops everything in typed HTML that could act", () => {
    const d = dom(render('<details><summary>More</summary>Text</details>\n\n<kbd>Ctrl</kbd> <script>alert(1)</script> <img src=x onerror=alert(1)> <a href="javascript:alert(1)">x</a> <iframe src="https://evil.example"></iframe> <style>body{display:none}</style> <form action="https://evil.example"><input name=q></form> <object data="x"></object>'));
    expect(d.querySelector("details summary")?.textContent).toBe("More");
    expect(d.querySelector("kbd")).not.toBeNull();
    for (const gone of ["script", "iframe", "style", "form", "object", "input:not([type=checkbox])"]) expect(d.querySelector(gone), gone).toBeNull();
    expect(d.querySelector("img")?.hasAttribute("onerror")).toBe(false);
    expect(d.querySelector("a")?.hasAttribute("href")).toBe(false);
  });
});
