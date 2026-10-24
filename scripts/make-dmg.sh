#!/usr/bin/env bash
# Packs build/ASMR X3000.app into a DMG whose window is the installer: the background from
# tools/dmg/make_background.py, the app on the left, Applications on the right.
#   scripts/make-dmg.sh OUT.dmg "Volume name"
# Finder lays the window out, so it runs on a Mac with a logged-in session (macOS may ask once
# to let the terminal control Finder).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/build/ASMR X3000.app"
OUT="${1:?usage: make-dmg.sh OUT.dmg \"Volume name\"}"
VOL="${2:-ASMR X3000}"
# Window and icon centres in points: the same as tools/dmg/make_background.py.
W=640 H=420 ICON=112 APP_X=170 APP_Y=178 APPS_X=470 APPS_Y=178

[[ -d "$APP" ]] || { echo "! build the app first: scripts/build-macos.sh"; exit 1; }
WORK="$(mktemp -d)"
trap 'hdiutil detach -quiet "/Volumes/$VOL" 2>/dev/null || true; rm -rf "$WORK"' EXIT

echo "▸ background"
python3 "$ROOT/tools/dmg/make_background.py" "$WORK/bg" >/dev/null
SRC="$WORK/src"
mkdir -p "$SRC/.background"
tiffutil -cathidpicheck "$WORK/bg/background.png" "$WORK/bg/background@2x.png" -out "$SRC/.background/background.tiff" 2>/dev/null
cp -R "$APP" "$SRC/"
ln -s /Applications "$SRC/Applications"

echo "▸ layout"
hdiutil detach -quiet "/Volumes/$VOL" 2>/dev/null || true
SIZE=$(( $(du -sm "$SRC" | cut -f1) + 40 ))
hdiutil create -quiet -volname "$VOL" -srcfolder "$SRC" -fs HFS+ -format UDRW -size "${SIZE}m" "$WORK/rw.dmg"
hdiutil attach -quiet -noautoopen "$WORK/rw.dmg"
osascript <<EOF
tell application "Finder"
  tell disk "$VOL"
    open
    set w to container window
    set current view of w to icon view
    set toolbar visible of w to false
    set statusbar visible of w to false
    -- The bounds include the title bar.
    set the bounds of w to {200, 120, 200 + $W, 120 + $H + 28}
    set o to the icon view options of w
    set arrangement of o to not arranged
    set icon size of o to $ICON
    set text size of o to 13
    set background picture of o to file ".background:background.tiff"
    set position of item "ASMR X3000.app" of w to {$APP_X, $APP_Y}
    set position of item "Applications" of w to {$APPS_X, $APPS_Y}
    update without registering applications
    delay 1
    close
  end tell
end tell
EOF
sync
hdiutil detach -quiet "/Volumes/$VOL"

echo "▸ compress"
rm -f "$OUT"
hdiutil convert -quiet "$WORK/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$OUT"
echo "✓ $OUT"
