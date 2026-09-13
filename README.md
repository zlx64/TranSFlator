# TranSFlator

A self-hosted web app for translating embedded video subtitles. Browse a media
library, pick a video, and TranSFlator extracts its subtitle track with
`ffmpeg`, auto-detects and normalizes the encoding, then translates it with
[llm-subtrans](https://github.com/machinewrapped/llm-subtrans) into your target
language using any of several LLM providers. The translated subtitle is written
back next to the source file and is downloadable from the UI.

Built for a single household or small group of self-hosters running this next
to a Jellyfin/Plex/Emby-style folder layout, who want foreign-language shows
translated to their preferred language without doing it by hand in a GUI tool.

## Features

- **Library browsing** — drill into one or more read-only media roots; folders
  and video files only, with per-file stream badges and filename search.
- **Smart stream picking** — `ffprobe` classifies each subtitle track
  (text vs. image-based, language, title, default/forced). Exactly one
  text-based track is auto-selected; multiple tracks show a disambiguated
  picker; image-only tracks are labeled "OCR required"; zero tracks offer an
  external `.srt`/`.ass`/`.vtt` upload.
- **Lossless extraction** — `ffmpeg -map 0:s:<n> -c:s srt` (subtitle stream
  only; video/audio are never re-encoded), with timeout enforcement and
  cleanup of partial output on failure.
- **Encoding normalization** — non-UTF-8 subtitles (CP1251 Cyrillic, Shift-JIS
  anime fan subs, …) are auto-detected and transcoded to UTF-8 before
  translation.
- **Live progress** — batch progress and raw logs stream to the UI over
  WebSockets; jobs can be canceled (the child process is actually killed) or
  retried (re-run / re-translate / re-parse).
- **Robust job lifecycle** — SQLite-persisted jobs with bounded concurrency,
  fail-fast on missing API keys, bounded auto-retry on transient (429/timeout)
  provider errors, and graceful shutdown that drains in-flight jobs and marks
  stragglers resumable. Interrupted jobs (e.g. after a container restart) are
  detected on boot and offer one-click resume.
- **Configurable output** — naming pattern, overwrite/suffix/skip behavior, and
  default target language, all settable in the UI.
- **Safe to expose on a LAN** — optional shared access token gates the whole
  API/UI; API keys are encrypted at rest and never logged or shown in plaintext.

## How it works

```
Browser (React SPA) ──HTTP/WS──► Rust backend (axum)
                                    │  ├─ ffprobe / ffmpeg (subprocess, timeouts)
                                    │  ├─ SQLite (sqlx): jobs, settings, media cache
                                    │  └─ bounded-concurrency job queue
                                    └─► llm-subtrans (Python, subprocess) ──► provider API
Mounted media volume (read-only)  +  data volume (read-write: DB + output)
```

Pipeline per job: **probe** (`ffprobe`) → **classify/select** the subtitle
stream → **extract** (`ffmpeg`) → **normalize** to UTF-8 → **translate**
(`llm-subtrans`, with progress parsed from its log) → **write** output next to
the source. Every step is streamed to the UI and persisted to SQLite.

## Quick start (Docker)

Prerequisites: Docker + Docker Compose, and a folder of videos.

```bash
# 1. Put your media library somewhere (or create ./media).
# 2. Configure.
cp .env.example .env
#    edit .env: set APP_SECRET (openssl rand -hex 32) and any provider keys
#    you want to pre-seed, and point MEDIA_ROOT_HOST at your library.
# 3. Build and run.
docker compose -f docker/docker-compose.yml up -d --build
# 4. Open http://localhost:8080
```

The image is a multi-stage build (Rust release → Vite frontend → Python
runtime with `ffmpeg` and a **pinned** `llm-subtrans` tag). It runs as an
unprivileged user and exposes a `/api/healthz` healthcheck.

> **Provider keys** can be pre-seeded via the environment (see below) or set
> later in the **Settings** UI, where they are stored encrypted in the DB.

## Configuration

All configuration is via environment variables with sane defaults; the
overridable ones (provider keys, concurrency, default target language, output
pattern, overwrite behavior, auth token) are also editable in the Settings UI.
See `.env.example` for the full annotated list. The key ones:

| Variable | Default | Description |
|---|---|---|
| `APP_SECRET` | *(dev fallback)* | Secret used to encrypt stored API keys (AES-256-GCM, key = SHA-256 of this). Set a strong value in production. |
| `AUTH_TOKEN` | *(empty = open)* | When set, all `/api/*` and WS requests require `Authorization: Bearer <token>` (WS via `?token=`). |
| `PORT` | `8080` | Host port the app is published on. |
| `LOG_LEVEL` | `info` | `error` \| `warn` \| `info` \| `debug` \| `trace`. |
| `MEDIA_ROOT_HOST` | `./media` | Host path mounted read-only at `/media`. |
| `CONCURRENCY` | `1` | Max concurrent translation jobs. |
| `OUTPUT_PATTERN` | `{name}.{lang}.srt` | Output filename pattern. Placeholders: `{name}` `{lang}` `{ext}`. |
| `OVERWRITE_BEHAVIOR` | `suffix` | `overwrite` \| `suffix` (auto `-2`, `-3`, …) \| `skip`. |
| `DEFAULT_TARGET_LANGUAGE` | `English` | Default target language for new jobs. |
| `LLM_SUBTRANS_VERSION` | `1.6.1` | Pinned llm-subtrans release tag (build arg). |
| `FFPROBE_TIMEOUT_SECS` | `30` | `ffprobe` subprocess timeout. |
| `FFMPEG_TIMEOUT_SECS` | `300` | `ffmpeg` extraction timeout. |
| `OPENROUTER_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` / `ANTHROPIC_API_KEY` / `DEEPSEEK_API_KEY` / `MISTRAL_API_KEY` | *(empty)* | Optional pre-seed of provider keys (normally set in Settings). |

## Volume mounts

Two volumes, defined in `docker/docker-compose.yml`:

| Mount | Container path | Mode | Purpose |
|---|---|---|---|
| `${MEDIA_ROOT_HOST:-./media}` | `/media` | **read-only** | Your media library. The app never writes here. |
| `transflator-data` (named volume) | `/data` | **read-write** | The SQLite DB (`/data/transflator.db`), extracted/uploaded subtitle working files, and the `uploads/` dir for externally-uploaded subtitles. |

Translated output is written **next to the source video** inside the media
mount, so for the output to be persisted you should mount a directory the
container can write to (e.g. a read-write bind mount, or a library share that
permits writes). The read-only default is the safe choice; if your library is
truly read-only, the output write will fail with a clear permission message
(§6.9) rather than a crash.

## Authentication

By default the app is open (for local/dev use). To gate it — recommended if you
expose it on a LAN, since it has filesystem read access and holds paid-API
credentials — set `AUTH_TOKEN`. Every `/api/*` request and WebSocket connection
then requires the token (WS via the `?token=` query param). The frontend prompts
for it on a `401` and keeps it in memory/localStorage.

## Supported providers

Mirrors llm-subtrans's own provider list 1:1 (FR-11):

| Provider | ID | API key env var | Notes |
|---|---|---|---|
| OpenAI | `openai` | `OPENAI_API_KEY` | supports a custom base URL |
| Google Gemini | `gemini` | `GEMINI_API_KEY` | |
| Anthropic Claude | `claude` | `ANTHROPIC_API_KEY` | |
| DeepSeek | `deepseek` | `DEEPSEEK_API_KEY` | supports a custom base URL |
| Mistral | `mistral` | `MISTRAL_API_KEY` | supports a custom server URL |
| OpenRouter | `openrouter` | `OPENROUTER_API_KEY` | `--auto` or a specific model |
| Custom (OpenAI-compatible) | `custom` | *(via `-k`, or none for local)* | point at any OpenAI-compatible server (e.g. a local model) |

Set keys in the **Settings** UI (encrypted at rest) or pre-seed them via the
environment. `custom` needs no key for a local server.

## Adding a new translation provider

Providers map 1:1 to llm-subtrans scripts, so adding one is a small, contained
change in the `translate` crate:

1. **Add the variant** to `Provider` in
   `backend/crates/translate/src/provider.rs` and implement each method:
   - `script_name()` — the llm-subtrans script under `scripts/` (e.g.
     `myprov-subtrans.py`, or reuse `llm-subtrans.py` for an OpenAI-compatible
     server).
   - `env_key()` — the env var llm-subtrans reads for the key, or `None` if the
     key is passed via `-k` / not needed.
   - `requires_api_key()` — `false` for keyless/local providers.
   - `as_str()` / `label()` / `parse()` — the stable id, the UI label, and the
     accepted input strings.
   - Add it to `all()` (this drives the Settings + new-job provider dropdowns,
     so the UI updates automatically).
2. **Map the flags** in `build_command()` (
   `backend/crates/translate/src/command.rs`). This is a pure, exhaustively
   unit-tested function (D1/D2) that turns `TranslateOptions` into the exact
   script + args + env. Add your provider's provider-specific flags here and,
   if it has a key, inject it as an **environment variable** (never on the
   command line — D2).
3. **Pre-seed the key** (optional): add `MYPROV_API_KEY` to `.env.example` and
   to the `environment:` block of `docker/docker-compose.yml`.
4. **Add a unit test** in `command.rs` asserting the exact command line for your
   provider (this is the highest-value test in the project — a wrong flag
   silently mistranslates a run).

That's it — the queue, persistence, progress parsing, and UI are all
provider-agnostic.

## Architecture

Cargo workspace under `backend/` (D4):

| Crate | Responsibility |
|---|---|
| `config` | Env/settings loading, secret encryption (AES-256-GCM, key = SHA-256 of `APP_SECRET`), boot validation (§6.12), SQLite pool init + migrations. |
| `media` | `ffprobe`/`ffmpeg` subprocess wrappers (timeouts, JSON parsing), stream classification, path-traversal guard (§6.9), media cache, encoding detect/normalize (§6.5), disk-space checks (§6.10). |
| `translate` | `build_command()` flag mapper (D1/D2), the llm-subtrans process wrapper (spawn/stream/kill-tree), progress parser (D3), and failure classification. |
| `jobs` | Job state machine, SQLite persistence (sqlx), bounded-concurrency queue, cancellation (kills the child), retry, and graceful-shutdown drain (NFR-6). |
| `api` | The `transflator` binary: axum REST + WebSocket, auth middleware (D7), serves the built frontend, `/healthz`. |

Frontend (D6): Vite + React 18 + TypeScript + TailwindCSS v4 + Radix UI
primitives + TanStack Query + Zustand + react-router, with a native WebSocket
hook for live job events. Dark mode by default.

### API surface

REST (all under `/api`, plus an open `/api/healthz`):

- `GET  /library/roots`, `GET /library/tree?path=…`, `GET /library/search?q=…`
- `GET  /media/streams?path=…` — cached/forced `ffprobe` stream info
- `POST /jobs` — create a job `{root, path, stream_index, target_language, provider, model}`
- `POST /jobs/upload` — create a job from an uploaded external subtitle (§6.1)
- `GET  /jobs`, `GET /jobs/:id` — list / inspect
- `POST /jobs/:id/cancel`, `POST /jobs/:id/retry` (`{mode: rerun|retranslate|reparse}`)
- `GET  /settings`, `PUT /settings` — providers, keys (write-only), concurrency, output pattern, auth token
- `WS   /jobs/:id/events` — status, progress, log lines, terminal state

Data model (SQLite, D5): `media_cache(path, size, mtime, …)`, `jobs(…)`,
`settings(key, value, is_secret)`.

## Design decisions (D1–D7)

- **D1 — llm-subtrans integration (Option A).** llm-subtrans is bundled into the
  same image at a pinned tag and invoked as a subprocess:
  `python <LLM_SUBTRANS_HOME>/scripts/<provider>-subtrans.py [flags] <input.srt>`,
  with `PYTHONPATH` and cwd set to the home so its `scripts.*` imports resolve.
  (Option B, a sidecar container, was rejected for v1 to keep operations simple.)
- **D2 — API keys never on the command line.** `build_command()` returns
  `(script, args, env)`; for known providers the key is injected as an
  environment variable (matching llm-subtrans's own `.env` keys) instead of `-k`,
  so it never appears in `ps` / process listings. `custom` uses `-k` (no stable
  env name).
- **D3 — Progress parsing.** llm-subtrans logs lines like
  `INFO: Translated batch 2.3: 90/240 lines (37%)`. A regex
  (`Translated batch \S+: (\d+)/(\d+) lines( \((\d+)%\))?`) extracts
  `processed/total` for `progress_pct`; raw lines are also forwarded to the log
  tail and the WS stream.
- **D4 — Backend layout.** The five-crate Cargo workspace above.
- **D5 — Persistence.** sqlx + SQLite with runtime queries (no compile-time
  macros); tables per the data model; migrations run at boot.
- **D6 — Frontend.** Vite + React 18 + TS + Tailwind v4 + Radix + TanStack Query
  + Zustand + react-router; native WebSocket; dark mode default.
- **D7 — Auth.** Optional shared access token (NFR-7). When `AUTH_TOKEN` is set,
  all `/api/*` and WS requests require it; when unset, open (dev).

## Development

```bash
# Backend (Rust workspace)
cd backend
cargo build            # debug
cargo test --workspace # 110 tests
cargo run -p transflator-api   # needs the env vars; see .env.example

# Frontend (Vite)
cd frontend
npm install
npm run dev            # dev server (proxies /api to the backend)
npm run build          # production build -> frontend/dist
npm test               # vitest component tests
```

The backend reads the same environment variables as the container (see
`.env.example`). For a quick local run without Docker, point `MEDIA_ROOTS`,
`DATA_DIR`, `DB_PATH`, `LLM_SUBTRANS_HOME`, `PYTHON_BIN`, `FFMPEG_BIN`, and
`FFPROBE_BIN` at local paths.

## Project layout

```
TranSFlator/
├── backend/                  # Rust workspace
│   └── crates/
│       ├── api/              # axum binary: REST + WS, serves frontend, /healthz
│       ├── media/            # ffprobe/ffmpeg wrappers, classification, guard, encoding
│       ├── translate/        # llm-subtrans command mapper + process wrapper + progress
│       ├── jobs/             # queue, state machine, persistence, drain
│       └── config/           # settings, secrets, boot validation, DB
├── frontend/                 # React + Vite + TS + Tailwind
│   └── src/ (pages, components, api client, ws hook, store)
├── docker/
│   ├── Dockerfile            # multi-stage: rust -> node -> python runtime
│   └── docker-compose.yml
├── .env.example
└── README.md
```
