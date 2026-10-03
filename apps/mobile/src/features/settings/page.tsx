// SPDX-License-Identifier: Apache-2.0
// A settings screen: the nav bar (large title, back to the parent) above its
// own scroll area. Load failures show a retry instead of a blank page.
import { LargeTitle, NavBar, useLargeTitleCollapse } from "@ghi/ui";
import {
  useLayoutEffect,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import { Btn } from "./controls";
import { useGo } from "./go";

/** A scroll area is a keyboard stop only while its content overflows (axe: scrollable-region-focusable). */
function useOverflows(ref: RefObject<HTMLElement | null>) {
  const [overflows, setOverflows] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setOverflows(el.scrollHeight > el.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    // The content grows without the container resizing: watch the inner wrapper.
    if (el.firstElementChild) ro.observe(el.firstElementChild);
    return () => ro.disconnect();
  }, [ref]);
  return overflows;
}

export function Page({
  title,
  back,
  trailing,
  children,
  error,
  onRetry,
}: {
  title: string;
  back?: "settings" | "privacy" | "about";
  trailing?: ReactNode;
  children?: ReactNode;
  error?: string;
  onRetry?: () => void;
}) {
  const { t } = useTranslation();
  const go = useGo();
  const { collapsed, scrollRef, titleRef } = useLargeTitleCollapse();
  const overflows = useOverflows(scrollRef);
  const onBack = () => go(back === "privacy" ? "/settings/privacy" : back === "about" ? "/settings/about" : "/settings");
  return (
    <div className="flex h-full flex-col" data-screen="settings">
      {back ? (
        <NavBar
          title={title}
          collapsed={collapsed}
          trailing={trailing}
          onBack={onBack}
          backLabel={
            back === "privacy"
              ? t("mobile.settings.rows.privacy")
              : back === "about"
                ? t("mobile.settings.about.title")
                : t("mobile.tabs.settings")
          }
        />
      ) : (
        <NavBar title={title} collapsed={collapsed} trailing={trailing} />
      )}
      <div
        ref={scrollRef}
        tabIndex={overflows ? 0 : undefined}
        className="min-h-0 flex-1 overflow-y-auto pb-6"
      >
        <div>
          <LargeTitle ref={titleRef}>{title}</LargeTitle>
          {error ? (
            <div
              role="alert"
              className="mx-4 my-3 flex flex-col items-start gap-3"
            >
              <p className="text-ios-body m-0">
                {t("mobile.settings.loadFailed")}
              </p>
              {onRetry && (
                <Btn onClick={onRetry}>{t("mobile.settings.retry")}</Btn>
              )}
            </div>
          ) : (
            children
          )}
        </div>
      </div>
    </div>
  );
}
