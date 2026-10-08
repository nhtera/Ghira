// SPDX-License-Identifier: Apache-2.0

import { Segmented } from "@/components/site/segmented";
import { landing } from "@/content/landing";
import { useMeetingLang } from "./hooks/use-meeting-lang";

/** The "Meeting language" row. Every instance shares one pick, so they stay in step. */
export function LangSwitch() {
  const [lang, setLang] = useMeetingLang();
  return (
    <div className="lang-row">
      <span>{landing.demo.langLabel}</span>
      <Segmented
        label={landing.demo.langLabel}
        value={lang}
        onChange={setLang}
        options={[
          { value: "en", label: landing.demo.english },
          { value: "vi", label: landing.demo.vietnamese, lang: "vi" },
        ]}
      />
    </div>
  );
}
