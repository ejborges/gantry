// Who is signed in, what they may do, and the color theme.
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Navigate, useLocation } from "react-router-dom";
import { api, setCsrf, useGet, type Me } from "./api";
import { Loading } from "./ui";
import { setTimeZone } from "./lib/format";
import { live } from "./realtime";

export type Theme = "system" | "light" | "dark";

interface Session {
  me: Me;
  can: (permission: string) => boolean;
  theme: Theme;
  setTheme: (t: Theme) => void;
}
const Ctx = createContext<Session | null>(null);

export function useSession(): Session {
  const s = useContext(Ctx);
  if (!s) throw new Error("useSession outside SessionProvider");
  return s;
}

export function applyTheme(t: Theme) {
  const dark = t === "dark" || (t === "system" && window.matchMedia?.("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  try {
    localStorage.setItem("gantry.theme", t);
  } catch {
    /* private browsing: the theme simply is not remembered on this device */
  }
}

export function SessionProvider({ children }: { children: ReactNode }) {
  const loc = useLocation();
  const q = useGet<Me>("/api/v1/auth/me", { retry: false, staleTime: 60_000 });
  const [theme, setThemeState] = useState<Theme>(() => (localStorage.getItem("gantry.theme") as Theme) || "system");

  useEffect(() => {
    if (q.data) {
      setCsrf(q.data.csrf);
      setTimeZone(q.data.user.timezone);
      // The account's saved theme wins over this device's on first load.
      if (!localStorage.getItem("gantry.theme")) setThemeState(q.data.user.theme);
    }
  }, [q.data]);
  useEffect(() => {
    applyTheme(theme);
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const on = () => applyTheme("system");
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, [theme]);
  useEffect(() => {
    const out = () => {
      live.stop();
      void q.refetch();
    };
    window.addEventListener("gantry:signed-out", out);
    return () => window.removeEventListener("gantry:signed-out", out);
  }, [q]);
  // Notifications addressed to this account arrive as they happen.
  const id = q.data?.user.id;
  useEffect(() => (id ? live.join(`user:${id}`) : undefined), [id]);

  const value = useMemo<Session | null>(() => {
    if (!q.data) return null;
    const perms = new Set(q.data.permissions);
    return {
      me: q.data,
      can: (p) => perms.has(p),
      theme,
      setTheme: (t) => {
        setThemeState(t);
        void api("PATCH", "/api/v1/auth/me", { theme: t }).catch(() => undefined);
      },
    };
  }, [q.data, theme]);

  if (q.isPending) return <div className="page narrow"><Loading what="Checking your sign-in" /></div>;
  // Read during render so the first paint already shows times in the account's zone.
  if (q.data) setTimeZone(q.data.user.timezone);
  if (!value) return <Navigate to={`/login?next=${encodeURIComponent(loc.pathname + loc.search)}`} replace />;
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
