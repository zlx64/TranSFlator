# syntax=docker/dockerfile:1

# =====================================================================
#  Stage 1 — Rust backend (release)
# =====================================================================
# sqlx's `sqlite` feature bundles libsqlite3 by default, so a C compiler is
# required to compile SQLite into the binary. The result is self-contained
# (no libsqlite3 needed at runtime).
FROM rust:1.97-bookworm AS rust
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY backend/ ./
RUN cargo build --release --bin transflator

# =====================================================================
#  Stage 2 — Frontend (static build)
# =====================================================================
FROM node:22-bookworm AS frontend
WORKDIR /app
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# =====================================================================
#  Stage 3 — llm-subtrans checkout (pinned tag, D1)
# =====================================================================
# Cloned in its own stage so the final image doesn't need git installed.
FROM python:3.12-slim-bookworm AS llm
ARG LLM_SUBTRANS_VERSION=1.6.1
ENV LLM_SUBTRANS_HOME=/opt/llm-subtrans
RUN apt-get update \
    && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN git clone --depth 1 --branch v${LLM_SUBTRANS_VERSION} \
        https://github.com/machinewrapped/llm-subtrans.git ${LLM_SUBTRANS_HOME}

# =====================================================================
#  Stage 4 — Runtime
# =====================================================================
FROM python:3.12-slim-bookworm AS runtime

# ffmpeg/ffprobe (extraction) and curl (healthcheck). git is not needed at
# runtime — the llm-subtrans checkout is copied in from the `llm` stage.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ffmpeg curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# llm-subtrans (pinned) + provider deps installed as extras: openai (covers
# openrouter/custom/deepseek), gemini, claude, mistral.
ENV LLM_SUBTRANS_HOME=/opt/llm-subtrans
COPY --from=llm ${LLM_SUBTRANS_HOME} ${LLM_SUBTRANS_HOME}
RUN pip install --no-cache-dir \
        "${LLM_SUBTRANS_HOME}[openai,gemini,claude,mistral]"

# Application.
ENV APP_HOME=/opt/transflator
WORKDIR ${APP_HOME}
COPY --from=rust /app/target/release/transflator ./transflator
COPY --from=frontend /app/dist ./frontend/dist

ENV WEB_ROOT=${APP_HOME}/frontend/dist \
    PYTHON_BIN=python3 \
    FFMPEG_BIN=ffmpeg \
    FFPROBE_BIN=ffprobe \
    DATA_DIR=/data \
    DB_PATH=/data/transflator.db

# Run as an unprivileged user; /data holds the DB + translated output.
RUN useradd --create-home --uid 10001 appuser \
    && mkdir -p /data \
    && chown -R appuser:appuser /data ${APP_HOME}
USER appuser
VOLUME ["/data"]

EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS http://localhost:8080/api/healthz || exit 1

CMD ["./transflator"]
