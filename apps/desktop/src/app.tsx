// SPDX-License-Identifier: Apache-2.0
// Placeholder shell. The real UI (tokens, router, screens) arrives in phase 9.
import { useEffect, useState } from "react";
import { locales } from "@ghi/i18n";
import { commands, type AppVersion } from "./bindings";

const messages = locales.en;

export function App() {
  const [version, setVersion] = useState<AppVersion | null>(null);

  useEffect(() => {
    // Outside Tauri (plain browser, tests) there is no IPC; keep the placeholder.
    if (!("__TAURI_INTERNALS__" in window)) return;
    commands.appVersion().then(setVersion, () => setVersion(null));
  }, []);

  return (
    <main>
      <h1>{messages.app.name}</h1>
      <p>{messages.app.tagline}</p>
      {version && (
        <p>
          v{version.app} · core {version.core}
        </p>
      )}
    </main>
  );
}
