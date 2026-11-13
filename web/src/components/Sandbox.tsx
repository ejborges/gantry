// A visualization or a piece of custom content, running in the sandboxed frame.
// See lib/sandbox.ts for what the frame can and cannot do.
import { useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";
import { mountSandbox, type SandboxContent, type SandboxHandle } from "../lib/sandbox";
import { SLOW_AFTER_MS } from "../ui";

interface Props {
  content: SandboxContent;
  /** Change this to make the content fetch its data again without starting over. */
  refreshKey?: number;
  /** Hide the small "refresh" control (the surrounding page has its own). */
  bare?: boolean;
  onError?: (message: string | null) => void;
}

export default function Sandbox({ content, refreshKey, bare, onError }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const handle = useRef<SandboxHandle | null>(null);
  const [state, setState] = useState<"drawing" | "slow" | "shown" | "failed">("drawing");
  const serial = JSON.stringify([content.kind, content.source, content.library, content.params, content.mode]);
  const cb = useRef(onError);
  cb.current = onError;

  useEffect(() => {
    if (!host.current) return;
    const h = mountSandbox(host.current, content, {
      rendered: () => (setState("shown"), cb.current?.(null)),
      error: (m) => (setState("failed"), cb.current?.(m)),
    });
    handle.current = h;
    return () => {
      h.destroy();
      handle.current = null;
    };
    // The frame is created once; later changes go through update().
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    setState("drawing");
    handle.current?.update(content);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serial]);

  useEffect(() => {
    if (refreshKey) handle.current?.refresh();
  }, [refreshKey]);

  // The same promise as everywhere else: a skeleton first, and words once it takes long.
  useEffect(() => {
    if (state !== "drawing") return;
    const t = setTimeout(() => setState((s) => (s === "drawing" ? "slow" : s)), SLOW_AFTER_MS);
    return () => clearTimeout(t);
  }, [state, serial]);

  return (
    <div className={`sandbox ${content.mode === "flow" ? "flow" : "tile"} ${state}`}>
      <div ref={host} className="sandbox-host" />
      {(state === "drawing" || state === "slow") && (
        <div className="sandbox-wait" role="status">
          <span className="skeleton" style={{ width: "62%" }} />
          <span className="skeleton" style={{ width: "86%" }} />
          <span className="skeleton" style={{ width: "44%" }} />
          {state === "slow" ? <span className="small muted">Still fetching the data for {content.title || "this"}…</span> : <span className="sr-only">Drawing {content.title || "the content"}</span>}
        </div>
      )}
      {!bare && state !== "drawing" && (
        <button type="button" className="btn ghost icon sm sandbox-refresh" aria-label={`Fetch the data for ${content.title || "this"} again`} title="Fetch the data again" onClick={() => (setState("drawing"), handle.current?.refresh())}>
          <RefreshCw />
        </button>
      )}
    </div>
  );
}
