#!/bin/sh
# Entry point of every FunSync CronJob (deploy/k8s): `run-job <command> [args…]`.
#
# - Fails fast (exit 1) when a path in FUNSYNC_REQUIRE_PATHS (colon-separated)
#   is missing, e.g. the library drive is unplugged, instead of letting a CLI
#   treat an empty mount as an empty library.
# - Exit 2 is the CLIs' "rate limited or work left" (AGENTS.md CLI pattern):
#   the run did its part and the next scheduled run continues, so the Job
#   counts as succeeded. 1 (error) and 3 (not logged in to the forum) fail
#   the Job, which `kubectl get jobs` then shows.
set -u

log() { echo "[run-job $(date -u +%FT%TZ)] $*"; }

if [ "$#" -eq 0 ]; then
  log "usage: run-job <command> [args...]"
  exit 64
fi

old_ifs=$IFS
IFS=:
for p in ${FUNSYNC_REQUIRE_PATHS:-}; do
  if [ ! -d "$p" ]; then
    log "missing $p (drive unplugged or not shared with Docker); not running $1"
    exit 1
  fi
done
IFS=$old_ifs

log "image ${FUNSYNC_IMAGE_SHA:-unknown}, host $(hostname): $*"
"$@"
code=$?
case $code in
  0) log "done" ;;
  2) log "exit 2 (rate limited or work left): the next run continues"; code=0 ;;
  3) log "exit 3: not logged in to EroScripts; log in from the app" ;;
  *) log "failed with exit $code" ;;
esac
exit $code
