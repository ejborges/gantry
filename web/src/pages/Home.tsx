import { Link } from "react-router-dom";
import { FolderKanban, PartyPopper } from "lucide-react";
import { useGet, type Item } from "../api";
import { useLiveRefresh } from "../realtime";
import { useSession } from "../session";
import { Empty, Loaded } from "../ui";
import { ItemTable } from "../work";
import { isOverdue } from "../lib/format";

interface List {
  items: Item[];
  total: number;
}

export default function Home() {
  const { me } = useSession();
  // What is assigned to you changes as others work: the lists follow without a reload.
  useLiveRefresh(["work"]);
  const mine = useGet<List>("/api/v1/items?assignee=me&category=todo,in_progress&sort=priority&limit=50");
  const reported = useGet<List>("/api/v1/items?reporter=me&category=todo,in_progress&sort=updated&limit=1");
  const recent = useGet<List>("/api/v1/items?sort=updated&limit=8");
  const projects = useGet<any[]>("/api/v1/projects");
  const overdue = mine.data?.items.filter((i) => isOverdue(i.due_date, false)).length ?? 0;
  const urgent = mine.data?.items.filter((i) => i.priority >= 3).length ?? 0;
  const first = me.user.name.split(" ")[0];

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>My work</h1>
          <p className="sub">Signed in as {first}. Everything assigned to you, across all projects.</p>
        </div>
      </div>
      <div className="stack loose">
        <div className="stat-row">
          <div className="stat">
            <div className="n">{mine.data?.total ?? "–"}</div>
            <div className="l">Open and assigned to you</div>
          </div>
          <div className={`stat ${overdue ? "alert" : ""}`}>
            <div className="n">{mine.data ? overdue : "–"}</div>
            <div className="l">Past their due date</div>
          </div>
          <div className={`stat ${urgent ? "warn" : ""}`}>
            <div className="n">{mine.data ? urgent : "–"}</div>
            <div className="l">High or critical priority</div>
          </div>
          <div className="stat">
            <div className="n">{reported.data?.total ?? "–"}</div>
            <div className="l">Open items you raised</div>
          </div>
        </div>
        <div className="two-col">
          <section className="panel">
            <header>
              <h2>Assigned to you</h2>
            </header>
            <Loaded q={mine}>
              {(d) => (
                <ItemTable
                  items={d.items}
                  showProject
                  hideAssignee
                  empty={
                    <Empty icon={<PartyPopper />} title="Nothing is waiting on you">
                      <p>When someone assigns you a work item it appears here, most urgent first.</p>
                    </Empty>
                  }
                />
              )}
            </Loaded>
          </section>
          <div className="stack loose">
            <section className="panel">
              <header>
                <h2>Projects</h2>
                <Link to="/projects" className="small">
                  All projects
                </Link>
              </header>
              <Loaded q={projects} lines={3}>
                {(list) =>
                  list.length === 0 ? (
                    <Empty icon={<FolderKanban />} title="No projects yet">
                      <Link className="btn primary sm" to="/projects?new=1">
                        Create a project
                      </Link>
                    </Empty>
                  ) : (
                    <div>
                      {list.slice(0, 7).map((p) => (
                        <Link key={p.key} to={`/projects/${p.key}`} className="list-row">
                          <span className="key">{p.key}</span>
                          <span className="grow truncate">{p.name}</span>
                          <span className="muted small nowrap">{p.open_items} open</span>
                        </Link>
                      ))}
                    </div>
                  )
                }
              </Loaded>
            </section>
            <section className="panel">
              <header>
                <h2>Recently changed</h2>
              </header>
              <Loaded q={recent} lines={3}>
                {(d) =>
                  d.items.length === 0 ? (
                    <Empty title="No activity yet" />
                  ) : (
                    <div>
                      {d.items.map((i) => (
                        <Link key={i.id} to={`/i/${i.key}`} className="list-row">
                          <span className={`status-dot ${i.status_category}`} />
                          <span className="key">{i.key}</span>
                          <span className="grow truncate">{i.title}</span>
                        </Link>
                      ))}
                    </div>
                  )
                }
              </Loaded>
            </section>
          </div>
        </div>
      </div>
    </div>
  );
}
