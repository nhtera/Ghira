// SPDX-License-Identifier: Apache-2.0
// The popover, mini recorder and detection panel are opaque native windows
// with their own rounded corners: the page fills the window with the card
// background, with no outer margin or shadow. Marks <html> with
// data-window="panel" while mounted (for styles that must differ in a panel).
import { useEffect } from "react";

export function usePanelWindow() {
  useEffect(() => {
    const html = document.documentElement;
    html.dataset.window = "panel";
    return () => {
      delete html.dataset.window;
    };
  }, []);
}
