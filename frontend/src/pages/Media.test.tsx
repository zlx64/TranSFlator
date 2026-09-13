import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import Media from "./Media";
import { api, type Stream, type StreamsResponse } from "@/lib/api";

vi.mock("@/lib/api", () => ({
  api: {
    get: vi.fn(),
    uploadSubtitleJob: vi.fn(),
    createJob: vi.fn().mockResolvedValue({ job: { id: "x" } }),
    putSettings: vi.fn().mockResolvedValue({}),
    listModels: vi.fn().mockResolvedValue({
      provider: "openai",
      supports_list: true,
      models: ["gpt-4o", "gpt-4o-mini"],
      error: null,
    }),
    getSettings: vi.fn().mockResolvedValue({
      providers: [],
      default_provider: "",
      default_model: "",
      default_target_language: "",
      output_pattern: "{name}.{lang}.srt",
      overwrite_behavior: "suffix",
      concurrency: 2,
      auth_token_set: false,
      auth_token_masked: null,
    }),
  },
  PROVIDERS: [
    { id: "openai", label: "OpenAI", needsKey: true },
    { id: "custom", label: "Custom (OpenAI-compatible)", needsKey: false },
  ],
  LANGUAGES: ["English", "German", "Japanese", "French"],
}));

const video: Stream = {
  index: 0,
  kind: "video",
  codec: "h264",
  default: false,
  forced: false,
};
const audio: Stream = {
  index: 1,
  kind: "audio",
  codec: "aac",
  language: "eng",
  default: false,
  forced: false,
};
const subJpn: Stream = {
  index: 2,
  kind: "subtitle",
  codec: "subrip",
  language: "jpn",
  title: "Main",
  default: true,
  forced: false,
  subtitle_kind: "text",
};
const subPgs: Stream = {
  index: 3,
  kind: "subtitle",
  codec: "hdmv_pgs_subtitle",
  language: "eng",
  default: false,
  forced: true,
  subtitle_kind: "image",
};
const subVtt: Stream = {
  index: 1,
  kind: "subtitle",
  codec: "webvtt",
  language: "eng",
  default: false,
  forced: false,
  subtitle_kind: "text",
};

function response(
  selection: StreamsResponse["selection"],
  preferred: number | null,
  streams: Stream[],
): StreamsResponse {
  const subs = streams.filter((s) => s.kind === "subtitle").length;
  return {
    info: {
      path: "/media/Show/Season 1/E01.mkv",
      duration_s: 3600.5,
      container: "matroska",
      size: 123456,
      streams,
      video_count: 1,
      audio_count: streams.length - 1 - subs,
      subtitle_count: subs,
    },
    selection,
    preferred,
  };
}

