#!/usr/bin/env bash
# Package the Guitar Studio plugin as macOS bundles:
#   target/plugin/Guitar Studio.vst3   and   target/plugin/Guitar Studio.clap
# Each holds the plugin binary and the plugin scanner (Contents/MacOS) and the built UI
# (Contents/Resources/ui). Run through `make plugin`, which builds those first.
#
# Usage: scripts/bundle-plugin.sh <cargo profile dir, e.g. target/release>
set -euo pipefail

BUILD="${1:-target/release}"
OUT="target/plugin"
NAME="Guitar Studio"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

[[ -f "$BUILD/libgs_plugin.dylib" ]] || { echo "missing $BUILD/libgs_plugin.dylib"; exit 1; }
[[ -f "$BUILD/gs-scan" ]] || { echo "missing $BUILD/gs-scan"; exit 1; }
[[ -f dist/index.html ]] || { echo "missing dist/ (npm run build)"; exit 1; }

bundle() { # <extension> <bundle id>
  local dir="$OUT/$NAME.$1"
  rm -rf "$dir"
  mkdir -p "$dir/Contents/MacOS" "$dir/Contents/Resources"
  cp "$BUILD/libgs_plugin.dylib" "$dir/Contents/MacOS/$NAME"
  cp "$BUILD/gs-scan" "$dir/Contents/MacOS/gs-scan"
  cp -R dist "$dir/Contents/Resources/ui"
  printf 'BNDL????' > "$dir/Contents/PkgInfo"
  cat > "$dir/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>English</string>
  <key>CFBundleExecutable</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$2</string>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleSignature</key><string>????</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
  # Ad-hoc signature: enough for a DAW on this Mac (Apple silicon refuses unsigned code).
  codesign --force --sign - "$dir/Contents/MacOS/gs-scan"
  codesign --force --sign - "$dir"
  echo "built $dir"
}

bundle vst3 dev.guitarstudio.plugin.vst3
bundle clap dev.guitarstudio.plugin.clap
