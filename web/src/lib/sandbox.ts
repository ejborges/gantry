// The host side of the sandbox that custom HTML and visualization scripts run in.
//
// The frame has an opaque origin and a policy that forbids every network connection, so a
// script inside it can reach nothing by itself. What it may do is ask this page for data.
// This page answers with a GET under /api/v1/, made as the person looking at the page, and
// nothing else: not other addresses, not other methods, and never credentials or tokens.
import { ApiError, sandboxGet } from "../api";
import { timeZone } from "./format";

export const LIBRARIES: Record<string, { name: string; scripts: string[]; global: string }> = {
  chartjs: { name: "Chart.js", scripts: ["/vendor/chartjs/chart.umd.min.js"], global: "Chart" },
  echarts: { name: "Apache ECharts", scripts: ["/vendor/echarts/echarts.min.js"], global: "echarts" },
  plotly: { name: "Plotly", scripts: ["/vendor/plotly/plotly.min.js"], global: "Plotly" },
  d3: { name: "D3", scripts: ["/vendor/d3/d3.min.js"], global: "d3" },
  html: { name: "HTML and script", scripts: [], global: "" },
};

export interface SandboxContent {
  /** `script`: the body of an async function. `html`: markup, whose <script> elements run in order. */
  kind: "script" | "html";
  source: string;
  library?: string;
  params?: Record<string, unknown>;
  /** `tile` fills its container; `flow` takes the height its content needs. */
  mode?: "tile" | "flow";
  title?: string;
}

export interface SandboxHandle {
  update(content: SandboxContent): void;
  refresh(): void;
  destroy(): void;
}

/** The application's colors as the charts should use them, read from the current theme. */
export function themeColors(): Record<string, unknown> {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  const dark = document.documentElement.dataset.theme === "dark";
  return {
    text: v("--ink"), muted: v("--muted"), grid: v("--viz-grid") || v("--line"), axis: v("--line-strong"), background: v("--surface"), accent: v("--action"), accentSoft: v("--action-soft"), danger: v("--danger"),
    todo: v("--todo"), progress: v("--progress"), done: v("--done"),
    // Eight series colors in a fixed order. Use them in this order and never cycle past the
    // eighth: fold further series into "Other".
    series: [v("--viz-1"), v("--viz-2"), v("--viz-3"), v("--viz-4"), v("--viz-5"), v("--viz-6"), v("--viz-7"), v("--viz-8")],
    // The ink for a label drawn inside a fill of the series color with the same index.
    seriesInk: ["#ffffff", "#ffffff", "#ffffff", dark ? "#ffffff" : "#0b0b0b", "#ffffff", "#ffffff", "#ffffff", "#ffffff"],
    priority: [v("--faint"), v("--todo"), v("--progress"), v("--warn-solid"), v("--danger")],
  };
}

function theme(): "light" | "dark" {
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

export function mountSandbox(host: HTMLElement, initial: SandboxContent, on?: { rendered?: () => void; error?: (message: string) => void }): SandboxHandle {
  let content = initial;
  let ready = false;
  const frame = document.createElement("iframe");
  // No allow-same-origin: the frame cannot read this page, its cookies or its storage.
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("referrerpolicy", "no-referrer");
  frame.title = content.title || "Custom content";
  frame.className = `sandbox-frame ${content.mode === "flow" ? "flow" : "tile"}`;
  frame.src = "/api/v1/sandbox/frame";
  // The frame's origin is opaque, so there is no origin to name; the message carries no secrets.
  const post = (msg: Record<string, unknown>) => frame.contentWindow?.postMessage(msg, "*");
  const run = () =>
    post({
      type: "run", kind: content.kind, source: content.source, scripts: LIBRARIES[content.library ?? "html"]?.scripts ?? [], params: content.params ?? {},
      theme: theme(), colors: themeColors(), mode: content.mode ?? "tile", locale: navigator.language, timeZone: timeZone(),
    });
  const onMessage = async (ev: MessageEvent) => {
    if (ev.source !== frame.contentWindow) return;
    const m = ev.data;
    if (!m || typeof m !== "object" || m.source !== "gantry-sandbox") return;
    switch (m.type) {
      case "ready":
        ready = true;
        run();
        break;
      case "api":
        try {
          const data = await sandboxGet(String(m.path), m.query && typeof m.query === "object" ? m.query : null);
          post({ type: "api-result", id: m.id, ok: true, data });
        } catch (e) {
          post({ type: "api-result", id: m.id, ok: false, error: e instanceof ApiError ? e.message : "The request failed." });
        }
        break;
      case "height":
        if (content.mode === "flow" && typeof m.height === "number") frame.style.height = `${Math.min(4000, Math.max(40, Math.ceil(m.height)))}px`;
        break;
      case "rendered":
        on?.rendered?.();
        break;
      case "error":
        on?.error?.(String(m.message ?? "The content reported an error."));
        break;
      default:
        break;
    }
  };
  window.addEventListener("message", onMessage);
  const observer = new MutationObserver(() => ready && post({ type: "theme", theme: theme(), colors: themeColors() }));
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  host.appendChild(frame);
  return {
    update(next) {
      content = next;
      frame.title = next.title || "Custom content";
      if (ready) run();
    },
    refresh() {
      if (ready) post({ type: "refresh" });
    },
    destroy() {
      window.removeEventListener("message", onMessage);
      observer.disconnect();
      frame.remove();
    },
  };
}
