// SPDX-License-Identifier: Apache-2.0
// Settings → Shortcuts: the keyboard map (shell/shortcuts.ts) with platform
// labels. The two global ones (they work from any app) can be turned off.
import { useTranslation } from "react-i18next";
import { Kbd, shortcutLabel, usePlatform } from "@ghi/ui";
import { APP_NAME } from "@ghi/i18n";
import { SHORTCUTS, type ShortcutId } from "../../shell/shortcuts";
import { Card, Row, Switch, useSettings } from "./parts";

const LOCAL: { id: ShortcutId; label: string }[] = [
  { id: "commandPalette", label: "settings.shortcuts.commandPalette" },
  { id: "settings", label: "settings.shortcuts.settings" },
  { id: "find", label: "settings.shortcuts.find" },
  { id: "notesTab", label: "settings.shortcuts.notesTab" },
  { id: "transcriptTab", label: "settings.shortcuts.transcriptTab" },
  { id: "export", label: "settings.shortcuts.export" },
];

function Keys({ id }: { id: ShortcutId }) {
  const platform = usePlatform();
  return <Kbd shortcut={shortcutLabel(SHORTCUTS[id], platform)} size="md" />;
}

export function ShortcutsSection() {
  const { t } = useTranslation();
  const { settings, patch } = useSettings();
  return (
    <div className="flex flex-col gap-4">
      {settings && (
        <Card title={t("settings.shortcuts.globalTitle")} hint={t("settings.shortcuts.globalHint")}>
          <Row label={t("settings.shortcuts.startStop")} id="sc-record">
            <Keys id="toggleRecording" />
            <Switch checked={settings.globalRecordShortcut} onChange={(v) => void patch({ globalRecordShortcut: v })} labelledBy="sc-record" />
          </Row>
          <Row label={t("settings.shortcuts.markMoment")} id="sc-mark">
            <Keys id="mark" />
            <Switch checked={settings.globalMarkShortcut} onChange={(v) => void patch({ globalMarkShortcut: v })} labelledBy="sc-mark" />
          </Row>
        </Card>
      )}
      <Card title={t("settings.shortcuts.inApp", { app: APP_NAME })}>
        {LOCAL.map((s) => (
          <Row key={s.id} label={t(s.label as "settings.shortcuts.commandPalette")}>
            <Keys id={s.id} />
          </Row>
        ))}
      </Card>
    </div>
  );
}
