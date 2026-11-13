import { describe, expect, it } from "vitest";
import { complete, controlFor, dateWords, decodeQuery, defaultValue, describe as say, encodeQuery, forServer, fromSaved, newCriterion, type FieldInfo, type ItemQuery } from "./criteria";

const fields: FieldInfo[] = [
  { key: "title", kind: "text", label: "Name", ops: ["contains", "eq"] },
  { key: "assignee", kind: "user", label: "Assignee", ops: ["in", "not_in", "is_empty"] },
  { key: "due", kind: "date", label: "Due date", ops: ["on", "before", "between", "is_empty"] },
  { key: "priority", kind: "number", label: "Priority", ops: ["eq", "gte"] },
  { key: "label", kind: "labels", label: "Label", ops: ["in", "all"] },
  { key: "overdue", kind: "flag", label: "Overdue", ops: ["is"] },
  { key: "watcher", kind: "watcher", label: "Followed by", ops: ["in"] },
  { key: "custom.severity", kind: "select", label: "Severity", ops: ["in"], custom: true, options: ["Minor", "Major"] },
];
const f = (key: string) => fields.find((x) => x.key === key)!;
const label = (_: string, v: unknown) => (v === "me" ? "Me" : String(v));
const fmt = (iso: string) => `<${iso}>`;

describe("the control a line needs", () => {
  it("follows the kind of field and the comparison", () => {
    expect(controlFor(f("title"), "contains")).toBe("text");
    expect(controlFor(f("assignee"), "in")).toBe("people");
    expect(controlFor(f("assignee"), "is_empty")).toBe("none");
    expect(controlFor(f("due"), "on")).toBe("date");
    expect(controlFor(f("due"), "between")).toBe("dates");
    expect(controlFor(f("priority"), "gte")).toBe("priority");
    expect(controlFor(f("label"), "all")).toBe("choice");
    expect(controlFor(f("overdue"), "is")).toBe("flag");
    expect(controlFor(f("watcher"), "in")).toBe("none");
    expect(controlFor(f("custom.severity"), "in")).toBe("choice");
  });

  it("starts each line with a value of the right shape", () => {
    expect(newCriterion(f("title"))).toEqual({ field: "title", op: "contains", value: "" });
    expect(newCriterion(f("assignee")).value).toEqual([]);
    expect(newCriterion(f("overdue")).value).toBe(true);
    expect(newCriterion(f("watcher")).value).toEqual(["me"]);
    expect(defaultValue(f("due"), "between")).toEqual(["", ""]);
  });
});

describe("what is sent to the server", () => {
  const q: ItemQuery = {
    match: "any",
    criteria: [
      { field: "title", op: "contains", value: "  pump " },
      { field: "assignee", op: "in", value: [] }, // unfinished
      { field: "due", op: "between", value: ["2026-01-01", ""] }, // unfinished
      { field: "due", op: "before", value: { days: 7 } },
      { field: "assignee", op: "is_empty" },
      { field: "gone", op: "eq", value: "x" }, // a field that no longer exists
      { field: "priority", op: "gte", value: 3 },
    ],
    sort: [{ field: "due", dir: "asc" }, { field: "", dir: "asc" }],
  };

  it("leaves out unfinished lines and trims text", () => {
    const sent = forServer(q, fields);
    expect(sent.match).toBe("any");
    expect(sent.criteria).toEqual([
      { field: "title", op: "contains", value: "pump" },
      { field: "due", op: "before", value: { days: 7 } },
      { field: "assignee", op: "is_empty" },
      { field: "priority", op: "gte", value: 3 },
    ]);
    expect(sent.sort).toEqual([{ field: "due", dir: "asc" }]);
  });

  it("knows which lines are complete", () => {
    expect(complete({ field: "due", op: "on", value: "2026-03-31" }, f("due"))).toBe(true);
    expect(complete({ field: "due", op: "on", value: "31/03/2026" }, f("due"))).toBe(false);
    expect(complete({ field: "due", op: "on", value: { days: 0 } }, f("due"))).toBe(true);
    expect(complete({ field: "priority", op: "eq", value: null }, f("priority"))).toBe(false);
    expect(complete({ field: "title", op: "regex", value: "x" }, f("title"))).toBe(false);
  });
});

