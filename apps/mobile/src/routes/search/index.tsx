// SPDX-License-Identifier: Apache-2.0
// Search routes (owned by 16-I). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
import { createRoute, useNavigate, type AnyRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { SearchScreen } from "../../features/search/search-screen";

export function searchRoutes(parent: AnyRoute) {
  const search = createRoute({
    getParentRoute: () => parent,
    path: "/search",
    // ?focus: opened from the meetings list's search field, keyboard up.
    validateSearch: (s: Record<string, unknown>): { focus?: boolean } => ({
      focus: s.focus === true || s.focus === "true" ? true : undefined,
    }),
    component: function SearchRoute() {
      const { focus } = search.useSearch();
      const navigate = useNavigate();
      // The flag is spent on arrival: drop it from the URL so Back from a result does not raise the keyboard again.
      const [arrivedWithFocus] = useState(Boolean(focus));
      useEffect(() => {
        if (focus) void navigate({ to: "/search", search: {}, replace: true });
      }, [focus, navigate]);
      return <SearchScreen autoFocus={arrivedWithFocus} />;
    },
  });
  return search;
}
