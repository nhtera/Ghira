// SPDX-License-Identifier: Apache-2.0
// Test helpers: providers and a seeded live store.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { render, type RenderResult } from "@testing-library/react";
import { ToastProvider } from "@ghi/ui";
import { initialLive, useLive, type LiveState } from "../../state/live";

const TOAST_LABEL = "notifications";

export function renderLive(ui: ReactElement): RenderResult {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ToastProvider label={TOAST_LABEL}>{ui}</ToastProvider>
    </QueryClientProvider>,
  );
}

export const setLive = (patch: Partial<LiveState>) => useLive.setState({ ...initialLive, ...patch });
