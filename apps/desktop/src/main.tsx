// SPDX-License-Identifier: Apache-2.0
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@ghi/ui/styles.css";
import { applyTheme } from "@ghi/ui";
import { App } from "./app";
import { usePrefs } from "./state/prefs";

// Before the first paint, so a dark window never flashes light.
applyTheme(usePrefs.getState().theme);

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
