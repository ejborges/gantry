import { StrictMode, lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { QueryClientProvider } from "@tanstack/react-query";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "./styles.css";
import "./styles-2.css";
import "./styles-3.css";
import { queryClient } from "./api";
import { SessionProvider, useSession } from "./session";
import { Loading, PageTitle, Toaster } from "./ui";
import Shell from "./shell";
import Login from "./pages/Login";
import Home from "./pages/Home";
import Projects from "./pages/Projects";
import Project from "./pages/Project";
import ItemPage from "./pages/Item";
import Portal from "./pages/Portal";
import SearchPage from "./pages/Search";
import Profile from "./pages/Profile";
import { ErrorBoundary, NotFound, StatusList, StatusRoute } from "./pages/Status";

// Editors and the administration console are large and not needed on first paint.
const Docs = lazy(() => import("./pages/Docs"));
const PeopleRoutes = lazy(() => import("./pages/People").then((m) => ({ default: m.PeopleRoutes })));
const TeamRoutes = lazy(() => import("./pages/People").then((m) => ({ default: m.TeamRoutes })));
const Planning = lazy(() => import("./pages/Planning"));
const Analytics = lazy(() => import("./pages/Analytics"));
const About = lazy(() => import("./pages/About"));
const Legal = lazy(() => import("./pages/About").then((m) => ({ default: m.LegalRoutes })));
const Admin = lazy(() => import("./pages/Admin"));

function Landing() {
  // Portal-only accounts have no workspace; send them to the help desk.
  const { can } = useSession();
  return can("workspace.access") ? <Home /> : <Navigate to="/portal" replace />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <ErrorBoundary>
        <Suspense fallback={<div className="page narrow"><Loading /></div>}>
          <Routes>
            <Route path="/login" element={<Login />} />
            {/* Readable without signing in: the statements linked from the sign-in page, and what an error means. */}
            <Route path="/legal/*" element={<Legal />} />
            <Route path="/status/:code" element={<div className="public-page"><StatusRoute /></div>} />
            <Route
              element={
                <SessionProvider>
                  <Shell />
                </SessionProvider>
              }
            >
              <Route index element={<Landing />} />
              <Route path="projects" element={<Projects />} />
              <Route path="projects/:key" element={<Project />} />
              <Route path="i/:key" element={<ItemPage />} />
              <Route path="docs/*" element={<Docs />} />
              <Route path="portal/*" element={<Portal />} />
              <Route path="search" element={<SearchPage />} />
              <Route path="profile" element={<Profile />} />
              <Route path="people/*" element={<PeopleRoutes />} />
              <Route path="teams/*" element={<TeamRoutes />} />
              <Route path="planning/*" element={<Planning />} />
              <Route path="analytics/*" element={<Analytics />} />
              <Route path="about/*" element={<About />} />
              <Route path="status" element={<StatusList />} />
              <Route path="admin/*" element={<Admin />} />
              <Route path="*" element={<NotFound />} />
            </Route>
          </Routes>
        </Suspense>
        </ErrorBoundary>
        <Toaster />
        <PageTitle />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
);
