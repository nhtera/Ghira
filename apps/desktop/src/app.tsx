// SPDX-License-Identifier: Apache-2.0
// Providers: i18n (language from prefs), theme, platform, query cache,
// tooltips, toasts, and the router.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { I18nextProvider, useTranslation } from "react-i18next";
import { initI18n } from "@ghi/i18n";
import { PlatformProvider, ToastProvider, TooltipProvider, detectPlatform, useTheme, type Platform } from "@ghi/ui";
import { makeRouter } from "./router";
import { usePrefs } from "./state/prefs";

const queryClient = new QueryClient({ defaultOptions: { queries: { staleTime: 30_000, retry: false } } });

/** `?platform=win|mac` overrides detection (dev and tests only). */
function platformOverride(): Platform {
  const p = new URLSearchParams(window.location.search).get("platform");
  return p === "win" || p === "mac" ? p : detectPlatform();
}

function Shell({ router }: { router: ReturnType<typeof makeRouter> }) {
  const { t } = useTranslation();
  return (
    <ToastProvider label={t("shell.notifications")}>
      <RouterProvider router={router} />
    </ToastProvider>
  );
}

export function App() {
  const language = usePrefs((s) => s.language);
  const theme = usePrefs((s) => s.theme);
  useTheme(theme);
  const [i18n] = useState(() => initI18n(language));
  const [router] = useState(() => makeRouter());
  const [platform] = useState(platformOverride);
  useEffect(() => {
    initI18n(language);
    document.documentElement.lang = language;
  }, [language]);
  useEffect(() => {
    document.documentElement.dataset.platform = platform;
  }, [platform]);
  return (
    <I18nextProvider i18n={i18n}>
      <PlatformProvider value={platform}>
        <QueryClientProvider client={queryClient}>
          <TooltipProvider>
            <Shell router={router} />
          </TooltipProvider>
        </QueryClientProvider>
      </PlatformProvider>
    </I18nextProvider>
  );
}
