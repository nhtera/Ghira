// SPDX-License-Identifier: Apache-2.0
// Providers: i18n (the phone's language), platform, and the router.
import { RouterProvider } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { I18nextProvider } from "react-i18next";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { PlatformProvider, setTextScale } from "@ghi/ui";
import { AppErrorBoundary } from "./features/store-problem";
import { makeRouter } from "./router";
import { browserOverrides, deviceLanguage } from "./state/language";

export function App() {
  const [overrides] = useState(browserOverrides);
  const [language] = useState(() => overrides.lang ?? deviceLanguage());
  const [i18n] = useState(() => initMobileI18n(language));
  const [router] = useState(() => makeRouter());
  useEffect(() => {
    document.documentElement.lang = language;
    document.documentElement.dataset.platform = "ios";
    if (overrides.scale) setTextScale(overrides.scale);
  }, [language, overrides]);
  // The dev browser has a desktop user agent; the phone is always "ios".
  return (
    <I18nextProvider i18n={i18n}>
      <PlatformProvider value="ios">
        <AppErrorBoundary>
          <RouterProvider router={router} />
        </AppErrorBoundary>
      </PlatformProvider>
    </I18nextProvider>
  );
}