describe("a search in an address", () => {
  it("survives being written and read back", () => {
    const q: ItemQuery = { match: "any", criteria: [{ field: "assignee", op: "in", value: ["me"] }, { field: "overdue", op: "is", value: true }], sort: [{ field: "due", dir: "desc" }] };
    expect(decodeQuery(encodeQuery(q))).toEqual(q);
  });

  it("ignores anything that is not a search", () => {
    for (const bad of [null, "", "not json", "[]", "{}", '{"c":"x"}', '{"c":[["only-one"]]}']) {
      const out = decodeQuery(bad);
      if (out) expect(out.criteria).toEqual([]);
    }
    const out = decodeQuery('{"c":[["a","eq",1],[1,2,3],"x"],"s":[["due","sideways"],["x"]],"m":"neither"}')!;
    expect(out.criteria).toEqual([{ field: "a", op: "eq", value: 1 }]);
    expect(out.sort).toEqual([{ field: "due", dir: "asc" }, { field: "x", dir: "asc" }]);
    expect(out.match).toBe("all");
  });

  it("reads a saved search from the server", () => {
    expect(fromSaved({ match: "any", criteria: [{ field: "label", op: "in", value: ["a"] }], sort: [{ field: "key", dir: "desc" }] })).toEqual({ match: "any", criteria: [{ field: "label", op: "in", value: ["a"] }], sort: [{ field: "key", dir: "desc" }] });
    expect(fromSaved(null)).toEqual({ match: "all", criteria: [], sort: [] });
  });
});

describe("a line in plain words", () => {
  it("names the field, the comparison and the values", () => {
    expect(say({ field: "assignee", op: "in", value: ["me"] }, f("assignee"), label, fmt)).toBe("Assignee is any of Me");
    expect(say({ field: "assignee", op: "is_empty" }, f("assignee"), label, fmt)).toBe("Assignee is empty");
    expect(say({ field: "due", op: "before", value: "2026-03-31" }, f("due"), label, fmt)).toBe("Due date is before <2026-03-31>");
    expect(say({ field: "due", op: "between", value: [{ days: -7 }, { days: 0 }] }, f("due"), label, fmt)).toBe("Due date is between 7 days ago and today");
    expect(say({ field: "overdue", op: "is", value: false }, f("overdue"), label, fmt)).toBe("Not: overdue");
    expect(say({ field: "watcher", op: "in", value: ["me"] }, f("watcher"), label, fmt)).toBe("I follow it");
  });

  it("writes relative dates the way people say them", () => {
    expect(dateWords({ days: 0 }, fmt)).toBe("today");
    expect(dateWords({ days: -3 }, fmt)).toBe("3 days ago");
    expect(dateWords({ days: 14 }, fmt)).toBe("in 14 days");
    expect(dateWords("", fmt)).toBe("…");
  });
});

describe("links from notifications", () => {
  // The address the server puts in a due-date reminder (REMINDER_LINK in notify.rs).
  const link = "/search?find=%7B%22c%22%3A%5B%5B%22assignee%22%2C%22in%22%2C%5B%22me%22%5D%5D%2C%5B%22status_category%22%2C%22not_in%22%2C%5B%22done%22%5D%5D%2C%5B%22due%22%2C%22is_not_empty%22%5D%5D%2C%22s%22%3A%5B%5B%22due%22%2C%22asc%22%5D%5D%7D";
  it("a reminder opens the reader's own unfinished work, soonest first", () => {
    const q = decodeQuery(new URLSearchParams(link.split("?")[1]).get("find"));
    expect(q).toEqual({
      match: "all",
      criteria: [
        { field: "assignee", op: "in", value: ["me"] },
        { field: "status_category", op: "not_in", value: ["done"] },
        { field: "due", op: "is_not_empty", value: undefined },
      ],
      sort: [{ field: "due", dir: "asc" }],
    });
  });
});
