// Statistics for the owner of a program, an initiative or a project: the state of the work
// and the accounts doing it. Every figure is also in a table, so nothing depends on the charts.
import { useState } from "react";
import { Link } from "react-router-dom";
import { useGet } from "../api";
import { Avatar, Empty, Loaded, PersonChip } from "../ui";
import { day, relative, today } from "../lib/format";
import { ChartColumn } from "lucide-react";

interface Group {
  key: string;
  label: string;
  total: number;
  open: number;
  done: number;
}

function Tile({ label, value, tone, hint }: { label: string; value: React.ReactNode; tone?: string; hint?: string }) {
  return (
    <div className={`stat ${tone ?? ""}`} title={hint}>
      <div className="stat-value">{value}</div>
      <div className="stat-label">{label}</div>
    </div>
  );
}

/** Horizontal bars: finished and open parts of each group, with the numbers beside them. */
export function Bars({ groups, caption }: { groups: Group[]; caption: string }) {
  const max = Math.max(1, ...groups.map((g) => g.total));
  if (groups.length === 0) return <p className="muted small">Nothing to count.</p>;
  return (
    <figure className="bars-figure">
      <table className="bars">
        <caption className="sr-only">{caption}</caption>
        <thead>
          <tr>
            <th scope="col"><span className="sr-only">Group</span></th>
            <th scope="col"><span className="sr-only">Chart</span></th>
            <th scope="col"><span className="legend done" aria-hidden="true" />Done</th>
            <th scope="col"><span className="legend open" aria-hidden="true" />Open</th>
          </tr>
        </thead>
        <tbody>
          {groups.slice(0, 14).map((g) => (
            <tr key={g.key} title={`${g.label}: ${g.done} done, ${g.open} open`}>
              <th scope="row" className="truncate">{g.label}</th>
              <td className="bar-cell" aria-hidden="true">
                <span className="bar">
                  {g.done > 0 && <span className="done" style={{ width: `${(g.done / max) * 100}%` }} />}
                  {g.open > 0 && <span className="open" style={{ width: `${(g.open / max) * 100}%` }} />}
                </span>
              </td>
              <td className="num small nowrap">{g.done}</td>
              <td className="num small nowrap">{g.open}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {groups.length > 14 && <figcaption className="small muted">The 14 largest of {groups.length}.</figcaption>}
    </figure>
  );
}

/** A column with a rounded top and a square foot on the baseline. */
function column(x: number, w: number, h: number, base: number): string {
  if (h <= 0) return "";
  const r = Math.min(3, h, w / 2);
  return `M${x},${base} V${base - h + r} Q${x},${base - h} ${x + r},${base - h} H${x + w - r} Q${x + w},${base - h} ${x + w},${base - h + r} V${base} Z`;
}

function Weekly({ weeks }: { weeks: { week: string; created: number; resolved: number }[] }) {
  const [at, setAt] = useState<number | null>(null);
  const max = Math.max(1, ...weeks.flatMap((w) => [w.created, w.resolved]));
  const W = 30, H = 96, top = 6;
  const tick = (iso: string) => new Date(`${iso}T12:00:00`).toLocaleDateString(undefined, { month: "numeric", day: "numeric" });
  const shown = at === null ? null : weeks[at];
  return (
    <figure className="weekly">
      <figcaption className="small muted row wrap">
        <span className="legend created" aria-hidden="true" /> Created <span className="legend resolved" aria-hidden="true" /> Finished
        <span className="grow" />
        <span role="status" className="weekly-readout">
          {shown ? (
            <>
              Week of {day(shown.week)}: <strong>{shown.created}</strong> created, <strong>{shown.resolved}</strong> finished
            </>
          ) : (
            "By week. Point at a week for its numbers."
          )}
        </span>
      </figcaption>
      <svg viewBox={`0 0 ${weeks.length * W} ${H + top + 20}`} role="img" aria-label={`Work created and finished in each of the last ${weeks.length} weeks. Latest week: ${weeks.at(-1)?.created ?? 0} created, ${weeks.at(-1)?.resolved ?? 0} finished. The same numbers are in the table below.`} onMouseLeave={() => setAt(null)}>
        <line x1={0} x2={weeks.length * W} y1={top} y2={top} className="grid" />
        <line x1={0} x2={weeks.length * W} y1={top + H / 2} y2={top + H / 2} className="grid" />
        <text x={2} y={top + 10} className="tick">{max}</text>
        {weeks.map((w, i) => (
          <g key={w.week} transform={`translate(${i * W}, 0)`} className={at === i ? "on" : ""} onMouseEnter={() => setAt(i)} onFocus={() => setAt(i)} onBlur={() => setAt(null)} tabIndex={0} role="img" aria-label={`Week of ${day(w.week)}: ${w.created} created, ${w.resolved} finished`}>
            <rect className="hit" x={0} y={0} width={W} height={H + top + 20} />
            <path className="created" d={column(5, 9, (w.created / max) * H, top + H)} />
            <path className="resolved" d={column(16, 9, (w.resolved / max) * H, top + H)} />
            {(i % 2 === 1 || weeks.length <= 6) && (
              <text x={W / 2} y={top + H + 14} textAnchor="middle" className="tick">
                {tick(w.week)}
              </text>
            )}
          </g>
        ))}
        <line x1={0} x2={weeks.length * W} y1={top + H} y2={top + H} className="axis" />
      </svg>
      <details className="small">
        <summary>The numbers as a table</summary>
        <table className="table">
          <thead>
            <tr>
              <th scope="col">Week of</th>
              <th scope="col" className="num">Created</th>
              <th scope="col" className="num">Finished</th>
            </tr>
          </thead>
          <tbody>
            {weeks.map((w) => (
              <tr key={w.week}>
                <th scope="row" style={{ fontWeight: 400 }}>{day(w.week)}</th>
                <td className="num">{w.created}</td>
                <td className="num">{w.resolved}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}

export default function Report({ path }: { path: string }) {
  const q = useGet<any>(path, { staleTime: 30_000 });
  return (
    <Loaded q={q} what="Counting the work" lines={6}>
      {(r) => {
        const t = r.totals;
        if (t.total === 0 && (r.scope.not_included ?? 0) === 0)
          return (
            <Empty icon={<ChartColumn />} title="Nothing to report yet">
              <p>Figures appear here once there is work in {r.scope.kind === "project" ? "this project" : "the projects beneath this"}.</p>
            </Empty>
          );
        const pct = t.total ? Math.round((t.done / t.total) * 100) : 0;
        const now = today();
        return (
          <div className="stack loose report">
            {r.scope.not_included > 0 && (
              <div className="notice">
                {r.scope.not_included} of the projects you can open {r.scope.not_included === 1 ? "is" : "are"} not counted here because you cannot read {r.scope.not_included === 1 ? "its" : "their"} reports. The figures cover {r.scope.projects} {r.scope.projects === 1 ? "project" : "projects"}.
              </div>
            )}
            <div className="stats">
              <Tile label="Work items" value={t.total} />
              <Tile label="Finished" value={`${pct}%`} hint={`${t.done} of ${t.total}`} tone="good" />
              <Tile label="In progress" value={t.in_progress} />
              <Tile label="Overdue" value={t.overdue} tone={t.overdue ? "bad" : ""} />
              <Tile label="Unassigned" value={t.unassigned} tone={t.unassigned ? "warn" : ""} />
              <Tile label="Created, 30 days" value={t.created_30d} />
              <Tile label="Finished, 30 days" value={t.resolved_30d} />
              <Tile label="Days to finish" value={t.cycle_days ?? "–"} hint="Average from creation to completion over the last 90 days" />
              <Tile label="Estimate left" value={t.estimate_open} hint={`${t.estimate_done} finished`} />
            </div>
            <div className="report-grid">
              <section className="panel">
                <header>
                  <h3>By status</h3>
                </header>
                <div className="body">
                  <Bars groups={r.by_status} caption="Work items by status" />
                </div>
              </section>
              <section className="panel">
                <header>
                  <h3>By type</h3>
                </header>
                <div className="body">
                  <Bars groups={r.by_type} caption="Work items by type" />
                </div>
              </section>
              <section className="panel">
                <header>
                  <h3>By priority</h3>
                </header>
                <div className="body">
                  <Bars groups={r.by_priority} caption="Work items by priority" />
                </div>
              </section>
              <section className="panel">
                <header>
                  <h3>Flow</h3>
                </header>
                <div className="body">
                  <Weekly weeks={r.weekly} />
                </div>
              </section>
              {r.by_project.length > 1 && (
                <section className="panel">
                  <header>
                    <h3>By project</h3>
                  </header>
                  <div className="body">
                    <Bars groups={r.by_project} caption="Work items by project" />
                  </div>
                </section>
              )}
            </div>
            <section className="panel">
              <header>
                <h3>Who is doing the work</h3>
                <span className="small muted">People and AI agents with work assigned here</span>
              </header>
              <div className="table-wrap">
                <table className="table">
                  <thead>
                    <tr>
                      <th scope="col">Account</th>
                      <th scope="col" className="num">Open</th>
                      <th scope="col" className="num">Overdue</th>
                      <th scope="col" className="num">Finished, 30 days</th>
                      <th scope="col" className="num hide-sm">Estimate left</th>
                      <th scope="col" className="num hide-sm">Changes, 30 days</th>
                    </tr>
                  </thead>
                  <tbody>
                    {r.people.map((p: any) => (
                      <tr key={p.person?.id ?? "none"}>
                        <th scope="row" style={{ fontWeight: 400 }}>
                          {p.person ? (
                            <Link to={`/people/${p.person.id}`}>
                              <PersonChip person={p.person} />
                            </Link>
                          ) : (
                            <PersonChip person={null} />
                          )}
                        </th>
                        <td className="num">{p.open}</td>
                        <td className={`num ${p.overdue ? "bad-text" : ""}`}>{p.overdue}</td>
                        <td className="num">{p.done_30d}</td>
                        <td className="num hide-sm">{p.estimate_open}</td>
                        <td className="num hide-sm">{p.actions_30d ?? 0}</td>
                      </tr>
                    ))}
                    {r.people.length === 0 && (
                      <tr>
                        <td colSpan={6} className="muted">Nothing is assigned to anyone.</td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </section>
            <div className="report-grid">
              <section className="panel">
                <header>
                  <h3>Most active, last 30 days</h3>
                </header>
                {r.contributors.length === 0 && <p className="muted" style={{ padding: 14 }}>No changes in the last 30 days.</p>}
                {r.contributors.slice(0, 8).map((c: any) => (
                  <div key={c.person?.id ?? "x"} className="list-row">
                    <Avatar person={c.person} />
                    <span className="grow truncate">
                      {c.person?.name ?? "Former account"}
                      {c.person?.kind === "agent" ? " (AI agent)" : ""}
                    </span>
                    <span className="small muted nowrap">
                      {c.actions} changes · {relative(c.last_active)}
                    </span>
                  </div>
                ))}
              </section>
              <section className="panel">
                <header>
                  <h3>Late, and due soon</h3>
                </header>
                {r.due.length === 0 && <p className="muted" style={{ padding: 14 }}>Nothing is late or due in the next two weeks.</p>}
                {r.due.slice(0, 12).map((i: any) => (
                  <Link key={i.key} to={`/i/${i.key}`} className="list-row">
                    <span className="key">{i.key}</span>
                    <span className="grow truncate">{i.title}</span>
                    <Avatar person={i.assignee} size="sm" />
                    <span className={`small nowrap ${i.due_date < now ? "bad-text" : "muted"}`}>
                      {i.due_date < now ? "Was due " : "Due "}
                      {day(i.due_date)}
                    </span>
                  </Link>
                ))}
              </section>
            </div>
          </div>
        );
      }}
    </Loaded>
  );
}
