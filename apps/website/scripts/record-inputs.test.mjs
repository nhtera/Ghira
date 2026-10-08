// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { test } from "node:test";
import { cssImports, outsideInputs } from "./record-inputs.mjs";

test("only repository files outside the site and node_modules are inputs", () => {
  const got = outsideInputs(
    [
      "/r/packages/ui/src/tokens/tokens.css?direct",
      "/r/apps/website/src/x.ts",
      "/r/apps/website/node_modules/react/index.js",
      "/r/node_modules/x/y.js",
      "\0virtual:x",
      "/elsewhere/a.ts",
      "/r/packages/i18n/locales/en.json",
    ],
    "/r",
    "/r/apps/website",
  );
  assert.deepEqual([...got].sort(), ["packages/i18n/locales/en.json", "packages/ui/src/tokens/tokens.css"]);
});

test("relative CSS @imports are followed, recursively", () => {
  const files = {
    "/r/apps/website/src/styles/site.css": '@import "tailwindcss/theme.css";\n@import "./fonts.css";\n@import "../../../../packages/ui/src/tokens/tokens.css";',
    "/r/apps/website/src/styles/fonts.css": "",
    "/r/packages/ui/src/tokens/tokens.css": '@import "./more.css";',
    "/r/packages/ui/src/tokens/more.css": "",
  };
  const got = cssImports("/r/apps/website/src/styles/site.css", (f) => {
    if (!(f in files)) throw new Error("missing");
    return files[f];
  });
  assert.deepEqual([...outsideInputs(got, "/r", "/r/apps/website")].sort(), ["packages/ui/src/tokens/more.css", "packages/ui/src/tokens/tokens.css"]);
});
