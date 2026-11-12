#!/bin/bash
# The omp auth broker the age-screen CronJob's judge signs in through (issue #34):
# `npm run omp:broker -- install | uninstall | status`.
#
# The judge is `omp -p …` in a pod. The pod must not open the Mac's ~/.omp
# (its SQLite stores aren't safe across Docker's file sharing, and a second
# OAuth refresher could sign the Mac out), so the Mac runs
# `omp auth-broker serve`: it keeps the credentials in ~/.omp/agent/agent.db,
# refreshes OAuth tokens itself, and hands pods short-lived access tokens over
# HTTP (bearer `~/.omp/auth-broker.token`, the funsync-omp Secret). The pod's
# omp is a broker client (OMP_AUTH_BROKER_URL / OMP_AUTH_BROKER_TOKEN), so the
# screener uses your normal sign-in (Claude subscription) like on the Mac.
#
# Bound to 127.0.0.1 only: pods reach it as host.docker.internal:8765 (Docker
# Desktop forwards to the Mac's loopback); the LAN can't.
# LaunchAgent com.funsync.omp-broker: starts at login, restarted if it exits.
set -euo pipefail

LABEL=com.funsync.omp-broker
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
OMP="${OMP_BIN:-/opt/homebrew/bin/omp}"
BIND="${OMP_BROKER_BIND:-127.0.0.1:8765}"
LOG="$HOME/Library/Logs/funsync-player/omp-broker.log"
DOMAIN="gui/$(id -u)"

install() {
  [ -x "$OMP" ] || { echo "no omp at $OMP (set OMP_BIN)" >&2; exit 1; }
  mkdir -p "$(dirname "$PLIST")" "$(dirname "$LOG")"
  cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$OMP</string>
    <string>auth-broker</string>
    <string>serve</string>
    <string>--bind=$BIND</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>30</integer>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
</dict>
</plist>
EOF
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  launchctl bootstrap "$DOMAIN" "$PLIST"
  echo "installed $PLIST (omp auth-broker on $BIND); refresh the Secret with: npm run k8s -- secrets"
}

uninstall() {
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "removed $LABEL"
}

status() {
  if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
    launchctl print "$DOMAIN/$LABEL" | grep -E '^\s+(state|pid|last exit code|runs) ='
  else
    echo "$LABEL not loaded"
  fi
  curl -s -m 3 "http://$BIND/v1/healthz" || echo "broker not answering on $BIND"
  echo
}

case "${1:-}" in
  install) install ;;
  uninstall) uninstall ;;
  status) status ;;
  *) echo "usage: npm run omp:broker -- install | uninstall | status" >&2; exit 64 ;;
esac
