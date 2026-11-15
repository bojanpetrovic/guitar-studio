#!/usr/bin/env bash
# Package the Guitar Studio plugin into target/plugin, for the platform it was built on.
#
# macOS:   Guitar Studio.vst3  and  Guitar Studio.clap  (bundles: the plugin binary and
#          the plugin scanner in Contents/MacOS, the built UI in Contents/Resources/ui)
# Windows: Guitar Studio.vst3  (Contents/x86_64-win holds the DLL and the scanner,
#          Contents/Resources/ui the UI)  and  Guitar Studio/  (a folder for the CLAP
#          folder: Guitar Studio.clap, gs-scan.exe and Resources/ui side by side)
#
# Run through `make plugin`, which builds those first (on Windows from Git Bash, as CI does).
#
# Usage: scripts/bundle-plugin.sh <cargo profile dir, e.g. target/release>
set -euo pipefail

BUILD="${1:-target/release}"
OUT="target/plugin"
NAME="Guitar Studio"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) PLATFORM=windows ;;
  Darwin) PLATFORM=macos ;;
  *) echo "the plugin is only packaged for macOS and Windows"; exit 1 ;;
esac

[[ -f dist/index.html ]] || { echo "missing dist/ (npm run build)"; exit 1; }

if [[ $PLATFORM == windows ]]; then
  [[ -f "$BUILD/gs_plugin.dll" ]] || { echo "missing $BUILD/gs_plugin.dll"; exit 1; }
  [[ -f "$BUILD/gs-scan.exe" ]] || { echo "missing $BUILD/gs-scan.exe"; exit 1; }
  rm -rf "$OUT/$NAME.vst3" "$OUT/$NAME"
  # VST3: the standard bundle layout.
  bin="$OUT/$NAME.vst3/Contents/x86_64-win"
  mkdir -p "$bin" "$OUT/$NAME.vst3/Contents/Resources"
  cp "$BUILD/gs_plugin.dll" "$bin/$NAME.vst3"
  cp "$BUILD/gs-scan.exe" "$bin/gs-scan.exe"
  cp -R dist "$OUT/$NAME.vst3/Contents/Resources/ui"
  echo "built $OUT/$NAME.vst3"
  # CLAP: one file; DAWs look through sub-folders of the CLAP folder, so it gets one.
  mkdir -p "$OUT/$NAME/Resources"
  cp "$BUILD/gs_plugin.dll" "$OUT/$NAME/$NAME.clap"
  cp "$BUILD/gs-scan.exe" "$OUT/$NAME/gs-scan.exe"
  cp -R dist "$OUT/$NAME/Resources/ui"
  echo "built $OUT/$NAME/$NAME.clap"
  exit 0
fi

[[ -f "$BUILD/libgs_plugin.dylib" ]] || { echo "missing $BUILD/libgs_plugin.dylib"; exit 1; }
[[ -f "$BUILD/gs-scan" ]] || { echo "missing $BUILD/gs-scan"; exit 1; }

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
