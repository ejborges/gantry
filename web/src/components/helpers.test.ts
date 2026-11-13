// The small pure functions behind the calendar, the comparison view, the Gantt chart, the
// hierarchy graph and the image tools of the editor.
import { describe, expect, it } from "vitest";
import { firstWeekday, monthGrid } from "./Calendar";
import { diffRows, diffStats } from "./DiffView";
import { addDays, cssColor } from "./Gantt";
import { buildTree, ringOf, statusWord, type GraphNode } from "./HierarchyGraph";
import { diagramBlock, imageAt, resizeImage } from "../editors/MarkdownEditor";

describe("the calendar", () => {
  it("shows whole weeks, starting where the reader's weeks start", () => {
    const sunday = monthGrid(2026, 10, 0); // November 2026 starts on a Sunday
    expect(sunday[0]).toBe("2026-11-01");
    expect(sunday.length % 7).toBe(0);
    expect(sunday).toContain("2026-11-30");
    const monday = monthGrid(2026, 10, 1);
    expect(monday[0]).toBe("2026-10-26");
    expect(monday[6]).toBe("2026-11-01");
    expect(monthGrid(2028, 1, 0)).toContain("2028-02-29"); // a leap year
  });

  it("starts the week on Sunday in the United States and on Monday in most of Europe", () => {
    expect(firstWeekday("en-US")).toBe(0);
    expect(firstWeekday("de-DE")).toBe(1);
    expect(firstWeekday("en-GB")).toBe(1);
  });
});

describe("comparing two versions of a text", () => {
  it("pairs a removed line with the line that replaced it", () => {
    const rows = diffRows("one\ntwo\nthree\n", "one\n2\nthree\nfour\n");
    expect(rows.map((r) => r.kind)).toEqual(["same", "change", "same", "add"]);
    expect(rows[1].left).toEqual({ n: 2, text: "two" });
    expect(rows[1].right).toEqual({ n: 2, text: "2" });
    expect(rows[3].right).toEqual({ n: 4, text: "four" });
    expect(diffStats(rows)).toEqual({ added: 2, removed: 1 });
  });

  it("reports no changes for identical texts and all lines for a new text", () => {
    expect(diffStats(diffRows("a\nb\n", "a\nb\n"))).toEqual({ added: 0, removed: 0 });
    expect(diffRows("", "a\nb\n").filter((r) => r.kind === "add")).toHaveLength(2);
    expect(diffRows("a\nb\n", "").every((r) => r.kind === "del" || (r.kind === "same" && r.left?.text === ""))).toBe(true);
  });
});

describe("Gantt dates and colors", () => {
  it("moves a date by days without a time zone shifting it", () => {
    expect(addDays("2026-03-08", 1)).toBe("2026-03-09"); // the day clocks change in the United States
    expect(addDays("2026-11-01", -1)).toBe("2026-10-31");
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(addDays("2028-02-28", 2)).toBe("2028-03-01");
  });

  it("uses named colors from the theme and exact colors as given", () => {
    expect(cssColor("red")).toBe("var(--danger)");
    expect(cssColor("#12ab34")).toBe("#12ab34");
    expect(cssColor("no-such-color")).toBe(cssColor("neutral"));
    expect(cssColor(undefined)).toBe(cssColor("neutral"));
  });
});

describe("the hierarchy graph", () => {
  const node = (id: string, kind: GraphNode["kind"], label: string, extra: Partial<GraphNode> = {}): GraphNode => ({ id, kind, level: kind, label, title: label, status: "todo", person: null, url: "/", ...extra });
  const nodes = [node("c:1", "container", "Program"), node("c:2", "container", "Initiative"), node("p:1", "project", "PUMP"), node("p:2", "project", "IT"), node("i:1", "item", "PUMP-1"), node("i:2", "item", "PUMP-2"), node("i:10", "item", "PUMP-10")];
  const edges = [{ from: "c:1", to: "c:2" }, { from: "c:2", to: "p:1" }, { from: "p:1", to: "i:1" }, { from: "i:1", to: "i:2" }, { from: "p:1", to: "i:10" }];

  it("hangs everything from one root, containers before projects before items", () => {
    const t = buildTree(nodes, edges, new Set(), "Workspace");
    expect(t.children.map((c) => c.label)).toEqual(["Program", "IT"]);
    const pump = t.children[0].children[0].children[0];
    expect(pump.label).toBe("PUMP");
    expect(pump.children.map((c) => c.label)).toEqual(["PUMP-1", "PUMP-10"]); // by number, not by text
    expect(pump.children[0].children.map((c) => c.label)).toEqual(["PUMP-2"]);
  });

  it("folds a branch and says how much is in it", () => {
    const t = buildTree(nodes, edges, new Set(["p:1"]), "Workspace");
    const pump = t.children[0].children[0].children[0];
    expect(pump.children).toEqual([]);
    expect(pump.hidden).toBe(2);
  });

  it("survives edges to things that are not there, a second parent, and a loop", () => {
    const odd = [...edges, { from: "p:9", to: "i:1" }, { from: "p:2", to: "i:1" }, { from: "i:2", to: "i:1" }, { from: "i:2", to: "c:1" }];
    const t = buildTree(nodes, odd, new Set(), "Workspace");
    const count = (n: { children: any[] }): number => 1 + n.children.reduce((s, c) => s + count(c), 0);
    expect(count(t)).toBe(nodes.length + 1); // every node once, plus the root
  });

  it("puts the levels above projects into the same three states as work", () => {
    expect(ringOf("active")).toBe("in_progress");
    expect(ringOf("planned")).toBe("todo");
    expect(ringOf("on_hold")).toBe("todo");
    expect(ringOf("done")).toBe("done");
    expect(statusWord({ status: "on_hold" })).toBe("On hold");
    expect(statusWord({ status: "todo", status_name: "Backlog" })).toBe("Backlog");
  });
});

describe("the image tools of the editor", () => {
  it("finds the picture under the cursor and its width", () => {
    const line = "Before ![plot|240](plot.png) and ![[photo.jpg|120]] after";
    expect(imageAt(line, 10)).toMatchObject({ alt: "plot", target: "plot.png", wiki: false, width: 240 });
    expect(imageAt(line, 40)).toMatchObject({ target: "photo.jpg", wiki: true, width: 120 });
    expect(imageAt(line, 2)).toBeNull();
    expect(imageAt("![no size](a.png)", 3)).toMatchObject({ width: null });
  });

  it("writes the picture again at a suggested, custom or original size", () => {
    const img = imageAt("![plot|240](plot.png)", 3)!;
    expect(resizeImage(img, 480)).toBe("![plot|480](plot.png)");
    expect(resizeImage(img, null)).toBe("![plot](plot.png)");
    const wiki = imageAt("![[photo.jpg]]", 3)!;
    expect(resizeImage(wiki, 800)).toBe("![[photo.jpg|800]]");
    expect(resizeImage(wiki, null)).toBe("![[photo.jpg]]");
  });

  it("finds the diagram block the cursor is in", () => {
    const lines = ["Text", "```diagram", '{"nodes":[],"edges":[]}', "```", "More", "```js", "x", "```"];
    expect(diagramBlock(lines, 3)).toEqual({ from: 2, to: 4, body: '{"nodes":[],"edges":[]}' });
    expect(diagramBlock(lines, 2)).toMatchObject({ from: 2, to: 4 });
    expect(diagramBlock(lines, 1)).toBeNull();
    expect(diagramBlock(lines, 5)).toBeNull();
    expect(diagramBlock(lines, 7)).toBeNull(); // inside another kind of block
  });
});
