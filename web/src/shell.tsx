// Application frame: navigation rail, top bar, search palette, notifications, mobile tab bar.
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, NavLink, Outlet, useLocation, useNavigate } from "react-router-dom";
import { ErrorBoundary } from "./pages/Status";
import { Bell, BookOpen, ChartColumn, FileText, FolderKanban, House, Info, LifeBuoy, LogOut, Menu as MenuIcon, Milestone, Monitor, Moon, Search, Settings, Shield, Sun, User, Users, CircleHelp, Ellipsis, SquareCheck, WifiOff } from "lucide-react";
import { api, queryClient, refresh, useGet } from "./api";
import { useSession, type Theme } from "./session";
import { Avatar, Menu, PRESENCE, Spinner, setSiteName, toast } from "./ui";
import { relative } from "./lib/format";
import { live, useLiveStatus } from "./realtime";

export function Logo() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M4 21V3h16v18M4 3l16 9M20 3 4 12M4 12h16" />
    </svg>
  );
}

function useNav() {
  const { can } = useSession();
  const member = can("workspace.access");
  return [
    member && { to: "/", label: "My work", icon: House, end: true },
    member && { to: "/projects", label: "Projects", icon: FolderKanban },
    member && { to: "/planning", label: "Planning", icon: Milestone },
    member && { to: "/docs", label: "Docs", icon: BookOpen },
    member && { to: "/analytics", label: "Analytics", icon: ChartColumn },
    { to: "/portal", label: "Help desk", icon: LifeBuoy },
    member && { to: "/people", label: "People", icon: Users },
    member && { to: "/search", label: "Search", icon: Search },
    can("admin.access") && { to: "/admin", label: "Administration", icon: Shield },
  ].filter(Boolean) as { to: string; label: string; icon: typeof House; end?: boolean }[];
}

interface Hit {
  kind: string;
  label: string;
  title: string;
  url: string;
}

function Palette({ onClose }: { onClose: () => void }) {
  const nav = useNav();
  const navigate = useNavigate();
  const [q, setQ] = useState("");
  const [debounced, setDebounced] = useState("");
  const [sel, setSel] = useState(0);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(q.trim()), 180);
    return () => clearTimeout(t);
  }, [q]);
  const results = useGet<{ results: Hit[] }>(debounced.length >= 2 ? `/api/v1/search?q=${encodeURIComponent(debounced)}&limit=8` : null);
  const entries = useMemo(() => {
    const pages = nav.filter((n) => !q || n.label.toLowerCase().includes(q.toLowerCase())).map((n) => ({ key: n.to, label: n.label, hint: "Go to", to: n.to, Icon: n.icon }));
    const hits = (results.data?.results ?? []).map((h) => ({ key: h.url, label: h.title, hint: h.label, to: h.url, Icon: h.kind === "page" ? FileText : SquareCheck }));
    return [...hits, ...pages];
  }, [nav, q, results.data]);
  useEffect(() => setSel(0), [entries.length]);
  const go = (to: string) => {
    onClose();
    navigate(to);
  };
  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette" role="dialog" aria-modal="true" aria-label="Search and go to">
        <div className="palette-input">
          {results.isFetching ? <Spinner /> : <Search />}
          <input
            autoFocus
            placeholder="Search work items and pages, or jump to a section"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              if (e.key === "ArrowDown") (e.preventDefault(), setSel((s) => Math.min(s + 1, entries.length - 1)));
              if (e.key === "ArrowUp") (e.preventDefault(), setSel((s) => Math.max(s - 1, 0)));
              if (e.key === "Enter") {
                if (entries[sel]) go(entries[sel].to);
                else if (q.trim()) go(`/search?q=${encodeURIComponent(q.trim())}`);
              }
            }}
            aria-label="Search"
          />
          <span className="kbd">Esc</span>
        </div>
        <div className="palette-list" role="listbox">
          {entries.map((e, i) => (
            <a key={e.key} href={e.to} role="option" aria-selected={i === sel} className="palette-item" onMouseEnter={() => setSel(i)} onClick={(ev) => (ev.preventDefault(), go(e.to))}>
              <e.Icon />
              <span className="grow truncate">{e.label}</span>
              <span className="muted small nowrap">{e.hint}</span>
            </a>
          ))}
          {entries.length === 0 && <div className="empty"><p>Nothing matches. Press Enter to open the full search.</p></div>}
        </div>
      </div>
    </div>
  );
}

