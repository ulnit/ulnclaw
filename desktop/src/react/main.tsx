import { createRoot } from "react-dom/client";
import { App } from "./App";
import "../hermes/tokens.css";
import "./shell.css";

document.getElementById("app")?.remove();
const host = document.createElement("div");
host.id = "react-root";
document.body.appendChild(host);
createRoot(host).render(<App />);
