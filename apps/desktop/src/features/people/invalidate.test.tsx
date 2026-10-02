// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { useInvalidatePeople } from "./queries";

vi.mock("../../ipc", () => ({ ipc: { commands: {} } }));

describe("useInvalidatePeople", () => {
  it("reloads everything that shows a person's name", () => {
    const client = new QueryClient();
    const spy = vi.spyOn(client, "invalidateQueries");
    const { result } = renderHook(() => useInvalidatePeople(), { wrapper: ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider> });
    result.current();
    const keys = spy.mock.calls.map((c) => (c[0] as { queryKey: readonly string[] }).queryKey[0]);
    expect(keys).toEqual(expect.arrayContaining(["people", "voice", "meeting", "meetings", "knownSpeakerNames", "search", "related"]));
  });
});
