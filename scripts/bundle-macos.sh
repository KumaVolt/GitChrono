#!/usr/bin/env bash
# Build GitChrono.app (+ .dmg) from one or more release binaries.
#   scripts/bundle-macos.sh <version> <binary> [<binary>...]
# Several binaries (e.g. arm64 + x86_64) are merged into a universal binary.
set -euo pipefail

version="$1"; shift
out="dist"
app="$out/GitChrono.app"

rm -rf "$app" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
if [ "$#" -gt 1 ]; then
  lipo -create "$@" -output "$app/Contents/MacOS/gitchrono"
else
  cp "$1" "$app/Contents/MacOS/gitchrono"
fi
chmod +x "$app/Contents/MacOS/gitchrono"
cp packaging/macos/GitChrono.icns "$app/Contents/Resources/"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>GitChrono</string>
  <key>CFBundleDisplayName</key><string>GitChrono</string>
  <key>CFBundleIdentifier</key><string>com.gitchrono.app</string>
  <key>CFBundleExecutable</key><string>gitchrono</string>
  <key>CFBundleIconFile</key><string>GitChrono</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature so Apple Silicon will launch it; replaced by a Developer ID
# signature in CI once signing secrets are configured.
codesign --force --deep --sign - "$app"

dmg="$out/GitChrono-${version}-macos-universal.dmg"
[ "$#" -gt 1 ] || dmg="$out/GitChrono-${version}-macos-$(uname -m).dmg"
rm -f "$dmg"
staging="$(mktemp -d)"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -quiet -volname "GitChrono ${version}" -srcfolder "$staging" -ov -format UDZO "$dmg"
rm -rf "$staging"
echo "$dmg"
