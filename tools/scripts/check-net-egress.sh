#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# RT-6: only crates/ghi-net may open network connections. Fails if code
# elsewhere uses sockets or HTTP clients. deny.toml bans the crates themselves;
# this also catches std/libc sockets, shelling out to curl, and native code.
set -euo pipefail
cd "$(dirname "$0")/../.."

outside=(':!crates/ghi-net/**' ':!third_party/**' ':!tools/scripts/check-net-egress.sh')

# Network crates as dependencies (any Cargo.toml outside ghi-net).
deps='^[[:space:]]*"?(reqwest|hyper|hyper-util|ureq|isahc|attohttpc|surf|curl|socket2|tungstenite|tokio-tungstenite|quinn|tauri-plugin-(http|websocket|shell|upload))"?[[:space:]]*='
# Rust sockets and processes that reach the network.
# (`\b` is not portable in git grep's ERE, so word boundaries are spelled out.)
w='(^|[^A-Za-z0-9_])'
rust="${w}(TcpListener|TcpStream|UdpSocket)(\$|[^A-Za-z0-9_])|${w}(libc|nix)::(connect|socket|bind)|${w}(reqwest|hyper|ureq|socket2|tungstenite|tokio_tungstenite|quinn)::|Command::new\\(\"(curl|wget)\""

# Native plugins (phases 4, 7, 17): Swift and Kotlin networking APIs.
native="${w}(URLSession|NWConnection|NWListener|CFSocket|HttpURLConnection|okhttp3|ServerSocket)|java\\.net\\.(Socket|URL)"
# First-party C/C++ (phase 3 onward): sockets, DNS and HTTP clients.
cpp="${w}(socket|connect|getaddrinfo|gethostbyname)\\(|httplib::|curl_easy_|WinHttp[A-Z]|InternetOpen"


status=0
scan() { # pattern, pathspecs...
  local pattern="$1"; shift
  local rc=0
  git grep -n -I --untracked -E "$pattern" -- "$@" "${outside[@]}" || rc=$?
  if [[ $rc -eq 0 ]]; then status=1; elif [[ $rc -ne 1 ]]; then echo "git grep failed ($rc)" >&2; exit 2; fi
}

scan "$deps" '*Cargo.toml'
scan "$rust" '*.rs'
scan "$native" '*.swift' '*.kt' '*.kts' '*.java' '*.m' '*.mm'
scan "$cpp" '*.c' '*.cc' '*.cpp' '*.h' '*.hpp' '*.m' '*.mm'

if [[ $status -ne 0 ]]; then
  echo "Network code outside crates/ghi-net (RT-6); see matches above." >&2
  exit 1
fi
echo "net egress: ok (network code only in crates/ghi-net)"
