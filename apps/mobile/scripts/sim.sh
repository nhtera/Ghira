#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Simulator-only helper for the iOS app (phase 16-B). Never touches a physical
# device: the target must be a UDID that `simctl` lists as a simulator, and
# `devicectl` is never called.
#
#   apps/mobile/scripts/sim.sh [--udid UDID] <command> [args]
#
#   create [name]             create an iPhone simulator on the newest iOS runtime and print its UDID
#                             (CI: echo "GHI_SIM_UDID=$(sim.sh create)" >> "$GITHUB_ENV")
#   udid                      print the resolved simulator UDID
#   boot                      boot the simulator (and show the Simulator app)
#   install [path/to/Ghira.app]   install the built app (default: the newest simulator build)
#   launch [--env K=V]... [-- app args]   (re)launch; K=V reaches the app as K (SIMCTL_CHILD_)
#   models                    copy the pinned models from models/ into the app container (SHA-256 checked)
#   grant                     grant the microphone permission (simctl privacy)
#   ui dark|light|size <category>|reset   appearance and Dynamic Type
#   container [app|data|group]    print a container path (default: data)
#   reset                     uninstall the app and reset its privacy grants
#
# Simulator: $GHI_SIM_UDID / --udid when given (any simulator, whatever runtime); else the
# iPhone 17 Pro on iOS 26.3 (override the lookup with $GHI_SIM_NAME / $GHI_SIM_OS).
# Languages: pass launch args, e.g.  launch -- -AppleLanguages "(vi)" -AppleLocale vi_VN
# Test hooks: build with `build-ios.sh --sim --test-hooks`, then
#   launch --env GHI_FAKE_MIC=/path/in.wav   (a path the simulator can read)
# Native self-test:  launch --env GHI_SELFTEST=<wav in Documents>  (see models).
set -euo pipefail

root="$(cd "$(dirname "$0")/../../.." && pwd)"
bundle="${GHI_BUNDLE_ID:-com.nhtera.ghira}"
group="group.com.nhtera.ghira"
models_dir="${GHI_MODELS_DIR:-$root/models}"
want_name="${GHI_SIM_NAME:-iPhone 17 Pro}"
want_os="${GHI_SIM_OS:-26.3}"

die() { echo "sim.sh: $*" >&2; exit 1; }

udid="${GHI_SIM_UDID:-}"
if [[ "${1:-}" == --udid ]]; then
  udid="${2:?--udid needs a value}"
  shift 2
fi
cmd="${1:-}"
[[ -n "$cmd" ]] || { sed -n "3,27p" "$0" | sed 's/^# \{0,1\}//'; exit 2; }
shift

# All simulators simctl knows: "udid<TAB>name<TAB>os<TAB>state".
list_sims() {
  xcrun simctl list devices available -j | python3 -c '
import json, sys
for runtime, devs in json.load(sys.stdin)["devices"].items():
    os_ver = runtime.rsplit("iOS-", 1)[-1].replace("-", ".") if "iOS-" in runtime else ""
    for d in devs:
        print("\t".join([d["udid"], d["name"], os_ver, d["state"]]))
'
}

resolve_udid() {
  local sims
  sims="$(list_sims)"
  if [[ -n "$udid" ]]; then
    # Refuse anything that is not a simulator (a physical iPhone UDID is not listed).
    grep -q "^$udid	" <<<"$sims" || die "refusing $udid: not a simulator known to simctl"
    return
  fi
  udid="$(awk -F'\t' -v n="$want_name" -v o="$want_os" '$2 == n && $3 == o {print $1; exit}' <<<"$sims")"
  [[ -n "$udid" ]] || die "no '$want_name' simulator on iOS $want_os (set GHI_SIM_UDID, GHI_SIM_NAME, GHI_SIM_OS)"
}

# `create` runs before the lookup: a runner image may have no matching simulator yet.
if [[ "$cmd" == create ]]; then
  name="${1:-Ghira CI}"
  python3 - "$name" <<'PY' | { read -r rt dt || exit 1; xcrun simctl create "$name" "$dt" "$rt"; }
import json, subprocess, sys
def simctl(*a):
    return json.loads(subprocess.check_output(["xcrun", "simctl", "list", *a, "-j"]))
def ver(s):
    return tuple(int(x) for x in s.split("."))
runtimes = [r for r in simctl("runtimes")["runtimes"] if r.get("isAvailable") and r["identifier"].split(".")[-1].startswith("iOS-")]
if not runtimes:
    sys.exit("sim.sh: no iOS simulator runtime installed")
