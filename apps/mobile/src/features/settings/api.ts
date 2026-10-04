// SPDX-License-Identifier: Apache-2.0
// Shared plumbing of the 16-J screens: unwrapping command results, loading a
// resource, running an action with an error code, and turning codes into words.
import { useCallback, useEffect, useState } from "react";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };

/** The data of an ok result; an error result throws its code. */
export function unwrap<T>(r: Result<T>): T {
  if (r.status === "ok") return r.data;
  throw new Error(r.error);
}

export const errCode = (e: unknown): string =>
  e instanceof Error ? e.message : String(e);

/** Loads once and on `reload()`; keeps the last data while reloading. */
export function useResource<T>(load: () => Promise<T>) {
  const [state, setState] = useState<{ data?: T; error?: string }>({});
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let alive = true;
    load().then(
      (data) => alive && setState({ data }),
      (e) => alive && setState({ error: errCode(e) }),
    );
    return () => {
      alive = false;
    };
  }, [load, tick]);
  const reload = useCallback(() => setTick((n) => n + 1), []);
  const set = useCallback((data: T) => setState({ data }), []);
  return { ...state, reload, set };
}

/** Runs actions one at a time and keeps the error code of the last one. */
export function useAction() {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = useCallback(
    async <T>(f: () => Promise<T>): Promise<T | undefined> => {
      setBusy(true);
      setError(null);
      try {
        return await f();
      } catch (e) {
        setError(errCode(e));
        return undefined;
      } finally {
        setBusy(false);
      }
    },
    [],
  );
  const clear = useCallback(() => setError(null), []);
  return { run, busy, error, clear };
}

const KNOWN = [
  "busy",
  "locked",
  "offline",
  "passwordTooShort",
  "noAuthMethod",
  "notFound",
  "noModel",
  "micPermission",
  "busyRecording",
  "tooShort",
  "tooQuiet",
  "unsupportedType",
  "tooLarge",
  "unreadable",
] as const;
export type KnownError = (typeof KNOWN)[number];

/** The error code a user can act on, or "generic". Rust text such as "strict offline is on…" maps by its leading words. */
export function knownError(
  code: string | null | undefined,
): KnownError | "generic" {
  if (!code) return "generic";
  if ((KNOWN as readonly string[]).includes(code)) return code as KnownError;
  if (/strict offline/i.test(code)) return "offline";
  // Core::store() while the app lock is on.
  if (/app is locked/i.test(code)) return "locked";
  return "generic";
}
