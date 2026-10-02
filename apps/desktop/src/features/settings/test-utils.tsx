// SPDX-License-Identifier: Apache-2.0
// Test helpers: a render wrapper with the app's providers.
import { render, type RenderResult } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { i18next } from "@ghi/i18n";
import { PlatformProvider, ToastProvider } from "@ghi/ui";

export function renderSettings(ui: ReactNode): RenderResult {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <PlatformProvider value="mac">
        <ToastProvider label={i18next.t("shell.notifications")}>{ui}</ToastProvider>
      </PlatformProvider>
    </QueryClientProvider>,
  );
}