rt = max(runtimes, key=lambda r: ver(r["version"]))
types = [t for t in simctl("devicetypes")["devicetypes"] if t["name"].startswith("iPhone")]
supported = {t["identifier"] for t in rt.get("supportedDeviceTypes", [])}
types = [t for t in types if t["identifier"] in supported] or types
pref = [t for t in types if t["name"] in ("iPhone 17 Pro", "iPhone 16 Pro", "iPhone 15 Pro")]
dt = (pref or types)[-1]
print(rt["identifier"], dt["identifier"])
PY
  exit
fi

resolve_udid

state() { list_sims | awk -F'\t' -v u="$udid" '$1 == u {print $4}'; }
ensure_booted() { [[ "$(state)" == Booted ]] || die "simulator $udid is not booted: run '$0 boot'"; }
container() { xcrun simctl get_app_container "$udid" "$bundle" "$1" 2>/dev/null || die "app $bundle is not installed on $udid (install it first)"; }

# Pinned models for the phone: ASR, diarization, voice (registry.toml).
registry_models() {
  python3 - "$root/crates/ghi-models/registry.toml" <<'PY'
import sys, tomllib
with open(sys.argv[1], "rb") as f:
    reg = tomllib.load(f)
for m in reg["model"]:
    if m["role"] in ("asr", "diarization", "voice"):
        print(m["file"], m["sha256"], m["size"], sep="\t")
PY
}

case "$cmd" in
  udid) echo "$udid" ;;
  boot)
    [[ "$(state)" == Booted ]] || xcrun simctl boot "$udid"
    xcrun simctl bootstatus "$udid" -b >/dev/null
    open -a Simulator --args -CurrentDeviceUDID "$udid" >/dev/null 2>&1 || true
    echo "booted $udid"
    ;;
  install)
    ensure_booted
    app="${1:-}"
    if [[ -z "$app" ]]; then
      app="$(find "$root/apps/mobile/src-tauri/gen/apple/build" -maxdepth 6 -name '*.app' \
        -exec stat -f '%m %N' {} + 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)"
    fi
    [[ -d "$app" ]] || die "no simulator .app found: run apps/mobile/scripts/build-ios.sh --sim"
    xcrun simctl install "$udid" "$app"
    echo "installed $app"
    ;;
  launch)
    ensure_booted
    envs=()
    while [[ "${1:-}" == --env ]]; do
      envs+=("SIMCTL_CHILD_${2:?--env needs K=V}")
      shift 2
    done
    [[ "${1:-}" == -- ]] && shift
    env ${envs[@]+"${envs[@]}"} xcrun simctl launch --terminate-running-process "$udid" "$bundle" "$@"
    ;;
  models)
    ensure_booted
    data="$(container data)"
    # The spike shell reads Documents/models; the v1 app Library/Application Support/Ghira/models.
    dests=("$data/Library/Application Support/Ghira/models" "$data/Documents/models")
    while IFS=$'\t' read -r file sha size; do
      src="$models_dir/$file"
      [[ -f "$src" ]] || die "missing $src: run tools/scripts/fetch-models.sh"
      [[ "$(stat -f %z "$src")" == "$size" ]] || die "$file: size mismatch"
      echo "checking $file..."
      [[ "$(shasum -a 256 "$src" | cut -d' ' -f1)" == "$sha" ]] || die "$file: SHA-256 mismatch"
      for dest in "${dests[@]}"; do
        mkdir -p "$dest"
        rm -f "$dest/$file"
        cp -c "$src" "$dest/$file" 2>/dev/null || cp "$src" "$dest/$file"
      done
      echo "  copied $file"
    done < <(registry_models)
    ;;
  grant)
    xcrun simctl privacy "$udid" grant microphone "$bundle"
    echo "microphone granted to $bundle"
    ;;
  ui)
    ensure_booted
    case "${1:-}" in
      dark | light) xcrun simctl ui "$udid" appearance "$1" ;;
      size) xcrun simctl ui "$udid" content_size "${2:?size needs a category, e.g. accessibility-extra-extra-extra-large}" ;;
      reset)
        xcrun simctl ui "$udid" appearance light
        xcrun simctl ui "$udid" content_size large
        ;;
      *) die "ui: dark | light | size <category> | reset" ;;
    esac
    ;;
  container)
    case "${1:-data}" in
      app | data) container "${1:-data}" ;;
      group) xcrun simctl get_app_container "$udid" "$bundle" "$group" ;;
      *) die "container: app | data | group" ;;
    esac
    ;;
  reset)
    xcrun simctl terminate "$udid" "$bundle" >/dev/null 2>&1 || true
    xcrun simctl uninstall "$udid" "$bundle" >/dev/null 2>&1 || true
    xcrun simctl privacy "$udid" reset all "$bundle" >/dev/null 2>&1 || true
    echo "reset $bundle on $udid"
    ;;
  *) die "unknown command '$cmd' (create|udid|boot|install|launch|models|grant|ui|container|reset)" ;;
esac
