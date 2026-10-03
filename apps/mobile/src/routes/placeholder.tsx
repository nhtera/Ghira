// SPDX-License-Identifier: Apache-2.0
// Empty tab screens until 16-H (record, onboarding), 16-I (meetings, search)
// and 16-J (settings) replace them.
import { useTranslation } from "react-i18next";

export function Placeholder({ screen }: { screen: "meetings" | "record" | "search" | "settings" | "onboarding" }) {
  const { t } = useTranslation();
  return (
    <section data-screen={screen} className="p-4">
      <p>{t(`mobile.shell.empty.${screen}`)}</p>
    </section>
  );
}
