// SPDX-License-Identifier: Apache-2.0
// M1 onboarding routes (owned by 16-H). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
import { createRoute, type AnyRoute } from "@tanstack/react-router";
import { OnboardingFlow } from "../../features/onboarding/onboarding-flow";

export function onboardingRoutes(parent: AnyRoute) {
  return createRoute({ getParentRoute: () => parent, path: "/onboarding", component: OnboardingFlow });
}
