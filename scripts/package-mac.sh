#!/usr/bin/env bash
# Build the macOS installer: one .pkg that puts the app in /Applications and the VST3 /
# CLAP plugins in /Library/Audio/Plug-Ins (each can be unticked under "Customize").
#
# An installer rather than a disk image: files it installs aren't quarantined, so the
# app and plugins open without Gatekeeper prompts once the installer itself was allowed
# (it isn't signed by an Apple Developer ID; see README "Installing").
#
# Usage: scripts/package-mac.sh <Guitar Studio.app> <plugin dir with the .vst3/.clap> <out dir>
set -euo pipefail

APP="$1"
PLUGINS="$2"
OUT="$3"
NAME="Guitar Studio"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
WORK="target/pkg"

[[ -d "$APP" ]] || { echo "missing $APP"; exit 1; }
[[ -d "$PLUGINS/$NAME.vst3" && -d "$PLUGINS/$NAME.clap" ]] || { echo "missing plugins in $PLUGINS"; exit 1; }

rm -rf "$WORK"
mkdir -p "$WORK/app" "$WORK/vst3" "$WORK/clap" "$OUT"
cp -R "$APP" "$WORK/app/"
cp -R "$PLUGINS/$NAME.vst3" "$WORK/vst3/"
cp -R "$PLUGINS/$NAME.clap" "$WORK/clap/"

component() { # <root> <install location> <identifier> <pkg name>
  local plist="$WORK/$4.plist"
  pkgbuild --analyze --root "$1" "$plist"
  # Install exactly there: don't "upgrade" some other copy found elsewhere on the disk.
  plutil -replace 0.BundleIsRelocatable -bool NO "$plist"
  pkgbuild --root "$1" --component-plist "$plist" --install-location "$2" \
    --identifier "$3" --version "$VERSION" "$WORK/$4.pkg"
}

component "$WORK/app" /Applications dev.guitarstudio.app app
component "$WORK/vst3" /Library/Audio/Plug-Ins/VST3 dev.guitarstudio.plugin.vst3 vst3
component "$WORK/clap" /Library/Audio/Plug-Ins/CLAP dev.guitarstudio.plugin.clap clap

cat > "$WORK/distribution.xml" <<XML
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
  <title>$NAME $VERSION</title>
  <options customize="allow" require-scripts="false" hostArchitectures="arm64,x86_64"/>
  <domains enable_localSystem="true"/>
  <allowed-os-versions><os-version min="13.0"/></allowed-os-versions>
  <choices-outline>
    <line choice="app"/>
    <line choice="vst3"/>
    <line choice="clap"/>
  </choices-outline>
  <choice id="app" title="$NAME app" description="The desktop app, in Applications.">
    <pkg-ref id="dev.guitarstudio.app"/>
  </choice>
  <choice id="vst3" title="VST3 plugin" description="Guitar Studio inside your DAW (Ableton Live, Cubase, REAPER, Studio One…).">
    <pkg-ref id="dev.guitarstudio.plugin.vst3"/>
  </choice>
  <choice id="clap" title="CLAP plugin" description="Guitar Studio inside DAWs that load CLAP plugins (Bitwig, REAPER…).">
    <pkg-ref id="dev.guitarstudio.plugin.clap"/>
  </choice>
  <pkg-ref id="dev.guitarstudio.app" version="$VERSION">app.pkg</pkg-ref>
  <pkg-ref id="dev.guitarstudio.plugin.vst3" version="$VERSION">vst3.pkg</pkg-ref>
  <pkg-ref id="dev.guitarstudio.plugin.clap" version="$VERSION">clap.pkg</pkg-ref>
</installer-gui-script>
XML

PKG="$OUT/Guitar-Studio-$VERSION-macOS.pkg"
productbuild --distribution "$WORK/distribution.xml" --package-path "$WORK" "$PKG"
echo "built $PKG"
