import { describe, expect, it } from "vitest";
import { ANY, arrange, deadEnds, fromMoves, keyFor, problems, removedStages, toMoves, unreachable, type WorkflowDef } from "./workflow";

const wf: WorkflowDef = {
  initial: "todo",
  statuses: [
    { key: "todo", name: "To do", category: "todo" },
    { key: "doing", name: "Doing", category: "in_progress" },
    { key: "review", name: "In review", category: "in_progress" },
    { key: "done", name: "Done", category: "done" },
  ],
  transitions: [
    { name: "Start", from: ["todo"], to: "doing" },
    { name: "Ask for review", from: ["doing"], to: "review" },
    { name: "Finish", from: ["doing", "review"], to: "done", requires: ["resolution"] },
    { name: "Reopen", from: [ANY], to: "todo" },
  ],
};

describe("workflow moves", () => {
  it("splits a transition into one move per starting stage and merges them back", () => {
    const moves = toMoves(wf.transitions);
    expect(moves).toHaveLength(5);
    expect(moves.filter((m) => m.name === "Finish").map((m) => m.from)).toEqual(["doing", "review"]);
    expect(fromMoves(moves)).toEqual(wf.transitions);
  });

  it("does not merge moves that differ in what they require", () => {
    const out = fromMoves([
      { id: "a", name: "Finish", from: "doing", to: "done", requires: [] },
      { id: "b", name: "Finish", from: "review", to: "done", requires: ["resolution"] },
    ]);
    expect(out).toHaveLength(2);
  });

  it("lets a move from any stage absorb the same move from a named one", () => {
    const out = fromMoves([
      { id: "a", name: "Cancel", from: "doing", to: "done", requires: [] },
      { id: "b", name: "Cancel", from: ANY, to: "done", requires: [] },
    ]);
    expect(out).toEqual([{ name: "Cancel", from: [ANY], to: "done" }]);
  });
});

describe("keys for new stages", () => {
  it("are lowercase, start with a letter, and do not repeat", () => {
    expect(keyFor("In Review", [])).toBe("in_review");
    expect(keyFor("In Review", ["in_review"])).toBe("in_review_2");
    expect(keyFor("2nd look", [])).toBe("s_2nd_look");
    expect(keyFor("Überprüfung!", [])).toMatch(/^[a-z][a-z0-9_]*$/);
    expect(keyFor("???", [])).toBe("stage");
  });
});

describe("checks before saving", () => {
  it("accepts a sound workflow", () => {
    expect(problems(wf)).toEqual([]);
    expect(unreachable(wf)).toEqual([]);
    expect(deadEnds(wf)).toEqual([]);
  });

  it("says what the server would refuse", () => {
    expect(problems({ ...wf, statuses: [] })).toEqual(["Add at least one stage."]);
    expect(problems({ ...wf, initial: "gone" })).toContain("Choose the stage where new work starts.");
    expect(problems({ ...wf, statuses: wf.statuses.filter((s) => s.category !== "done"), transitions: [] })).toContain("At least one stage must count as done.");
    expect(problems({ ...wf, transitions: [{ name: "", from: ["todo"], to: "doing" }] })).toContain("Every move needs a name.");
    expect(problems({ ...wf, transitions: [{ name: "Go", from: ["todo"], to: "nowhere" }] })).toContain("A move leads to or from a stage that no longer exists.");
  });

  it("finds stages nothing leads to and stages nothing leaves", () => {
    const broken: WorkflowDef = { ...wf, transitions: [{ name: "Start", from: ["todo"], to: "doing" }] };
    expect(unreachable(broken).map((s) => s.key)).toEqual(["review", "done"]);
    expect(deadEnds(broken).map((s) => s.key)).toEqual(["doing", "review"]);
  });

  it("lists the stages a change takes away, for mapping the work in them", () => {
    const after = { ...wf, statuses: wf.statuses.filter((s) => s.key !== "review") };
    expect(removedStages(wf, after).map((s) => s.key)).toEqual(["review"]);
    expect(removedStages(wf, wf)).toEqual([]);
  });
});

describe("laying out the canvas", () => {
  it("runs left to right in the order work flows", () => {
    const at = arrange(wf.statuses, toMoves(wf.transitions));
    expect(at.todo.x).toBeLessThan(at.doing.x);
    expect(at.doing.x).toBeLessThan(at.done.x);
  });

  it("still runs left to right when every move starts from any stage", () => {
    const any = wf.statuses.map((s) => ({ id: s.key, name: s.name, from: ANY, to: s.key, requires: [] }));
    const at = arrange(wf.statuses, any);
    const xs = wf.statuses.map((s) => at[s.key].x);
    expect([...xs].sort((a, b) => a - b)).toEqual(xs);
    expect(new Set(xs).size).toBe(xs.length);
  });

  it("keeps the places someone chose", () => {
    const at = arrange(wf.statuses, [], { todo: { x: 500, y: 40 } });
    expect(at.todo).toEqual({ x: 500, y: 40 });
  });
});
