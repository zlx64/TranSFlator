import { getToken } from "./auth";

// Typed REST client for the TranSFlator backend.
// Same-origin in production (backend serves the built frontend); in dev the
// Vite proxy forwards /api to the backend (see vite.config.ts).

export class ApiError extends Error {
  status: number;
  code?: string;
  constructor(status: number, message: string, code?: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

async function request<T>(
  path: string,
  init: RequestInit = {},
  params?: Record<string, string | number | boolean | undefined>,
): Promise<T> {
  const url = new URL(path, window.location.origin);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined) url.searchParams.set(k, String(v));
    }
  }

  const headers = new Headers(init.headers);
  // FormData: the browser sets the multipart Content-Type (boundary).
  if (
    init.body &&
    !headers.has("Content-Type") &&
    !(init.body instanceof FormData)
  ) {
    headers.set("Content-Type", "application/json");
  }
  const token = getToken();
  if (token) headers.set("Authorization", `Bearer ${token}`);

  const res = await fetch(url.toString(), { ...init, headers });

  if (res.status === 401) {
    // Surface to the auth gate; the UI prompts for a token.
    window.dispatchEvent(new CustomEvent("transflator:unauthorized"));
  }

  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    let code: string | undefined;
    try {
      const data = await res.json();
      if (data && typeof data === "object") {
        if (typeof data.error === "string") message = data.error;
        if (typeof data.code === "string") code = data.code;
      }
    } catch {
      // non-JSON error body; keep default message
    }
    throw new ApiError(res.status, message, code);
  }

  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export async function downloadLog(name: string): Promise<void> {
  const token = getToken();
  const res = await fetch(`/api/health/logs/${encodeURIComponent(name)}`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  });
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const data = await res.json();
      if (
        data &&
        typeof data === "object" &&
        typeof (data as Record<string, unknown>).error === "string"
      ) {
        message = (data as { error: string }).error;
      }
    } catch {
      // non-JSON error body; keep the default message
    }
    throw new ApiError(res.status, message);
  }
  const blob = await res.blob();
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

export async function downloadJob(id: string, filename: string): Promise<void> {
  const token = getToken();
  const res = await fetch(`/api/jobs/${id}/download`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  });
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const data = await res.json();
      if (
        data &&
        typeof data === "object" &&
        typeof (data as Record<string, unknown>).error === "string"
      ) {
        message = (data as { error: string }).error;
      }
    } catch {
      // non-JSON error body; keep the default message
    }
    throw new ApiError(res.status, message);
  }
  const blob = await res.blob();
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

