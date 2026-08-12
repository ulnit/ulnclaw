// hermes-parity PoC (v0.3.2): mounts verbatim hermes primitives
// (Button variants + Loader) to prove the React+Tailwind pipeline works
// inside the Tauri shell against ulnclaw tokens.
import { createRoot } from "react-dom/client";
import { Button } from "./hermes/button";
import { Loader } from "./hermes/loader";
import "./hermes/tokens.css";

function Probe(): React.JSX.Element {
  return (
    <div className="flex flex-col gap-2 rounded-md bg-(--ui-bg-quaternary) p-2">
      <div className="flex flex-wrap items-center gap-1.5">
        <Button variant="default" size="xs">Primary</Button>
        <Button variant="secondary" size="xs">Secondary</Button>
        <Button variant="outline" size="xs">Outline</Button>
        <Button variant="ghost" size="xs">Ghost</Button>
        <Button variant="text" size="inline">Text</Button>
      </div>
      <div className="flex items-center gap-2 text-(--ui-text-secondary)">
        <Loader type="lemniscate-bloom" />
        <span className="text-[0.6875rem]">hermes primitives live</span>
      </div>
    </div>
  );
}

export function mountReactProbe(): void {
  if (document.getElementById("react-probe")) return;
  const footer = document.querySelector(".sidebar-footer");
  if (!footer) return;
  const host = document.createElement("div");
  host.id = "react-probe";
  footer.before(host);
  createRoot(host).render(<Probe />);
}

mountReactProbe();
