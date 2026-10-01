// SPDX-License-Identifier: Apache-2.0
import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";
import { noLiteralStrings, noNetwork, safeRendering } from "../../eslint.shared.mjs";

export default tseslint.config(
  { ignores: ["gallery/dist"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
    plugins: { "react-hooks": reactHooks },
    rules: reactHooks.configs.recommended.rules,
  },
  { files: ["src/**/*.{ts,tsx}", "gallery/**/*.{ts,tsx}"], ...noNetwork },
  { files: ["src/**/*.{ts,tsx}", "gallery/**/*.{ts,tsx}"], ...safeRendering },
  // Stories, tests and the gallery may use sample text.
  { files: ["src/**/*.tsx"], ignores: ["src/**/*.stories.tsx", "src/**/*.test.tsx"], ...noLiteralStrings },
  { files: ["scripts/**/*.mjs"], languageOptions: { globals: globals.node } },
);