export const api = {
  get: <T>(
    path: string,
    params?: Record<string, string | number | boolean | undefined>,
  ) => request<T>(path, { method: "GET" }, params),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, {
      method: "POST",
      body: body === undefined ? undefined : JSON.stringify(body),
    }),
  put: <T>(path: string, body?: unknown) =>
    request<T>(path, {
      method: "PUT",
      body: body === undefined ? undefined : JSON.stringify(body),
    }),
  del: <T>(path: string) => request<T>(path, { method: "DELETE" }),

  // ---- Jobs (Phase 4) ----
  createJob: (body: CreateJobBody) =>
    request<{ job: Job }>("/api/jobs", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  listJobs: (limit = 50) =>
    request<{ jobs: Job[] }>("/api/jobs", { method: "GET" }, { limit }),
  getJob: (id: string) => request<{ job: Job }>(`/api/jobs/${id}`),
  cancelJob: (id: string) =>
    request<{ ok: boolean }>(`/api/jobs/${id}/cancel`, { method: "POST" }),
  deleteJob: (id: string) =>
    request<{ ok: boolean }>(`/api/jobs/${id}`, { method: "DELETE" }),
  deleteJobs: (filter: "all" | "failed" | "done" | "interrupted") =>
    request<{ ok: boolean; deleted: number }>("/api/jobs", { method: "DELETE" }, { filter }),
  retryJob: (id: string, mode: string) =>
    request<{ job: Job }>(`/api/jobs/${id}/retry`, {
      method: "POST",
      body: JSON.stringify({ mode }),
    }),
  downloadJob,
  /** §6.1: translate an uploaded external .srt/.ass/.vtt instead of an
   *  extracted stream. `path` is the video the output should sit next to. */
  uploadSubtitleJob: (file: File, body: UploadJobBody) => {
    const form = new FormData();
    form.append("file", file);
    form.append("path", body.path);
    form.append("root", String(body.root ?? 0));
    form.append("provider", body.provider);
    if (body.target_language) form.append("target_language", body.target_language);
    if (body.model) form.append("model", body.model);
    if (body.movie_name) form.append("movie_name", body.movie_name);
    if (body.description) form.append("description", body.description);
    if (body.start_now) form.append("start_now", "true");
    return request<{ job: Job }>("/api/jobs/upload", {
      method: "POST",
      body: form,
    });
  },

  // ---- Settings (Phase 6) ----
  getSettings: () =>
    request<SettingsResponse>("/api/settings", { method: "GET" }),
  putSettings: (body: UpdateSettingsBody) =>
    request<SettingsResponse>("/api/settings", {
      method: "PUT",
      body: JSON.stringify(body),
    }),

  // ---- Plex PIN authentication / discovery ----
  plexConnect: () =>
    request<PlexConnectStart>("/api/settings/plex/connect", {
      method: "POST",
    }),
  plexPoll: (pinId: number) =>
    request<PlexPollResponse>(`/api/settings/plex/connect/${pinId}`, {
      method: "GET",
    }),
  plexServers: () =>
    request<PlexServersResponse>("/api/settings/plex/servers", {
      method: "GET",
    }),
  testPlex: (body?: { server_url?: string }) =>
    request<TestConnectionResponse>("/api/settings/plex/test", {
      method: "POST",
      body: JSON.stringify(body ?? {}),
    }),
  removePlex: () =>
    request<{ ok: boolean }>("/api/settings/plex", { method: "DELETE" }),
  testJellyfin: (body?: { server_url?: string; token?: string }) =>
    request<TestConnectionResponse>("/api/settings/jellyfin/test", {
      method: "POST",
      body: JSON.stringify(body ?? {}),
    }),
  removeJellyfin: () =>
    request<{ ok: boolean }>("/api/settings/jellyfin", { method: "DELETE" }),

  // ---- Model discovery (FR-11 extension) ----
  /** List a provider's models for the dropdown (empty if not listable). */
  listModels: (provider: string) =>
    request<ModelsResponse>("/api/models", { method: "GET" }, { provider }),

  // ---- Health / diagnostics ----
  listLogs: () => request<LogsResponse>("/api/health/logs", { method: "GET" }),
  downloadLog,
};

/** Translation providers (mirrors the backend `Provider::all()` order). */
export const PROVIDERS: { id: string; label: string; needsKey: boolean }[] = [
  { id: "openai", label: "OpenAI", needsKey: true },
  { id: "gemini", label: "Google Gemini", needsKey: true },
  { id: "claude", label: "Anthropic Claude", needsKey: true },
  { id: "deepseek", label: "DeepSeek", needsKey: true },
  { id: "mistral", label: "Mistral", needsKey: true },
  { id: "openrouter", label: "OpenRouter", needsKey: true },
  { id: "custom", label: "Custom (OpenAI-compatible)", needsKey: false },
];

/**
 * Target languages for the dropdown. Natural-language names — the string is
 * passed verbatim to llm-subtrans (`-l`) and into the output filename
 * (`{lang}`), so keep them filename-friendly.
 */
