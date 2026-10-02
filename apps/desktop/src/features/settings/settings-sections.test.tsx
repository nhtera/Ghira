// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Licenses } from "../../generated/licenses";
import { ipc } from "../../ipc";
import { AiSection } from "./ai-section";
import { LanguagesSection } from "./languages-section";
import { LicensesList } from "./licenses-list";
import { PrivacySection } from "./privacy-section";
import { renderSettings } from "./test-utils";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("custom vocabulary", () => {
  it("adds a term, refuses a duplicate and counts n/200", async () => {
    const user = userEvent.setup();
    renderSettings(<LanguagesSection />);
    const input = await screen.findByRole("textbox", { name: "Add a term" });
    const count = () => screen.getByTestId("vocab-count").textContent;
    await waitFor(() => expect(count()).toMatch(/^\d+\/200$/));
    const n = Number(count()!.split("/")[0]);
    await user.type(input, "Zzterm{Enter}");
    await waitFor(() => expect(count()).toBe(`${n + 1}/200`));
    await user.type(input, "zzterm{Enter}");
    expect(await screen.findByText("Already in the list.")).toBeTruthy();
    expect(count()).toBe(`${n + 1}/200`);
    await user.click(screen.getByRole("button", { name: "Remove Zzterm" }));
    await waitFor(() => expect(count()).toBe(`${n}/200`));
  });
});

describe("provider keys", () => {
  it("never puts a saved key back into the page", async () => {
    const user = userEvent.setup();
    const secret = "sk-test-SECRET-1234567890";
    renderSettings(<AiSection />);
    const input = await screen.findByLabelText("OpenAI API key");
    await user.type(input, secret);
    await user.click(screen.getByRole("button", { name: "Save OpenAI key" }));
    const row = await screen.findByTestId("key-openai");
    await within(row).findByText("Key saved");
    expect(document.body.innerHTML).not.toContain(secret);
    expect(screen.queryByLabelText("OpenAI API key")).toBeNull();
    await user.click(within(row).getByRole("button", { name: "Remove OpenAI key" }));
    await screen.findByLabelText("OpenAI API key");
  });

  it("starts with an empty request log", async () => {
    renderSettings(<AiSection />);
    expect((await screen.findByTestId("log-empty")).textContent).toBe("Nothing has been sent.");
  });
});

describe("privacy", () => {
  it("asks before shortening audio retention, then applies it", async () => {
    const user = userEvent.setup();
    const update = vi.spyOn(ipc.commands, "updateSettings");
    renderSettings(<PrivacySection />);
    const group = await screen.findByRole("radiogroup", { name: "Keep audio for" });
    await user.click(within(group).getByRole("radio", { name: "30 days" }));
    const confirm = await screen.findByRole("alertdialog");
    expect(confirm.textContent).toContain("Audio older than 30 days will be deleted now");
    expect(update).not.toHaveBeenCalled();
    await user.click(within(confirm).getByRole("button", { name: "Delete older audio" }));
    await waitFor(() => expect(update).toHaveBeenCalledWith({ audioRetentionDays: 30 }));
  });

  it("only exports with a long enough, repeated password", async () => {
    const user = userEvent.setup();
    const exp = vi.spyOn(ipc.commands, "exportEverything");
    renderSettings(<PrivacySection />);
    const button = await screen.findByRole("button", { name: "Export everything…" });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    await user.type(screen.getByLabelText("Password"), "short");
    expect((button as HTMLButtonElement).disabled).toBe(true);
    await user.clear(screen.getByLabelText("Password"));
    await user.type(screen.getByLabelText("Password"), "correct horse");
    await user.type(screen.getByLabelText("Repeat the password"), "correct hors");
    expect((button as HTMLButtonElement).disabled).toBe(true);
    await user.type(screen.getByLabelText("Repeat the password"), "e");
    expect((button as HTMLButtonElement).disabled).toBe(false);
    await user.click(button);
    await waitFor(() => expect(exp).toHaveBeenCalledWith("correct horse"));
  });

  it("deletes everything only after the word is typed", async () => {
    const user = userEvent.setup();
    const del = vi.spyOn(ipc.commands, "deleteAllData");
    renderSettings(<PrivacySection />);
    await user.click(await screen.findByRole("button", { name: "Delete all meetings and voice data…" }));
    const go = screen.getByRole("button", { name: "Delete everything" }) as HTMLButtonElement;
    expect(go.disabled).toBe(true);
    await user.type(screen.getByLabelText("Type DELETE to confirm"), "delete");
    expect(go.disabled).toBe(false);
    await user.click(go);
    await waitFor(() => expect(del).toHaveBeenCalledTimes(1));
  });
});

describe("delete everything, double submit", () => {
  it("calls deleteAllData once and locks the form while it runs", async () => {
    const user = userEvent.setup();
    let release: (v: { status: "ok"; data: null }) => void = () => {};
    const del = vi.spyOn(ipc.commands, "deleteAllData").mockImplementation(() => new Promise((r) => (release = r)));
    renderSettings(<PrivacySection />);
    await user.click(await screen.findByRole("button", { name: "Delete all meetings and voice data…" }));
    const input = screen.getByLabelText("Type DELETE to confirm") as HTMLInputElement;
    await user.type(input, "DELETE{Enter}");
    await user.type(input, "{Enter}");
    expect(del).toHaveBeenCalledTimes(1);
    expect(input.disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Delete everything" }) as HTMLButtonElement).disabled).toBe(true);
    release({ status: "ok", data: null });
  });

  it("releases the form and shows the error when it fails", async () => {
    const user = userEvent.setup();
    const del = vi.spyOn(ipc.commands, "deleteAllData").mockResolvedValue({ status: "error", error: "store is busy" });
    renderSettings(<PrivacySection />);
    await user.click(await screen.findByRole("button", { name: "Delete all meetings and voice data…" }));
    const input = screen.getByLabelText("Type DELETE to confirm") as HTMLInputElement;
    await user.type(input, "DELETE{Enter}");
    await waitFor(() => expect(input.disabled).toBe(false));
    expect(await screen.findByText(/store is busy/)).toBeTruthy();
    await user.type(input, "{Enter}");
    await waitFor(() => expect(del).toHaveBeenCalledTimes(2));
  });
});

const DATA: Licenses = {
  licenses: { mit: { id: "MIT", name: "MIT", text: "Permission is hereby granted" } },
  rust: [
    { name: "serde", version: "1.0.200", license: "MIT", licenseKeys: ["mit"] },
    { name: "tokio", version: "1.40.0", license: "MIT", licenseKeys: ["mit"] },
  ],
  js: [],
  models: [{ id: "m", name: "nvidia/model", license: "OpenMDW-1.1", url: "https://example.org/m", licenseKeys: [] }],
  assets: [],
};

describe("licenses list", () => {
  it("searches and shows the license text of a row", async () => {
    const user = userEvent.setup();
    renderSettings(<LicensesList loader={async () => DATA} />);
    await screen.findByText("tokio");
    await user.type(screen.getByRole("searchbox", { name: "Search licenses" }), "serde");
    expect(screen.queryByText("tokio")).toBeNull();
    await user.click(screen.getByRole("button", { name: /serde/ }));
    expect(screen.getByText("Permission is hereby granted")).toBeTruthy();
  });

  it("shows the URL of a model whose license text is not bundled", async () => {
    const user = userEvent.setup();
    renderSettings(<LicensesList loader={async () => DATA} />);
    await user.click(await screen.findByRole("button", { name: /nvidia\/model/ }));
    expect(screen.getByText("https://example.org/m")).toBeTruthy();
  });
});