function Notifications() {
  const { me } = useSession();
  const list = useGet<any[]>("/api/v1/notifications", { staleTime: 30_000, refetchInterval: 120_000 });
  const unread = list.data ? list.data.filter((n) => !n.read_at).length : me.unread_notifications;
  // A notification that arrives while the application is open is announced as it arrives.
  useEffect(() => {
    const on = (e: Event) => {
      const n = (e as CustomEvent).detail;
      if (n?.title) toast(n.title);
    };
    window.addEventListener("gantry:notify", on);
    return () => window.removeEventListener("gantry:notify", on);
  }, []);
  return (
    <Menu
      trigger={(_, toggle) => (
        <button
          className="btn ghost icon dot-badge"
          data-count={unread > 0 ? (unread > 99 ? "99+" : unread) : undefined}
          aria-label={unread ? `Notifications, ${unread} unread` : "Notifications"}
          onClick={() => {
            toggle();
            if (unread) void api("POST", "/api/v1/notifications/read").then(() => queryClient.invalidateQueries({ queryKey: ["/api/v1/notifications"] }));
          }}
        >
          <Bell />
        </button>
      )}
    >
      {(close) => (
        <div style={{ width: 320, maxHeight: 420, overflowY: "auto" }}>
          <div className="menu-label">Notifications</div>
          {(list.data ?? []).slice(0, 30).map((n) => (
            <Link key={n.id} to={n.link || "/"} onClick={close} style={{ alignItems: "flex-start" }}>
              <span className="stack tight grow">
                <span style={{ fontWeight: n.read_at ? 400 : 600 }}>{n.title}</span>
                <span className="small muted">{relative(n.created_at)}</span>
              </span>
            </Link>
          ))}
          {list.data?.length === 0 && <div className="menu-label">You are all caught up.</div>}
          <hr />
          <Link to="/profile#notifications" onClick={close}>
            <Settings /> Choose what you are told about
          </Link>
        </div>
      )}
    </Menu>
  );
}

const THEMES: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "system", label: "Match device", icon: Monitor },
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
];

function UserMenu() {
  const { me, theme, setTheme } = useSession();
  async function signOut() {
    await api("POST", "/api/v1/auth/logout").catch(() => undefined);
    live.stop();
    queryClient.clear();
    window.location.assign("/login");
  }
  return (
    <Menu
      trigger={(_, toggle) => (
        <button className="btn ghost icon" onClick={toggle} aria-label={`Account menu for ${me.user.name}`}>
          <Avatar person={me.user} />
        </button>
      )}
    >
      {(close) => (
        <>
          <div className="menu-label">
            <strong style={{ color: "var(--ink)" }}>{me.user.name}</strong>
            <br />
            {me.user.email}
          </div>
          <hr />
          <Link to="/profile" onClick={close}>
            <User /> Profile and API tokens
          </Link>
          <div className="menu-label">Status</div>
          {Object.entries(PRESENCE).map(([k, label]) => (
            <button key={k} role="menuitemradio" aria-checked={(me.user.presence ?? "available") === k} onClick={() => void api("PATCH", "/api/v1/auth/me", { presence: k }).then(() => refresh()).catch(() => undefined)}>
              <span className={`presence-dot inline ${k}`} aria-hidden="true" /> {label}
              {(me.user.presence ?? "available") === k && <span className="badge blue" style={{ marginLeft: "auto" }}>On</span>}
            </button>
          ))}
          <div className="menu-label">Theme</div>
          {THEMES.map((t) => (
            <button key={t.value} onClick={() => setTheme(t.value)} role="menuitemradio" aria-checked={theme === t.value}>
              <t.icon /> {t.label}
              {theme === t.value && <span className="badge blue" style={{ marginLeft: "auto" }}>On</span>}
            </button>
          ))}
          <hr />
          <a href="/manual/" target="_blank" rel="noreferrer">
            <CircleHelp /> Manual
          </a>
          <Link to="/about" onClick={close}>
            <Info /> About, privacy and terms
          </Link>
          <button onClick={signOut}>
            <LogOut /> Sign out
          </button>
        </>
      )}
    </Menu>
  );
}

