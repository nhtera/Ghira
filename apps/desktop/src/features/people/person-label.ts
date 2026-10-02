// SPDX-License-Identifier: Apache-2.0
// How a person is named on screen: Me has no name in the data.
import type { TFunction } from "i18next";
import type { PersonRow } from "../../bindings";

export const personName = (p: Pick<PersonRow, "name" | "isMe">, t: TFunction): string => (p.isMe ? t("speakers.me") : p.name);
