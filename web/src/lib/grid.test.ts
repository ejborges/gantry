import { describe, expect, it } from "vitest";
import { changed, clamp, compact, overlaps, placeNew, settle, type Box } from "./grid";

const box = (id: string, x: number, y: number, w: number, h: number): Box => ({ id, x, y, w, h });
const noOverlap = (tiles: Box[]) => tiles.every((a) => tiles.every((b) => !overlaps(a, b)));

describe("the dashboard grid", () => {
  it("keeps a tile inside twelve columns and at least one cell large", () => {
    expect(clamp(box("a", 10, 0, 6, 4))).toMatchObject({ x: 6, w: 6 });
    expect(clamp(box("a", -3, -2, 0, 0))).toMatchObject({ x: 0, y: 0, w: 1, h: 1 });
    expect(clamp(box("a", 2.4, 1.6, 30, 30))).toMatchObject({ x: 0, y: 2, w: 12, h: 12 });
  });

  it("pushes down what a moved tile lands on, and what that lands on in turn", () => {
    const tiles = [box("a", 0, 0, 6, 2), box("b", 0, 2, 6, 2), box("c", 0, 4, 6, 2), box("d", 6, 0, 6, 2)];
    const out = settle(tiles, { ...tiles[0], y: 1 });
    expect(noOverlap(out)).toBe(true);
    expect(out.find((t) => t.id === "a")).toMatchObject({ y: 1 });
    expect(out.find((t) => t.id === "b")).toMatchObject({ y: 3 });
    expect(out.find((t) => t.id === "c")).toMatchObject({ y: 5 });
    expect(out.find((t) => t.id === "d")).toBe(tiles[3]); // untouched tiles are the same objects
  });

  it("settles a crowded layout without leaving overlaps", () => {
    // Regression: the first version read the queue while searching it and lost its place.
    const tiles = [box("kpi", 0, 0, 12, 2), box("status", 0, 2, 4, 4), box("flow", 4, 2, 8, 4), box("load", 0, 6, 6, 5), box("due", 6, 6, 6, 5)];
    const out = settle(tiles, { ...tiles[1], x: 1 });
    expect(noOverlap(out)).toBe(true);
    expect(out).toHaveLength(5);
  });

  it("closes gaps without changing the order", () => {
    const out = compact([box("a", 0, 3, 6, 2), box("b", 6, 9, 6, 2), box("c", 0, 20, 12, 1)]);
    expect(out.map((t) => t.y)).toEqual([0, 0, 2]);
    expect(noOverlap(out)).toBe(true);
  });

  it("places a new tile beside the last row when it fits, otherwise beneath everything", () => {
    expect(placeNew([], 6, 4)).toEqual({ x: 0, y: 0, w: 6, h: 4 });
    expect(placeNew([box("a", 0, 0, 6, 4)], 6, 4)).toEqual({ x: 6, y: 0, w: 6, h: 4 });
    expect(placeNew([box("a", 0, 0, 8, 4)], 6, 4)).toEqual({ x: 0, y: 4, w: 6, h: 4 });
  });

  it("reports only the tiles whose place or size differs", () => {
    const before = [box("a", 0, 0, 6, 2), box("b", 6, 0, 6, 2)];
    expect(changed(before, [before[0], { ...before[1], h: 3 }]).map((t) => t.id)).toEqual(["b"]);
    expect(changed(before, [...before, box("c", 0, 2, 4, 4)]).map((t) => t.id)).toEqual(["c"]);
  });
});
