// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { ConsentBanner } from "./toolbar";
import { renderLive, setLive } from "./test-utils";

afterEach(cleanup);

describe("ConsentBanner", () => {
  it("offers the copy button and goes away when dismissed", async () => {
    setLive({ state: "recording", meeting: "dismiss-me" });
    renderLive(<ConsentBanner meeting="dismiss-me" />);
    expect(screen.getByRole("button", { name: "Copy consent message" })).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByTestId("consent-hint")).toBeNull();
  });

  it("is not shown once consent is confirmed", () => {
    setLive({ state: "recording", meeting: "m-ok", session: { mode: "call", language: null, title: "", consentConfirmed: true } });
    renderLive(<ConsentBanner meeting="m-ok" />);
    expect(screen.queryByTestId("consent-hint")).toBeNull();
  });
});
