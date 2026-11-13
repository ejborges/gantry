// A month calendar for choosing a date. Works with the keyboard: arrows move by day and
// week, Page Up and Page Down by month, Home and End to the ends of the week, Enter chooses.
import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { today } from "../lib/format";

const pad = (n: number) => String(n).padStart(2, "0");
const iso = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const parse = (s: string) => {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
};

/** 0 = Sunday … 6 = Saturday, as weeks start where the reader is. */
export function firstWeekday(locale = navigator.language): number {
  try {
    const l = new Intl.Locale(locale) as Intl.Locale & { weekInfo?: { firstDay: number }; getWeekInfo?: () => { firstDay: number } };
    const day = (l.getWeekInfo?.() ?? l.weekInfo)?.firstDay;
    if (day) return day % 7;
  } catch {
    /* fall through to the convention for the language's region */
  }
  return /-(US|CA|MX|BR|JP|IL|IN|PH|ZA)$/i.test(locale) || locale === "en" ? 0 : 1;
}

/** The days shown for a month: whole weeks, so some belong to the months before and after. */
export function monthGrid(year: number, month: number, weekStart: number): string[] {
  const first = new Date(year, month, 1);
  const lead = (first.getDay() - weekStart + 7) % 7;
  const days = new Date(year, month + 1, 0).getDate();
  const cells = Math.ceil((lead + days) / 7) * 7;
  return Array.from({ length: cells }, (_, i) => iso(new Date(year, month, 1 - lead + i)));
}

export default function Calendar({ value, onPick, min, max, label = "Choose a date" }: { value?: string | null; onPick: (date: string) => void; min?: string; max?: string; label?: string }) {
  const now = today();
  const [cursor, setCursor] = useState(value && /^\d{4}-\d{2}-\d{2}$/.test(value) ? value : now);
  const grid = useRef<HTMLDivElement>(null);
  const moved = useRef(false);
  const weekStart = useMemo(() => firstWeekday(), []);
  const at = parse(cursor);
  const days = useMemo(() => monthGrid(at.getFullYear(), at.getMonth(), weekStart), [at.getFullYear(), at.getMonth(), weekStart]); // eslint-disable-line react-hooks/exhaustive-deps
  const names = useMemo(() => Array.from({ length: 7 }, (_, i) => new Date(2024, 0, 7 + ((weekStart + i) % 7))), [weekStart]);
  const allowed = (d: string) => (!min || d >= min) && (!max || d <= max);
  const move = (days: number, months = 0) => {
    const d = parse(cursor);
    if (months) {
      const day = d.getDate();
      d.setDate(1);
      d.setMonth(d.getMonth() + months);
      d.setDate(Math.min(day, new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate()));
    }
    d.setDate(d.getDate() + days);
    moved.current = true;
    setCursor(iso(d));
  };
  useEffect(() => {
    if (moved.current) grid.current?.querySelector<HTMLButtonElement>(`[data-day="${cursor}"]`)?.focus();
  }, [cursor]);
  const month = at.toLocaleDateString(undefined, { month: "long", year: "numeric" });
  return (
    <div className="calendar" role="group" aria-label={label}>
      <div className="calendar-head">
        <button type="button" className="btn ghost icon sm" aria-label="Previous month" onClick={() => move(0, -1)}>
          <ChevronLeft />
        </button>
        <strong aria-live="polite">{month}</strong>
        <button type="button" className="btn ghost icon sm" aria-label="Next month" onClick={() => move(0, 1)}>
          <ChevronRight />
        </button>
      </div>
      <div
        className="calendar-grid"
        role="grid"
        aria-label={month}
        ref={grid}
        onKeyDown={(e) => {
          const step: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [-7, 0], ArrowDown: [7, 0], PageUp: [0, -1], PageDown: [0, 1] };
          if (step[e.key]) {
            e.preventDefault();
            move(...step[e.key]);
          } else if (e.key === "Home" || e.key === "End") {
            e.preventDefault();
            const offset = (parse(cursor).getDay() - weekStart + 7) % 7;
            move(e.key === "Home" ? -offset : 6 - offset);
          }
        }}
      >
        <div role="row" className="calendar-row">
          {names.map((d) => (
            <span key={d.getDay()} role="columnheader" aria-label={d.toLocaleDateString(undefined, { weekday: "long" })} className="calendar-dow">
              {d.toLocaleDateString(undefined, { weekday: "narrow" })}
            </span>
          ))}
        </div>
        {Array.from({ length: days.length / 7 }, (_, w) => (
          <div role="row" className="calendar-row" key={w}>
            {days.slice(w * 7, w * 7 + 7).map((d) => {
              const date = parse(d);
              const outside = date.getMonth() !== at.getMonth();
              return (
                <button
                  type="button"
                  role="gridcell"
                  key={d}
                  data-day={d}
                  tabIndex={d === cursor ? 0 : -1}
                  disabled={!allowed(d)}
                  aria-selected={d === value}
                  aria-current={d === now ? "date" : undefined}
                  aria-label={date.toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" })}
                  className={`calendar-day ${outside ? "outside" : ""}`}
                  onClick={() => onPick(d)}
                >
                  {date.getDate()}
                </button>
              );
            })}
          </div>
        ))}
      </div>
      <div className="calendar-foot">
        <button type="button" className="btn sm" disabled={!allowed(now)} onClick={() => onPick(now)}>
          Today
        </button>
        <span className="small muted">{value ? parse(value).toLocaleDateString(undefined, { dateStyle: "medium" }) : "No date chosen"}</span>
      </div>
    </div>
  );
}
