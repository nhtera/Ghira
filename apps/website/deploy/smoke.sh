#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Smoke checks against the deployed site:
# - the home page and docs pages answer 200 (no redirect) with every
#   security header; /docs/x/ redirects to /docs/x;
# - an unknown path is the 404 page with status 404;
# - no response sets a cookie, no page carries Cloudflare-injected
#   /cdn-cgi/ content, and every inline script is allowed by the page's CSP
#   (a zone feature that injects scripts or cookies fails here);
# - robots.txt is byte-equal to the build's (no managed robots.txt);
# - hashed assets are cached as immutable; /llms.txt is served.
# When live (second argument "1", the ghira.app domain): also the www
# redirect, every published nav page, and the security mail address.
#
#   apps/website/deploy/smoke.sh [https://ghira.app] [live: 1] 
#   BUILD_ASSETS=<dir> compares robots.txt with the build output.
set -euo pipefail

base="${1:-https://ghira.app}"
live="${2:-}"
here="$(cd "$(dirname "$0")" && pwd)"
fail=0
check() { echo "$1"; [ "$2" = ok ] || fail=1; }

headers() { curl -fsS -o /dev/null -D - --retry 5 --retry-delay 5 --retry-all-errors "$1" | tr -d '\r'; }
status() { curl -sS -o /dev/null -w '%{http_code}' --retry 5 --retry-delay 5 "$1"; }
body() { curl -fsS --retry 5 --retry-delay 5 --retry-all-errors "$1"; }

pages=(/ /docs /docs/privacy)
[ "$live" = 1 ] && pages+=(/docs/getting-started /docs/security)
for path in "${pages[@]}"; do
  h="$(headers "$base$path")"
  code="$(printf '%s\n' "$h" | awk 'toupper($1) ~ /^HTTP/ {c=$2} END {print c}')"
  [ "$code" = 200 ] && check "$path: 200" ok || check "$path: status $code (want 200)" bad
  for name in strict-transport-security content-security-policy x-content-type-options x-frame-options referrer-policy permissions-policy; do
    if printf '%s\n' "$h" | grep -qi "^$name:"; then check "$path: $name" ok; else check "$path: missing $name" bad; fi
  done
  if printf '%s\n' "$h" | grep -qi '^set-cookie:'; then check "$path: sets a cookie" bad; else check "$path: no cookie" ok; fi
  # Pages are piped, never held in a shell variable: the hydration data
  # carries a NUL byte, which bash would drop (and the script hash with it).
  # -a: grep would otherwise print "binary file matches". Counts, not -q: -q
  # exits at the first match and, under pipefail, curl's SIGPIPE would flip it.
  if [ "$(body "$base$path" | grep -ac '/cdn-cgi/')" != 0 ]; then check "$path: Cloudflare-injected /cdn-cgi/ content" bad; else check "$path: no /cdn-cgi/" ok; fi
  csp="$(printf '%s\n' "$h" | grep -i '^content-security-policy:' | cut -d: -f2-)"
  if body "$base$path" | node "$here/check-inline-scripts.mjs" "$csp"; then check "$path: inline scripts allowed by the CSP" ok; else check "$path: an inline script is not in the CSP" bad; fi
done

code="$(curl -sS -o /dev/null -w '%{http_code} %{redirect_url}' "$base/docs/privacy/")"
[ "$code" = "307 $base/docs/privacy" ] || [ "$code" = "301 $base/docs/privacy" ] || [ "$code" = "308 $base/docs/privacy" ] && check "/docs/privacy/: redirects to /docs/privacy" ok || check "/docs/privacy/: $code" bad

nope="$base/nope-$(date +%s)"
code="$(status "$nope")"
[ "$code" = 404 ] && check "/nope: 404" ok || check "/nope: status $code (want 404)" bad
[ "$(curl -sS --retry 5 "$nope" 2>/dev/null | grep -ac "Page not found")" != 0 ] && check "/nope: site 404 page" ok || check "/nope: not the site 404 page" bad
if curl -sS -o /dev/null -D - "$nope" | tr -d '\r' | grep -qi '^set-cookie:'; then check "/nope: sets a cookie" bad; else check "/nope: no cookie" ok; fi

[ "$(status "$base/llms.txt")" = 200 ] && check "/llms.txt: 200" ok || check "/llms.txt: not served" bad

if [ -n "${BUILD_ASSETS:-}" ]; then
  if cmp -s <(body "$base/robots.txt") "$BUILD_ASSETS/robots.txt"; then check "/robots.txt: as built" ok; else check "/robots.txt: differs from the build (managed robots.txt?)" bad; fi
fi

asset="$(body "$base/" | grep -ao '/assets/[^"]*\.js' | sed -n 1p)"
if [ -n "$asset" ] && headers "$base$asset" | grep -qi '^cache-control:.*immutable'; then check "$asset: immutable" ok; else check "${asset:-no asset found}: not immutable" bad; fi

if [ "$live" = 1 ]; then
  code="$(curl -sS -o /dev/null -w '%{http_code} %{redirect_url}' "https://www.ghira.app/docs?x=1")"
  [ "$code" = "301 https://ghira.app/docs?x=1" ] && check "www: 301 to the apex" ok || check "www: $code (want 301 https://ghira.app/docs?x=1)" bad
  [ "$(body "$base/docs/security" | grep -ac 'mailto:security@ghira.app')" != 0 ] && check "/docs/security: security@ghira.app" ok || check "/docs/security: no mailto:security@ghira.app" bad
fi

exit "$fail"
