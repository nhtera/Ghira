// SPDX-License-Identifier: Apache-2.0
// The app lock as the shell knows it (shell/lock-gate.tsx keeps it current):
// `null` until the core has answered.
import { create } from "zustand";

export const useLock = create<{ locked: boolean | null }>(() => ({ locked: null }));