export const LANGUAGES: string[] = [
  "Afrikaans",
  "Amharic",
  "Arabic",
  "Armenian",
  "Azerbaijani",
  "Bengali",
  "Bulgarian",
  "Burmese",
  "Catalan",
  "Croatian",
  "Czech",
  "Danish",
  "Dutch",
  "English",
  "Estonian",
  "Finnish",
  "French",
  "Georgian",
  "German",
  "Greek",
  "Hebrew",
  "Hindi",
  "Hungarian",
  "Icelandic",
  "Indonesian",
  "Italian",
  "Japanese",
  "Kazakh",
  "Khmer",
  "Korean",
  "Lao",
  "Latvian",
  "Lithuanian",
  "Malay",
  "Maltese",
  "Marathi",
  "Mongolian",
  "Nepali",
  "Norwegian",
  "Persian",
  "Polish",
  "Portuguese",
  "Punjabi",
  "Romanian",
  "Russian",
  "Serbian",
  "Simplified Chinese",
  "Slovak",
  "Slovenian",
  "Spanish",
  "Swahili",
  "Swedish",
  "Tagalog",
  "Tamil",
  "Telugu",
  "Thai",
  "Traditional Chinese",
  "Turkish",
  "Ukrainian",
  "Urdu",
  "Uzbek",
  "Vietnamese",
  "Welsh",
];

// ---- Settings (Phase 6) ----

export interface ProviderSetting {
  id: string;
  label: string;
  needs_key: boolean;
  key_set: boolean;
  key_masked: string | null;
}

export interface SettingsResponse {
  providers: ProviderSetting[];
  /** Last-selected provider/model/language — pre-fill the translate form. */
  default_provider: string;
  default_model: string;
  default_target_language: string;
  output_pattern: string;
  overwrite_behavior: "overwrite" | "suffix" | "skip";
  concurrency: number;
  auth_token_set: boolean;
  auth_token_masked: string | null;
  custom_server_url: string;
  custom_endpoint: string;
  custom_model: string;
  custom_models_url: string;
  custom_chat: boolean;
  plex_server_url: string;
  plex_token_set: boolean;
  plex_token_masked: string | null;
  jellyfin_server_url: string;
  jellyfin_token_set: boolean;
  jellyfin_token_masked: string | null;
}

export interface UpdateSettingsBody {
  /** provider id → new key; empty string clears, omitted providers are kept. */
  api_keys?: Record<string, string>;
  /** Last-selected provider; empty string clears, omitted keeps it. */
  default_provider?: string;
  /** Last-selected model; empty string clears, omitted keeps it. */
  default_model?: string;
  default_target_language?: string;
  output_pattern?: string;
  overwrite_behavior?: string;
  concurrency?: number;
  auth_token?: string;
  custom_server_url?: string;
  custom_endpoint?: string;
  custom_model?: string;
  custom_models_url?: string;
  custom_chat?: boolean;
  plex_server_url?: string;
  plex_token?: string;
  jellyfin_server_url?: string;
  jellyfin_token?: string;
}

// ---- Plex PIN authentication / discovery ----

export interface PlexConnectStart {
  pin_id: number;
  code: string;
  auth_url: string;
}

export interface PlexPollResponse {
  authorized: boolean;
  token_set: boolean;
}

export interface PlexServerOption {
  name: string;
  url: string;
  host: string;
  port: number;
  secure: boolean;
  local: boolean;
  hints: string[];
}

export interface PlexServersResponse {
  servers: PlexServerOption[];
}

export type TestConnectionField = "server_url" | "token" | "general";

export interface TestConnectionResponse {
  ok: boolean;
  message: string;
  field: TestConnectionField;
}

// ---- Model discovery (FR-11 extension) ----

export interface ModelsResponse {
  provider: string;
  /** False when the provider has no public listing endpoint (free-text only). */
  supports_list: boolean;
  /** Available model ids; empty when unsupported or when listing failed. */
  models: string[];
  /** Set when listing failed (e.g. missing key, HTTP error). */
  error?: string | null;
}

// ---- Shared domain types (kept in sync with backend serde models) ----

export interface HealthResponse {
  service: string;
  status: "ok";
  time: string;
}

export interface LogFile {
  name: string;
  size: number;
  modified_at: string | null;
}

