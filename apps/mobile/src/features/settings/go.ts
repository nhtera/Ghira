// SPDX-License-Identifier: Apache-2.0
// Navigation by path. The area route builders take an AnyRoute parent, so the
// router's typed `to` doesn't know the nested settings paths; this keeps the
// cast in one place.
import { useNavigate } from "@tanstack/react-router";
import { useCallback } from "react";

export function useGo() {
  const nav = useNavigate();
  return useCallback(
    (to: string, opts?: { replace?: boolean }) =>
      void nav({ to, replace: opts?.replace } as never),
    [nav],
  );
}
