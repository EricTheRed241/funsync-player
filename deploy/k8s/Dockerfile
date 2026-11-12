# funsync-runner: the image the CronJobs in deploy/k8s run (issue #34).
# Build with `npm run k8s:build` (context = repo root, .dockerignore keeps it
# to code). Node + the repo's scripts for the JS CLIs, the Rust CLIs built for
# linux, ffmpeg (age-screen sheets, ffprobe for the scouts), unar / bsdtar
# (packed scenes), Chromium (VRBangers downloads) and omp (age-screen judge).

FROM rust:1-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# content-preferences.json is embedded with include_str!.
COPY renderer/js/content-preferences.json renderer/js/content-preferences.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p pornstar-tags -p age-screen -p video-tags \
 && mkdir -p /out \
 && cp target/release/pornstar-tags target/release/age-screen target/release/video-tags /out/

FROM node:26-bookworm-slim
ARG OMP_URL=https://build.stencil.so/d/omp/20261106-051327-75e7801/omp-linux-arm64
ARG GIT_SHA=dev
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl ffmpeg unar libarchive-tools chromium fonts-noto-cjk tini \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL "$OMP_URL" -o /usr/local/bin/omp && chmod +x /usr/local/bin/omp
WORKDIR /app
COPY package.json package-lock.json ./
# Dev dependencies included: megajs (MEGA downloads) is one. No lifecycle
# scripts, so Electron's binary is never downloaded.
RUN npm ci --ignore-scripts --no-audit --no-fund && npm cache clean --force
COPY . .
COPY --from=rust /out/ /usr/local/bin/
COPY deploy/k8s/run-job.sh /usr/local/bin/run-job
RUN chmod +x /usr/local/bin/run-job
ENV NODE_ENV=production \
    FUNSYNC_IMAGE_SHA=$GIT_SHA \
    FUNSYNC_CHROME_PATH=/usr/bin/chromium \
    FUNSYNC_CHROME_PROFILE=vrbangers-chromium-linux \
    FUNSYNC_CHROME_NO_SANDBOX=1 \
    FUNSYNC_CONFIG_READONLY=1
ENTRYPOINT ["/usr/bin/tini", "--", "run-job"]