function renderMedia() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter
        initialEntries={[
          "/media?root=0&path=Show%2FSeason%201%2FE01.mkv",
        ]}
      >
        <Media />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("Media page — FR-7 subtitle selection branching", () => {
  it("auto-selects a single text track and enables Translate", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response({ auto: { stream_index: 1 } }, 1, [video, subVtt]),
    );
    renderMedia();

    expect(
      await screen.findByText("Subtitle track auto-selected"),
    ).toBeInTheDocument();
    expect(screen.getByText("#1 — eng (text)")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /translate/i })).toBeEnabled();
  });

  it("shows a picker for multiple tracks, pre-selecting the preferred one", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response("picker", 2, [video, audio, subJpn, subPgs]),
    );
    renderMedia();

    expect(
      await screen.findByText(
        "Multiple subtitle tracks found — choose one to translate:",
      ),
    ).toBeInTheDocument();
    // §6.7 per-track labels.
    expect(
      screen.getByText("#2 — jpn, Main (default, text)"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("#3 — eng (forced, image — OCR required)"),
    ).toBeInTheDocument();

    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(2);
    // Preferred (default) track is pre-selected → Translate enabled.
    expect(radios[0]).toBeChecked();
    expect(screen.getByRole("button", { name: /translate/i })).toBeEnabled();

    // Switching to the image track disables Translate.
    await userEvent.click(screen.getByText("#3 — eng (forced, image — OCR required)"));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /translate/i })).toBeDisabled(),
    );
  });

  it("shows the OCR notice and disables Translate for image-only files", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response("image_only", 2, [video, subPgs]),
    );
    renderMedia();

    expect(
      await screen.findByText("Image-based subtitles only"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /translate/i })).toBeDisabled();
  });

  it("shows the no-subtitles notice and disables Translate", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response("none", null, [video, audio]),
    );
    renderMedia();

    expect(
      await screen.findByText("No subtitle tracks found"),
    ).toBeInTheDocument();
    const translate = screen
      .getAllByRole("button", { name: /translate/i })
      .find((b) => b.textContent === "Translate");
    expect(translate).toBeDisabled();
  });

  it("offers external subtitle upload when there are no tracks (§6.1)", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response("none", null, [video, audio]),
    );
    renderMedia();

    expect(
      await screen.findByText("No subtitle tracks found"),
    ).toBeInTheDocument();
    const uploadBtn = screen.getByRole("button", {
      name: /upload & start now/i,
    });
    expect(uploadBtn).toBeDisabled();

    (
      api.uploadSubtitleJob as ReturnType<typeof vi.fn>
    ).mockResolvedValue({ job: { id: "x" } });
    const fileInput = screen.getByLabelText("Subtitle file");
    await userEvent.upload(
      fileInput,
      new File(["1\n00:00:00,000 --> 00:00:02,000\nHi"], "ext.srt", {
        type: "text/plain",
      }),
    );
    await waitFor(() => expect(uploadBtn).toBeEnabled());
    await userEvent.click(uploadBtn);
    await waitFor(() =>
      expect(api.uploadSubtitleJob).toHaveBeenCalledWith(
        expect.any(File),
        expect.objectContaining({
          path: "Show/Season 1/E01.mkv",
          provider: "openai",
        }),
      ),
    );
  });

  it("shows a model dropdown when the provider lists models", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response({ auto: { stream_index: 1 } }, 1, [video, subVtt]),
    );
    renderMedia();

    await screen.findByText("Subtitle track auto-selected");
    await userEvent.click(screen.getByRole("button", { name: /translate/i }));

    const select = await screen.findByRole("combobox", { name: /model/i });
    expect(select).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /auto \(provider default\)/i }),
    ).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "gpt-4o" })).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "gpt-4o-mini" }),
    ).toBeInTheDocument();
  });

  it("falls back to a free-text model input when the provider does not list models", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response({ auto: { stream_index: 1 } }, 1, [video, subVtt]),
    );
    (api.listModels as ReturnType<typeof vi.fn>).mockResolvedValue({
      provider: "openai",
      supports_list: false,
      models: [],
      error: null,
    });
    renderMedia();

    await screen.findByText("Subtitle track auto-selected");
    await userEvent.click(screen.getByRole("button", { name: /translate/i }));

    expect(await screen.findByPlaceholderText(/gpt-4o-mini/i)).toBeInTheDocument();
  });

  it("shows a language dropdown and pre-fills the remembered language", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response({ auto: { stream_index: 1 } }, 1, [video, subVtt]),
    );
    (api.getSettings as ReturnType<typeof vi.fn>).mockResolvedValue({
      providers: [],
      default_provider: "",
      default_model: "",
      default_target_language: "German",
      output_pattern: "{name}.{lang}.srt",
      overwrite_behavior: "suffix",
      concurrency: 2,
      auth_token_set: false,
      auth_token_masked: null,
    });
    renderMedia();

    await screen.findByText("Subtitle track auto-selected");
    await userEvent.click(screen.getByRole("button", { name: /translate/i }));

    const select = await screen.findByRole("combobox", {
      name: /target language/i,
    });
    await waitFor(() => expect(select).toHaveValue("German"));
    expect(screen.getByRole("option", { name: "Japanese" })).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /auto \(default\)/i }),
    ).toBeInTheDocument();
  });

  it("sends optional context fields and the start-now flag (§13)", async () => {
    (api.get as ReturnType<typeof vi.fn>).mockResolvedValue(
      response({ auto: { stream_index: 1 } }, 1, [video, subVtt]),
    );
    renderMedia();

    await screen.findByText("Subtitle track auto-selected");
    await userEvent.click(
      screen.getByRole("button", { name: "Translate" }),
    );

    await userEvent.type(
      await screen.findByLabelText("Show name (optional)"),
      "Test Show",
    );
    await userEvent.type(
      screen.getByLabelText("Additional information (optional)"),
      "A test description",
    );
    await userEvent.click(
      screen.getByRole("checkbox", { name: /start now/i }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: /add to queue/i }),
    );

    await waitFor(() =>
      expect(api.createJob).toHaveBeenCalledWith(
        expect.objectContaining({
          path: "Show/Season 1/E01.mkv",
          provider: "openai",
          movie_name: "Test Show",
          description: "A test description",
          start_now: false,
        }),
      ),
    );
  });
});
