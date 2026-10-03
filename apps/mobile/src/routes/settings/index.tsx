// SPDX-License-Identifier: Apache-2.0
// Settings routes (owned by 16-J): the list at /settings and its screens
// (models, voice profile, privacy, cloud, consent, about). router.tsx mounts
// what this file returns; add child routes here, never in router.tsx.
import { createRoute, Outlet, type AnyRoute } from "@tanstack/react-router";
import { ModelsScreen } from "../../features/models";
import { AboutScreen } from "./about";
import { CloudScreen } from "./cloud";
import { ConsentScreen } from "./consent";
import { SettingsHome } from "./home";
import { LicensesScreen } from "./licenses";
import { PrivacyScreen } from "./privacy";
import { VoiceScreen } from "./voice";

export function settingsRoutes(parent: AnyRoute) {
  const settings = createRoute({ getParentRoute: () => parent, path: "/settings", component: Outlet });
  const at = <P extends string>(path: P, component: () => React.JSX.Element) => createRoute({ getParentRoute: () => settings, path, component });
  return settings.addChildren([
    at("/", SettingsHome),
    at("/models", ModelsScreen),
    at("/voice", VoiceScreen),
    at("/privacy", PrivacyScreen),
    at("/cloud", CloudScreen),
    at("/consent", ConsentScreen),
    at("/about", AboutScreen),
    at("/about/licenses", LicensesScreen),
  ]);
}
