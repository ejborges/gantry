import { describe, expect, it } from "vitest";
import { borderPoint, diagramSvg, parseDiagram, serializeDiagram, wrapText } from "./diagram";

describe("diagrams written as text", () => {
  it("keeps only what a diagram may contain", () => {
    const d = parseDiagram(JSON.stringify({
      nodes: [{ id: "a", x: 0, y: 0, w: 120, h: 50, shape: "diamond", text: "Go?", color: "amber", onclick: "alert(1)" }, { id: "b", x: 1e9, y: "x", w: 1, h: 1, shape: "<script>", text: 5, color: "url(evil)" }],
      edges: [{ id: "e", from: "a", to: "b", label: "yes", style: "dashed", arrow: "both" }, { from: "a", to: "nowhere" }, { from: "a", to: "a" }],
      extra: "ignored",
    }));
    expect(d.nodes).toHaveLength(2);
    expect(d.nodes[0]).toEqual({ id: "a", x: 0, y: 0, w: 120, h: 50, shape: "diamond", text: "Go?", color: "amber" });
    expect(d.nodes[1]).toMatchObject({ x: 20000, y: 0, w: 30, h: 20, shape: "rect", text: "", color: "neutral" });
    expect(d.edges).toHaveLength(1); // edges to nothing and to itself are dropped
    expect(d.edges[0]).toMatchObject({ from: "a", to: "b", label: "yes", style: "dashed", arrow: "both" });
  });

  it("refuses what is not a diagram", () => {
    for (const bad of ["", "[]", "{}", '{"nodes":"x"}', "not json"]) expect(() => parseDiagram(bad)).toThrow();
  });

  it("writes a diagram back so that reading it gives the same diagram", () => {
    const d = parseDiagram('{"nodes":[{"id":"a","x":10.4,"y":20.6,"w":150,"h":56,"shape":"round","text":"Start","color":"green"},{"id":"b","x":300,"y":20,"w":150,"h":56,"shape":"rect","text":"End"}],"edges":[{"id":"e1","from":"a","to":"b","label":"then"}]}');
    const again = parseDiagram(serializeDiagram(d));
    expect(again.nodes.map((n) => [n.id, n.x, n.y, n.shape, n.text, n.color])).toEqual([["a", 10, 21, "round", "Start", "green"], ["b", 300, 20, "rect", "End", "neutral"]]);
    expect(again.edges).toHaveLength(1);
  });
});

describe("drawing", () => {
  it("draws text as text, so markup in a label cannot become markup on the page", () => {
    const svg = diagramSvg(parseDiagram('{"nodes":[{"id":"a","x":0,"y":0,"w":160,"h":60,"shape":"rect","text":"<img src=x onerror=alert(1)>"}],"edges":[]}'));
    expect(svg.querySelector("img")).toBeNull();
    expect(svg.textContent).toContain("<img src=x");
    expect(svg.getAttribute("role")).toBe("img");
  });

  it("starts a connection on the outline of a shape, toward the other shape", () => {
    const n = { id: "a", x: 0, y: 0, w: 100, h: 40, shape: "rect" as const, text: "" };
    expect(borderPoint(n, 500, 20)).toEqual({ x: 100, y: 20 });
    expect(borderPoint(n, 50, -300)).toEqual({ x: 50, y: 0 });
  });

  it("breaks long text into lines that fit", () => {
    const lines = wrapText("Replace the capacitor on every rev C board", 120);
    expect(lines.length).toBeGreaterThan(1);
    expect(lines.join(" ")).toBe("Replace the capacitor on every rev C board");
    expect(wrapText("", 100)).toEqual([""]);
  });
});
