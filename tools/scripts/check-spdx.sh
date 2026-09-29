#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Every first-party source file carries `SPDX-License-Identifier: Apache-2.0`
# in its first 5 lines (docs/05 §4.1).
set -euo pipefail
cd "$(dirname "$0")/../.."

missing=0
while IFS= read -r f; do
  [[ -f "$f" ]] || continue
  if ! head -n 5 "$f" | grep -q 'SPDX-License-Identifier: Apache-2.0'; then
    echo "missing SPDX header: $f" >&2
    missing=1
  fi
done < <(git ls-files --cached --others --exclude-standard -- \
  '*.rs' '*.ts' '*.tsx' '*.js' '*.mjs' '*.cjs' '*.css' '*.html' '*.hbs' \
  '*.swift' '*.kt' '*.kts' '*.java' '*.c' '*.h' '*.m' '*.mm' '*.py' '*.sh' '*.ps1' \
  ':!:third_party/**')

if [[ $missing -ne 0 ]]; then
  echo "Add '// SPDX-License-Identifier: Apache-2.0' (or '#' for scripts) near the top." >&2
  exit 1
fi
echo "spdx: ok"
