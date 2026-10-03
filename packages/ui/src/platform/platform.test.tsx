// SPDX-License-Identifier: Apache-2.0
import { renderHook } from "@testing-library/react";
import { createInstance } from "i18next";
import { I18nextProvider, initReactI18next } from "react-i18next";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, copyContext, detectAppPlatform, detectPlatform, shortcutLabel, usePlatformContext, type AppPlatform } from "./platform";

function withUserAgent(ua: string) {
  vi.stubGlobal("navigator", { userAgent: ua });
}

describe("platform", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("detects iPhone, Windows and Mac", () => {
    withUserAgent("Mozilla/5.0 (iPhone; CPU iPhone OS 26_3 like Mac OS X) AppleWebKit/605.1.15");
    expect(detectAppPlatform()).toBe("ios");
    expect(detectPlatform()).toBe("mac");
    withUserAgent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
    expect(detectPlatform()).toBe("win");
    withUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)");
    expect(detectPlatform()).toBe("mac");
  });

  it("keeps the desktop copy contexts and reads _mac on iOS", () => {
    expect(copyContext("mac")).toBe("mac");
    expect(copyContext("win")).toBe("win");
    expect(copyContext("ios")).toBe("mac");
  });

  it("labels shortcuts as before", () => {
    expect(shortcutLabel("Mod+K", "mac")).toBe("⌘K");
    expect(shortcutLabel("Mod+K", "win")).toBe("Ctrl+K");
      });

  it("reads key_ios when it exists, else key_mac, on iOS only", () => {
    const i18n = createInstance();
    void i18n.use(initReactI18next).init({
      lng: "en",
      resources: { en: { translation: { both_mac: "m", both_ios: "i", only_mac: "m", only_win: "w" } } },
    });
    const ctx = (platform: AppPlatform) => {
      const wrapper = ({ children }: { children: ReactNode }) => (
        <I18nextProvider i18n={i18n}>
          <PlatformProvider value={platform}>{children}</PlatformProvider>
        </I18nextProvider>
      );
      return renderHook(() => usePlatformContext(), { wrapper }).result.current;
    };
    expect(ctx("ios")("both") as string).toBe("ios");
    expect(ctx("ios")("only")).toBe("mac");
    expect(ctx("mac")("both")).toBe("mac");
    expect(ctx("win")("only")).toBe("win");
  });
});
