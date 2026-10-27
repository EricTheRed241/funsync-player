# AGENTS.md — FunSync Player (fork)

Local desktop video player that plays funscripts on stroker and vibration devices (Handy, Buttplug/Intiface, Autoblow, TCode serial/UDP), with a web remote and a VR content server for HereSphere/DeoVR. `README.md` is the feature documentation: every user-visible feature has a section there, so update it with each change.

## Repository and remotes
- `fork` = `EricTheRed241/funsync-player` (ours, public). `origin` = `DaveMakesWaves/funsync-player` (upstream). **Never push to `origin`.**
- Work on a branch from `fork/main`, preferably in a worktree: `git worktree add ../funsync-<topic> -b <type>/<topic> fork/main`, then symlink `node_modules` from the main checkout and reuse `backend/.venv`.
- Open the PR against `fork` `main` (`gh pr create --repo EricTheRed241/funsync-player`), merge it with a merge commit, then fast-forward local `main` and remove the worktree.
- Branch prefixes in use: `feat/`, `fix/`, `docs/`. Commit subject: `<area>: <what>` (e.g. `eroscripts:scout: …`, `preview-check: …`).
- `notes/` (planning docs, `notes/CLAUDE.md` test-environment notes) and `.claude/` are gitignored and may not exist in a checkout.

## Architecture
| Part | Where | Notes |
|---|---|---|
| Electron main | `electron/main.js` | Owns `config.json` (`electron/store.js`, electron-conf), IPC, window, starts the backend; single-instance lock |
| Library scan | `electron/library-scan.js` | Pairs videos with funscripts, variants, axis companions; no Electron dependency, shared with CLIs |
| EroScripts client | `electron/eroscripts-api.js` | Discourse API, attachments, pixeldrain listing, downloads (forum host only) |
| Renderer | `renderer/` (`js/`, `components/`, `styles/`, `locales/`) | ESM inside a CommonJS package; UI strings go through i18n (`data-i18n*`, `t()`), with every locale file updated |
| Backend | `backend/` (FastAPI, Python 3.13, `backend/.venv`) | Port **5123**; `routes/` (media, remote, vr_api, …) and `services/` (vr_sections, library_edits, funscript, …) |
| Web remote | `backend/web-remote/` | PWA served at `/remote/` |
| CLIs | `scripts/*.mjs` | `eroscripts:sync`, `eroscripts:scout`, `faptap:scout`, `dedupe:scenes`, `upgrade:axes`, `prune:generated`, `eroscripts:preview-check`; the two scouts share `scripts/discovery-io.mjs` (config lock, library walk, Discovery playlist) and run `dedupe:scenes --apply` after downloads (`scripts/scene-dedupe-lib.mjs`: same video → one copy kept) |

Invariants:
- **The desktop app owns the library.** The backend keeps view-only copies registered by the desktop (`POST /api/media/register`). Edits from the phone or headset go through `services/library_edits` and are applied by the desktop. CLIs write `config.json` only while the app is closed (check `SingletonLock` in userData), or they write side files the app merges (`eroscripts-links.json`).
- Backend networking: binds `0.0.0.0` by default; `--local-only` / `FUNSYNC_LOCAL_ONLY=1` binds `127.0.0.1`.
- Axis files are `<stem>.<axis>.funscript` (`AXIS_SUFFIXES` in `library-scan.js`). Generated companions are `<stem>.generated.<axis>.funscript`. **Hand-made axes always win**: generators skip folders with hand-made axes.
- Paths stored in config must use the filesystem's own spelling (Unicode NFC/NFD), because playlists and the headset match scan paths byte for byte.
- Skip AppleDouble sidecars (`._*`, see `isAppleDoubleSidecar`) in any directory walk.

## CLI pattern
- Logic goes in `scripts/<name>-lib.mjs` (pure, injectable, tested). The CLI in `scripts/<name>.mjs` does IO only, parses flags, prints usage on `--help`, and uses exit codes 0 / 1 / 2 (rate limited or work left) / 3 (not logged in).
- CLIs that import `renderer/*.js` re-exec once with `--disable-warning=MODULE_TYPELESS_PACKAGE_JSON` (copy the block from an existing script).
- Forum access is read-only and paced (`createPacer`, ≥ 700 ms). Silence electron-log in CLIs (`log.transports.*.level = false`).

## Tests
- JS: `npx vitest run` (`tests/unit/**/*.test.js`, jsdom by default). Node-only tests start with the `@vitest-environment node` header block used in existing tests. Run one file with `npx vitest run tests/unit/<file>.test.js`.
- Python: `cd backend && .venv/bin/python -m pytest` (`asyncio_mode = auto`).
- Tests cover behaviour (filters, pairing, precedence, boundaries), not wiring. Never hit the network: fake the forum and hosts, and use temp dirs for scenes.
- Smoke-test CLIs against a copied userData (`--user-data <tmp>`) and a temp folder, never the real `config.json`.

## Environment pitfalls
- Don't run `npm install --ignore-scripts` (or `--no-save` installs) in the main checkout: it leaves `node_modules/electron` without its binary. To repair, extract the cached `electron-v<ver>-<platform>.zip` into `node_modules/electron/dist` and write `node_modules/electron/path.txt` (on macOS: `Electron.app/Contents/MacOS/Electron`).
- `npm start` runs `electron .`. The backend is spawned by the app; `npm run backend:dev` runs it alone.
