// SPDX-License-Identifier: Apache-2.0
// M5 in the app: files the share extension left in the App Group inbox. A
// banner says how many wait (the extension closed before the choices were
// made); the sheet lists them with language and target to confirm. When an
// import finishes a short "Added to {{app}}" toast appears. Nothing here
// shows while the app is locked (inbox_list refuses).
import { Banner, Icon, Sheet } from "@ghi/ui";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  InboxItem,
  MeetingLanguage,
  ProcessingTarget,
} from "../../bindings";
import { ipc } from "../../ipc";
import { LOCKED_EVENT, UNLOCKED_EVENT } from "../app-lock/events";
import { InboxRow } from "./inbox-item";

const TOAST_MS = 4000;

export function ImportInbox() {
  const { t } = useTranslation();
  const [items, setItems] = useState<InboxItem[]>([]);
  const [open, setOpen] = useState(false);
  const [toast, setToast] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const importing = useRef(new Set<string>());

  const refresh = useCallback(async () => {
    const r = await ipc.commands.inboxList().catch(() => null);
    if (r?.status === "error" && /locked/i.test(r.error)) {
      // Locked: nothing of the inbox may stay on screen.
      importing.current = new Set();
      setItems([]);
      setOpen(false);
      setToast(false);
      return;
    }
    if (r?.status !== "ok") return;
    const now = new Set(
      r.data.filter((i) => i.state === "importing").map((i) => i.id),
    );
    // Something that was importing and is gone: it was added.
    const done = [...importing.current].some(
      (id) => !r.data.some((i) => i.id === id),
    );
    importing.current = now;
    setItems(r.data);
    if (done) setToast(true);
  }, []);

  useEffect(() => {
    queueMicrotask(() => void refresh());
    let off: (() => void) | undefined;
    let alive = true;
    void ipc
      .onMobileEvent((e) => {
        if (e.type === "inboxChanged") void refresh();
      })
      .then((u) => (alive ? (off = u) : u()));
    const onShow = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    document.addEventListener("visibilitychange", onShow);
    const onLocked = () => void refresh();
    window.addEventListener(UNLOCKED_EVENT, onShow);
    window.addEventListener(LOCKED_EVENT, onLocked);
    return () => {
      alive = false;
      off?.();
      document.removeEventListener("visibilitychange", onShow);
      window.removeEventListener(UNLOCKED_EVENT, onShow);
      window.removeEventListener(LOCKED_EVENT, onLocked);
    };
  }, [refresh]);

  useEffect(() => {
    if (!toast) return;
    const id = setTimeout(() => setToast(false), TOAST_MS);
    return () => clearTimeout(id);
  }, [toast]);

  const waiting = items.filter((i) => i.state === "pending").length;
  const sheetOpen = open && items.length > 0;

  const confirm = async (
    item: InboxItem,
    language: MeetingLanguage,
    target: ProcessingTarget,
  ) => {
    setErrors((e) => ({ ...e, [item.id]: "" }));
    const r = await ipc.commands
      .inboxConfirm(item.id, language, target)
      .catch((e: unknown) => ({ status: "error" as const, error: String(e) }));
    if (r.status === "error") setErrors((e) => ({ ...e, [item.id]: r.error }));
    await refresh();
  };
  const dismiss = async (item: InboxItem) => {
    await ipc.commands.inboxDismiss(item.id).catch(() => null);
    await refresh();
  };

  return (
    <>
      {(waiting > 0 || (toast && !sheetOpen)) && (
        <div className="pointer-events-none fixed inset-x-0 top-0 z-30 flex flex-col gap-2 px-3 pt-safe">
          {waiting > 0 && (
            <div className="pointer-events-auto mt-2">
              <Banner
                variant="info"
                icon="inbox"
                title={t("mobile.inbox.waiting", { count: waiting })}
                action={{
                  label: t("mobile.inbox.review"),
                  onPress: () => setOpen(true),
                }}
              />
            </div>
          )}
          {toast && !sheetOpen && (
            <div
              role="status"
              className="pointer-events-auto mt-2 flex items-center gap-2 rounded-(--ios-radius-group) bg-surface2 py-1 ps-3 pe-1 text-ink shadow-float"
            >
              <Icon
                name="check_circle"
                size={20}
                className="shrink-0 text-accent"
              />
              <span className="text-ios-subhead flex-1">
                {t("mobile.inbox.added")}
              </span>
              <button
                type="button"
                aria-label={t("mobile.inbox.closeToast")}
                onClick={() => setToast(false)}
                className="grid min-h-ios-target min-w-ios-target place-items-center text-muted"
              >
                <Icon name="close" size={20} />
              </button>
            </div>
          )}
        </div>
      )}
      <Sheet
        open={sheetOpen}
        onOpenChange={setOpen}
        title={t("mobile.inbox.title")}
        closeLabel={t("mobile.sheet.close")}
        handleLabel={t("mobile.sheet.handle")}
        detent="large"
      >
        {toast && (
          // The sheet covers the toast, so it says so here too.
          <p
            role="status"
            className="text-ios-subhead m-0 mb-2 flex items-center gap-2 text-accent"
          >
            <Icon name="check_circle" size={20} className="shrink-0" />
            {t("mobile.inbox.added")}
          </p>
        )}
        <ul className="m-0 list-none p-0">
          {items.map((item) => (
            <InboxRow
              key={item.id}
              item={item}
              error={errors[item.id] || null}
              onImport={(l, tg) => void confirm(item, l, tg)}
              onDismiss={() => void dismiss(item)}
            />
          ))}
        </ul>
      </Sheet>
    </>
  );
}
