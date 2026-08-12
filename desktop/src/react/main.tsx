import { createRoot } from "react-dom/client";
import { applyStatic } from "../i18n";
import { App } from "./App";
import "../hermes/tokens.css";
import "./shell.css";

document.getElementById("app")?.remove();
const host = document.createElement("div");
host.id = "react-root";
document.body.appendChild(host);
applyStatic();
createRoot(host).render(<App />);
