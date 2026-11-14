#!/bin/sh
# The eroscripts-scout CronJob's command (run by run-job): the EroScripts daily
# run (scan, backfill, downloads of the posts swiped right, SLR-hosted ones
# included), then the SLR scenes swiped right on the swipe deck's SLR tab
# (`slr-scout --fetch`). One after the other, so the two never spend the SLR
# download quota at the same time. Exit: the first code that isn't 0 or 2
# (an error, 3 = a login expired), else 2 when either left work, else 0.
set -u

node scripts/eroscripts-scout.mjs --daily --passes 2 --wait 1h
eroscripts=$?
echo "[scout-daily] eroscripts-scout exit $eroscripts"

slr-scout --fetch
slr=$?
echo "[scout-daily] slr-scout --fetch exit $slr"

for code in "$eroscripts" "$slr"; do
  case $code in 0|2) ;; *) exit "$code" ;; esac
done
[ "$eroscripts" = 2 ] || [ "$slr" = 2 ] && exit 2
exit 0
