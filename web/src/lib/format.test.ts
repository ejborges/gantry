import { describe, expect, it } from "vitest";
import { bytes, day, duration, initials, isOverdue, priorityLabel, relative, slaState, titleCase } from "./format";

const NOW = new Date("2026-03-10T12:00:00Z").getTime();

describe("relative", () => {
  it("describes recent and distant times", () => {
    expect(relative("2026-03-10T11:59:40Z", NOW)).toBe("just now");
    expect(relative("2026-03-10T11:30:00Z", NOW)).toMatch(/30 minutes ago/);
    expect(relative("2026-03-10T09:00:00Z", NOW)).toMatch(/3 hours ago/);
    expect(relative("2026-03-12T12:00:00Z", NOW)).toMatch(/in 2 days/);
    expect(relative("2025-03-10T12:00:00Z", NOW)).toMatch(/last year|1 year ago/);
  });
  it("is empty for a missing value", () => {
    expect(relative(null)).toBe("");
    expect(relative(undefined)).toBe("");
  });
});

describe("calendar dates", () => {
  it("does not shift a date across time zones", () => {
    // 2026-11-06 must show as the 6th wherever the browser is.
    expect(day("2026-11-06")).toMatch(/6/);
    expect(day("2026-11-06")).not.toMatch(/\b5\b/);
    expect(day(null)).toBe("");
  });
  it("flags only open items whose date has passed", () => {
    const today = new Date(2026, 2, 10);
    expect(isOverdue("2026-03-09", false, today)).toBe(true);
    expect(isOverdue("2026-03-10", false, today)).toBe(false);
    expect(isOverdue("2026-03-09", true, today)).toBe(false);
    expect(isOverdue(null, false, today)).toBe(false);
  });
});

describe("service-level state", () => {
  const due = "2026-03-10T15:00:00Z";
  it("counts down, warns under two hours, and reports misses", () => {
    expect(slaState(due, false, null, NOW)).toEqual({ state: "ok", label: "3 h left" });
    expect(slaState(due, false, null, NOW + 2 * 3600_000)).toEqual({ state: "soon", label: "1 h left" });
    expect(slaState(due, false, null, NOW + 4 * 3600_000)).toEqual({ state: "late", label: "Overdue 1 h" });
    expect(slaState(due, true, null, NOW)?.state).toBe("late");
  });
  it("reports how a finished target ended", () => {
    expect(slaState(due, false, "2026-03-10T14:00:00Z", NOW)).toEqual({ state: "met", label: "Met" });
    expect(slaState(due, true, "2026-03-10T16:00:00Z", NOW)).toEqual({ state: "late", label: "Missed" });
    expect(slaState(null, false, null, NOW)).toBeNull();
  });
});

describe("small formatters", () => {
  it("formats durations", () => {
    expect(duration(0)).toBe("0 min");
    expect(duration(59 * 60000)).toBe("59 min");
    expect(duration(90 * 60000)).toBe("1 h 30 min");
    expect(duration(2 * 3600_000)).toBe("2 h");
    expect(duration(72 * 3600_000)).toBe("3 d");
    expect(duration(-5)).toBe("0 min");
  });
  it("formats sizes, initials, priorities and labels", () => {
    expect(bytes(512)).toBe("512 B");
    expect(bytes(2048)).toBe("2.0 KB");
    expect(bytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(initials("Ada Okafor")).toBe("AO");
    expect(initials("  morgan  ")).toBe("M");
    expect(initials("Jean Luc Picard")).toBe("JP");
    expect(initials("")).toBe("?");
    expect(priorityLabel(4)).toBe("Critical");
    expect(priorityLabel(99)).toBe("None");
    expect(titleCase("in_progress")).toBe("In progress");
    expect(titleCase("item.create")).toBe("Item create");
  });
});
