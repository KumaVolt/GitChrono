#!/usr/bin/env bash
# Takes the README screenshots (macOS only; needs Screen Recording permission):
#   scripts/screenshots.sh [<outdir>]   → <outdir>/{overview,diff,changes,terminal,settings}.png
# Builds the release app and the demo repos, then launches the app once per
# scene with a throwaway HOME (so your own settings and tabs are never touched)
# and captures just its window. Defaults to docs/screenshots.
set -euo pipefail

out=${1:-docs/screenshots}
settle=${SETTLE:-4} # seconds to let a scene finish loading before capturing
mkdir -p "$out"
out=$(cd "$out" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

cargo build --release -p gitchrono-app --locked
bin="${CARGO_TARGET_DIR:-$root/target}/release/gitchrono"

tmp=$(mktemp -d)
app=""
trap '[ -z "$app" ] || kill "$app" 2>/dev/null; rm -rf "$tmp"' EXIT
repos=$(scripts/demo-repos.sh "$tmp/repos" | tail -1)

# A clean HOME with three repository tabs and no update check.
home="$tmp/home"
config="$home/Library/Application Support/gitchrono"
mkdir -p "$config"
cat > "$config/session.json" <<JSON
{
  "tabs": ["$repos/aurora", "$repos/nebula-api", "$repos/orbit-docs"],
  "active": 0,
  "recent": ["$repos/aurora", "$repos/nebula-api", "$repos/orbit-docs"],
  "auto_update": false
}
JSON
# The terminal's zsh reads its startup files from here: a short, fixed prompt,
# and no "Last login" banner.
printf '%s\n' "PROMPT='%1~ %F{8}›%f '" clear > "$home/.zshrc"

# Window id of a process's main window, from CoreGraphics.
cat > "$tmp/windowid.swift" <<'SWIFT'
import CoreGraphics
let pid = Int32(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
for w in windows where w[kCGWindowOwnerPID as String] as? Int32 == pid && w[kCGWindowLayer as String] as? Int == 0 {
    let height = (w[kCGWindowBounds as String] as? [String: Any])?["Height"] as? Double ?? 0
    if height > 200 { print(w[kCGWindowNumber as String]!); break }
}
SWIFT
swiftc -O -o "$tmp/windowid" "$tmp/windowid.swift"

shot() { # name [VAR=value...]
  local name=$1; shift
  env HOME="$home" ZDOTDIR="$home" "$@" "$bin" > "$tmp/$name.log" 2>&1 &
  app=$!
  local id=""
  for _ in $(seq 1 100); do
    id=$("$tmp/windowid" "$app")
    [ -n "$id" ] && break
    sleep 0.2
  done
  if [ -z "$id" ]; then
    echo "no window for $name" >&2
    cat "$tmp/$name.log" >&2
    exit 1
  fi
  sleep "$settle"
  screencapture -o -x -l "$id" "$out/$name.png"
  kill "$app"
  wait "$app" 2>/dev/null || true
  app=""
  # Retina captures are 2880px wide; 2000 is plenty for a README.
  if [ "$(sips -g pixelWidth "$out/$name.png" | awk '/pixelWidth/ {print $2}')" -gt 2000 ]; then
    sips --resampleWidth 2000 "$out/$name.png" > /dev/null
  fi
  echo "$out/$name.png"
}

# Rows: 0 is the uncommitted changes, 3 is "Stripe checkout flow" on feature/billing.
shot overview GITCHRONO_SELECT=3
shot diff GITCHRONO_SELECT=3:diff
shot changes GITCHRONO_SELECT=0:diff
shot terminal GITCHRONO_SELECT=3 "GITCHRONO_TERMINAL=git log --oneline --graph -8"
shot settings GITCHRONO_SETTINGS=1