export interface LogsResponse {
  logs: LogFile[];
}

export interface RootInfo {
  index: number;
  name: string;
  path: string;
}

export interface LibraryEntry {
  name: string;
  /** Path relative to the root, using `/` separators. */
  rel_path: string;
  is_dir: boolean;
  size?: number;
  is_video: boolean;
  /** Cached ffprobe metadata, present only when the file has been probed. */
  duration_s?: number | null;
  container?: string | null;
  audio_count?: number | null;
  subtitle_count?: number | null;
  /** Unique, cacheable thumbnail URL for video files. */
  thumbnail_url?: string | null;
}

export interface LibraryTreeResponse {
  root: number;
  path: string;
  folders: LibraryEntry[];
  files: LibraryEntry[];
}

export interface SearchResponse {
  results: LibraryEntry[];
}

export type StreamKind = "video" | "audio" | "subtitle" | "unknown";
export type SubtitleKind = "text" | "image";

export interface Stream {
  index: number;
  kind: StreamKind;
  codec: string;
  language?: string;
  title?: string;
  default: boolean;
  forced: boolean;
  subtitle_kind?: SubtitleKind;
}

/**
 * FR-7 auto-select hint (serde externally-tagged enum: unit variants
 * serialize as plain strings, `Auto` as `{ auto: { stream_index } }`).
 */
export type Selection =
  | { auto: { stream_index: number } }
  | "picker"
  | "image_only"
  | "none";

export interface MediaInfo {
  path: string;
  duration_s: number | null;
  container: string | null;
  format_name?: string;
  size?: number;
  streams: Stream[];
  video_count: number;
  audio_count: number;
  subtitle_count: number;
}

export interface StreamsResponse {
  info: MediaInfo;
  selection: Selection;
  /** Picker pre-highlight (default track, else first text track). */
  preferred: number | null;
  /** Unique, cacheable thumbnail URL for the current video file. */
  thumbnail_url?: string | null;
}

export type JobStatus =
  | "queued"
  | "running"
  | "done"
  | "failed"
  | "canceled"
  | "interrupted";

/** Mirrors the backend `jobs` table (spec §9). */
export interface Job {
  id: string;
  source_path: string;
  subtitle_stream_index: number | null;
  source_language: string | null;
  target_language: string;
  provider: string;
  model: string | null;
  movie_name?: string | null;
  description?: string | null;
  status: JobStatus;
  progress_pct: number;
  output_path: string | null;
  project_file_path: string | null;
  /** Set when the job translates an uploaded external subtitle (§6.1). */
  external_subtitle_path?: string | null;
  /** Retained tail of the job log. */
  log_tail: string;
  error_message: string | null;
  /** How the (next) run behaves: `rerun` | `retranslate` | `reparse`. */
  retry_mode: string;
  created_at: string;
  updated_at: string;
}

export interface CreateJobBody {
  root?: number;
  /** Path relative to the root, `/` separators. */
  path: string;
  /** Omit to auto-select (single text track). */
  stream_index?: number;
  target_language?: string;
  provider: string;
  model?: string;
  /** Optional show-name context passed to llm-subtrans (`--moviename`). */
  movie_name?: string;
  /** Optional description context passed to llm-subtrans (`--description`). */
  description?: string;
  /** Jump ahead of normal queued jobs (§13). */
  start_now?: boolean;
}

export interface UploadJobBody {
  root?: number;
  /** The video file the subtitle belongs to (output lands next to it). */
  path: string;
  provider: string;
  target_language?: string;
  model?: string;
  movie_name?: string;
  description?: string;
  start_now?: boolean;
}

/** Mirrors the backend `JobEvent` (tagged `type`, snake_case). */
export type JobEvent =
  | { type: "status"; status: string }
  | { type: "progress"; pct: number }
  | { type: "log"; line: string }
  | { type: "done"; output_path: string }
  | { type: "failed"; message: string };

export interface JobEventEnvelope {
  job_id: string;
  event: JobEvent;
}
