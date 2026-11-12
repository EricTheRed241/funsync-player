#!/bin/bash
# FunSync jobs in the local cluster (issue #34): `npm run k8s -- <command>`.
#
#   build                 build funsync-runner:<git sha> and tag it :current (the CronJobs use :current)
#   apply                 build, then apply deploy/k8s (namespace, funsync-app Service, CronJobs) and the Secrets
#   secrets               (re)create the Secrets from local files (never stored in the repo):
#                         funsync-vrbangers from vb_hack.txt, funsync-omp from the omp auth
#                         broker's token (npm run omp:broker -- install) and your model roles
#   run <job>             start a CronJob now (a one-off Job) and follow its log
#   pause [job|all]       suspend a CronJob (all: every CronJob)
#   resume [job|all]      resume it (faptap-scout stays paused unless named)
#   status                CronJobs, recent Jobs and the funsync-app Service
#   logs <job>            log of that CronJob's latest Job
#
# Docker Desktop's Kubernetes (kubeadm mode) runs pods on the Docker engine,
# so a local image needs no registry: the CronJobs use imagePullPolicy Never.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
NS=funsync
CONTEXT=docker-desktop
K=(kubectl --context "$CONTEXT" -n "$NS")
JOBS=(eroscripts-scout pornstar-tags age-screen eroscripts-sync faptap-scout)

die() { echo "k8s: $*" >&2; exit 1; }

build() {
  local sha
  sha=$(git -C "$REPO" rev-parse --short HEAD)
  if [ -n "$(git -C "$REPO" status --porcelain --untracked-files=no)" ]; then sha="$sha-dirty"; fi
  docker build -f "$REPO/deploy/k8s/Dockerfile" --build-arg GIT_SHA="$sha" \
    -t "funsync-runner:$sha" -t funsync-runner:current "$REPO"
  echo "built funsync-runner:$sha (tagged :current)"
}

secrets() {
  kubectl --context "$CONTEXT" get ns "$NS" >/dev/null 2>&1 || kubectl --context "$CONTEXT" create ns "$NS" >/dev/null
  local vrb="$REPO/vb_hack.txt"
  [ -f "$vrb" ] || vrb="$HOME/Documents/repo/funsync-player/vb_hack.txt"
  if [ -f "$vrb" ]; then
    "${K[@]}" create secret generic funsync-vrbangers --from-file=credentials="$vrb" \
      --dry-run=client -o yaml | "${K[@]}" apply -f - >/dev/null
    echo "secret funsync-vrbangers: from $vrb"
  else
    echo "secret funsync-vrbangers: no vb_hack.txt, skipped (VRBangers downloads will fail)"
  fi
  # The age-screen judge signs in through the Mac's omp auth broker
  # (deploy/macos/omp-broker.sh): its bearer token plus the Mac's model roles,
  # so the pod's omp picks the same model.
  local token_file="$HOME/.omp/auth-broker.token"
  if [ -s "$token_file" ]; then
    local roles tmp
    roles=$(omp config get modelRoles --json 2>/dev/null | node -e '
      let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => {
        const roles = JSON.parse(s || "{}").value || {};
        const lines = Object.entries(roles).map(([k, v]) => `  ${k}: ${JSON.stringify(v)}`);
        process.stdout.write(lines.length ? `modelRoles:\n${lines.join("\n")}\n` : "");
      });') || roles=""
    tmp=$(mktemp -d)
    printf '%s' "$(tr -d '\n' < "$token_file")" > "$tmp/OMP_AUTH_BROKER_TOKEN"
    printf '%s' "$roles" > "$tmp/config.yml"
    "${K[@]}" create secret generic funsync-omp --from-file="$tmp" \
      --dry-run=client -o yaml | "${K[@]}" apply -f - >/dev/null
    rm -rf "$tmp"
    echo "secret funsync-omp: omp auth broker token + model roles"
  else
    echo "secret funsync-omp: no $token_file (npm run omp:broker -- install), skipped: age-screen's judge can't sign in"
  fi
}

job_name() {
  local j="${1:-}"
  [ -n "$j" ] || die "name a job: ${JOBS[*]}"
  for k in "${JOBS[@]}"; do [ "$k" = "$j" ] && return 0; done
  die "unknown job '$j' (${JOBS[*]})"
}

suspend_jobs() {
  local value=$1 target=${2:-all}
  local list=()
  if [ "$target" = all ]; then
    list=("${JOBS[@]}")
    # Resuming "all" leaves the paused FapTap scout alone.
    [ "$value" = false ] && list=(eroscripts-scout pornstar-tags age-screen eroscripts-sync)
  else
    job_name "$target"; list=("$target")
  fi
  for j in "${list[@]}"; do
    "${K[@]}" patch cronjob "$j" -p "{\"spec\":{\"suspend\":$value}}" >/dev/null
    echo "$j: suspend=$value"
  done
}

cmd=${1:-}; shift || true
case "$cmd" in
  build) build ;;
  apply)
    build
    secrets
    kubectl --context "$CONTEXT" apply -k "$REPO/deploy/k8s"
    ;;
  secrets) secrets ;;
  run)
    job_name "${1:-}"
    name="$1-manual-$(date +%Y%m%d-%H%M%S)"
    "${K[@]}" create job "$name" --from="cronjob/$1" >/dev/null
    echo "job $name started"
    # Until the pod has left Pending (it may also fail at once, e.g. drive missing).
    phase=""
    for _ in $(seq 120); do
      phase=$("${K[@]}" get pods -l "job-name=$name" -o jsonpath='{.items[0].status.phase}' 2>/dev/null || true)
      [ -n "$phase" ] && [ "$phase" != Pending ] && break
      sleep 1
    done
    [ "$phase" = Pending ] && "${K[@]}" describe pods -l "job-name=$name" | grep -E 'Warning' | tail -n 3
    "${K[@]}" logs -f "job/$name" || true
    # The Job's own status lags the pod's exit by a moment.
    for _ in $(seq 30); do
      done_count=$("${K[@]}" get job "$name" -o jsonpath='{.status.succeeded}{.status.failed}' 2>/dev/null || true)
      [ -n "$done_count" ] && break
      sleep 1
    done
    "${K[@]}" get job "$name" -o jsonpath='{.metadata.name}: succeeded={.status.succeeded} failed={.status.failed}{"\n"}'
    ;;
  pause) suspend_jobs true "${1:-all}" ;;
  resume) suspend_jobs false "${1:-all}" ;;
  status)
    "${K[@]}" get cronjobs
    echo
    "${K[@]}" get jobs --sort-by=.metadata.creationTimestamp 2>/dev/null | tail -n 12
    echo
    "${K[@]}" get svc funsync-app
    ;;
  logs)
    job_name "${1:-}"
    latest=$("${K[@]}" get jobs -l "funsync/job=$1" --sort-by=.metadata.creationTimestamp -o name | tail -n 1)
    [ -n "$latest" ] || die "no jobs of $1 yet"
    "${K[@]}" logs "$latest"
    ;;
  ""|-h|--help|help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) die "unknown command '$cmd' (see npm run k8s -- help)" ;;
esac
