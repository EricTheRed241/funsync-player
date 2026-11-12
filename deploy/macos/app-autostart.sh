#!/bin/bash
# Start the FunSync desktop app when you log in, and again if it crashes
# (issue #34): `npm run app:autostart -- install | uninstall | status`.
#
# A LaunchAgent (~/Library/LaunchAgents/com.funsync.app.plist) runs the repo's
# Electron binary with the repo as the app. Launched this way the process is
# Electron.app itself, so macOS asks about Electron.app's own permissions
# (Documents for the repo, removable volumes for the library, local network
# for the devices) rather than those of a shell or of launchd.
#   - RunAtLoad: starts at login (macOS can't start a windowed app before one).
#   - KeepAlive SuccessfulExit=false: a crash (non-zero exit or a signal) is
#     restarted after ThrottleInterval; quitting the app normally (exit 0)
#     leaves it closed until the next login or `launchctl kickstart`.
# The app restarts its own backend when /health stops answering
# (electron/python-bridge.js), so nothing else watches it.
set -euo pipefail

LABEL=com.funsync.app
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
ELECTRON="$REPO/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"
LOG_DIR="$HOME/Library/Logs/funsync-player"
DOMAIN="gui/$(id -u)"

install() {
  [ -x "$ELECTRON" ] || { echo "no Electron binary at $ELECTRON (npm ci first)" >&2; exit 1; }
  mkdir -p "$(dirname "$PLIST")" "$LOG_DIR"
  cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$ELECTRON</string>
    <string>$REPO</string>
  </array>
  <key>WorkingDirectory</key><string>$REPO</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key><false/>
  </dict>
  <key>ThrottleInterval</key><integer>30</integer>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>$LOG_DIR/app-launchd.log</string>
  <key>StandardErrorPath</key><string>$LOG_DIR/app-launchd.log</string>
</dict>
</plist>
EOF
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  launchctl bootstrap "$DOMAIN" "$PLIST"
  echo "installed $PLIST; the app starts now if it isn't running, and at every login"
}

uninstall() {
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "removed $LABEL (a running app keeps running)"
}

status() {
  if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
    launchctl print "$DOMAIN/$LABEL" | grep -E '^\s+(state|pid|last exit code|runs) ='
  else
    echo "$LABEL not loaded"
  fi
}

case "${1:-}" in
  install) install ;;
  uninstall) uninstall ;;
  status) status ;;
  *) echo "usage: npm run app:autostart -- install | uninstall | status" >&2; exit 64 ;;
esac