export default function Shell() {
  const nav = useNav();
  const loc = useLocation();
  const { me } = useSession();
  const [palette, setPalette] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const site = { data: { site: me.site } };
  const mainRef = useRef<HTMLElement>(null);
  const connection = useLiveStatus();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => {
    setDrawer(false);
    window.scrollTo(0, 0);
  }, [loc.pathname]);
  useEffect(() => setSiteName(me.site?.name), [me.site?.name]);

  const tabs = nav.slice(0, 4);
  return (
    <div className="app">
      <a href="#main" className="skip-link">Skip to content</a>
      <aside className={`rail ${drawer ? "open" : ""}`} aria-label="Main navigation">
        <Link to="/" className="brand" title={site.data?.site?.name || "Gantry"}>
          <Logo />
          <span>{site.data?.site?.name || "Gantry"}</span>
        </Link>
        <nav className="nav">
          {nav.map((n) => (
            <NavLink key={n.to} to={n.to} end={n.end} title={n.label}>
              <n.icon />
              <span>{n.label}</span>
            </NavLink>
          ))}
        </nav>
        <div className="rail-foot nav">
          <a href="/manual/" target="_blank" rel="noreferrer" title="Manual">
            <CircleHelp />
            <span>Manual</span>
          </a>
          <NavLink to="/about" title="About, privacy and terms">
            <Info />
            <span>About</span>
          </NavLink>
          <NavLink to="/profile" title="Profile">
            <Settings />
            <span>Profile</span>
          </NavLink>
        </div>
      </aside>
      {drawer && <div className="scrim" onClick={() => setDrawer(false)} />}
      <div className="main">
        <header className="topbar">
          <button className="btn ghost icon only-sm" onClick={() => setDrawer(true)} aria-label="Open navigation">
            <MenuIcon />
          </button>
          <Link to="/" className="only-sm" style={{ color: "var(--ink)", fontWeight: 600 }}>
            {site.data?.site?.name || "Gantry"}
          </Link>
          {me.permissions.includes("workspace.access") && (
            <button className="search-trigger" onClick={() => setPalette(true)} aria-keyshortcuts="Control+K">
              <Search />
              <span>Search or jump to…</span>
              <span className="kbd">Ctrl K</span>
            </button>
          )}
          <span className="grow hide-sm" />
          {connection === "offline" && (
            <span className="badge outline" role="status" title="Live updates are paused. Pages still work; reload to see the latest.">
              <WifiOff /> <span className="hide-sm">Live updates paused</span>
            </span>
          )}
          <Notifications />
          <UserMenu />
        </header>
        {site.data?.site?.announcement && <div className="banner" role="status">{site.data.site.announcement}</div>}
        <main id="main" ref={mainRef} tabIndex={-1}>
          <ErrorBoundary resetKey={loc.pathname}>
            <Outlet />
          </ErrorBoundary>
        </main>
      </div>
      <nav className="tabbar" aria-label="Sections">
        {tabs.map((n) => (
          <NavLink key={n.to} to={n.to} end={n.end}>
            <n.icon />
            {n.label}
          </NavLink>
        ))}
        <button onClick={() => setDrawer(true)}>
          <Ellipsis />
          More
        </button>
      </nav>
      {palette && <Palette onClose={() => setPalette(false)} />}
    </div>
  );
}
