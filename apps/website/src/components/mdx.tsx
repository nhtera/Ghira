// SPDX-License-Identifier: Apache-2.0

import type { MDXComponents } from "mdx/types";
import type { ComponentProps } from "react";
import { strings } from "@/content/strings";

// Scrollable code and tables are keyboard-focusable named groups.
const group = (label: string) => ({ role: "group", "aria-label": label, tabIndex: 0 });

export function getMDXComponents(components?: MDXComponents) {
  return {
    pre: (props: ComponentProps<"pre">) => (
      <div className="code" {...group(strings.docs.codeLabel)}>
        <pre {...props} />
      </div>
    ),
    table: (props: ComponentProps<"table">) => (
      <div className="table-scroll" {...group(strings.docs.tableLabel)}>
        <table {...props} />
      </div>
    ),
    ...components,
  } satisfies MDXComponents;
}
