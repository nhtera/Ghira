// SPDX-License-Identifier: Apache-2.0
// Whether cloud AI is offered at all (Settings → AI). Off by default: the
// meeting's cloud entry points stay hidden until it is turned on.
import { useQuery } from "@tanstack/react-query";
import { settingsQuery } from "../../shell/root-view";

export const useCloudOffered = (): boolean => useQuery(settingsQuery).data?.cloudOffered ?? false;
