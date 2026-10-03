// SPDX-License-Identifier: Apache-2.0
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@ghi/ui/styles.css";
import "./styles.css";
import { applyTheme } from "@ghi/ui";
import { App } from "./app";

// Before the first paint, so a dark phone never flashes light.
applyTheme("system");

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
