//! Subtitle stream classification and the auto-select rule (FR-6, FR-7, §6.7).

use crate::model::{Selection, Stream, SubtitleKind};

/// Classify a subtitle codec as text-based or image-based (FR-6).
///
/// Returns `None` for codecs that are not subtitles.
pub fn classify_subtitle_codec(codec: &str) -> Option<SubtitleKind> {
    match codec.to_ascii_lowercase().as_str() {
        // Text-based: directly extractable / translatable.
        "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "vtt" | "mov_text" | "text"
        | "mpeg4" => Some(SubtitleKind::Text),
        // Image-based: pixel images, not directly translatable (§6.2).
        "hdmv_pgs_subtitle" | "pgs" | "dvd_subtitle" | "vobsub" | "dvb_subtitle"
        | "dvbsubtitle" | "xsub" => Some(SubtitleKind::Image),
        _ => None,
    }
}

/// Apply the auto-select rule (FR-7) to a file's streams.
///
/// - No subtitle streams            -> [`Selection::None`]
/// - Exactly one subtitle stream:
///     - text-based                 -> [`Selection::Auto`]
///     - image-based                -> [`Selection::ImageOnly`]
/// - More than one subtitle stream  -> [`Selection::Picker`] (any mix)
pub fn select_subtitle(streams: &[Stream]) -> Selection {
    let subs: Vec<&Stream> = streams
        .iter()
        .filter(|s| s.kind == crate::model::StreamKind::Subtitle)
        .collect();

    match subs.len() {
        0 => Selection::None,
        1 => match subs[0].subtitle_kind {
            Some(SubtitleKind::Text) => Selection::Auto {
                stream_index: subs[0].index,
            },
            _ => Selection::ImageOnly,
        },
        _ => Selection::Picker,
    }
}

/// Count video/audio/subtitle streams (used to rebuild cached info).
pub fn count_kinds(streams: &[Stream]) -> (usize, usize, usize) {
    let mut video = 0usize;
    let mut audio = 0usize;
    let mut subtitle = 0usize;
    for s in streams {
        match s.kind {
            crate::model::StreamKind::Video => video += 1,
            crate::model::StreamKind::Audio => audio += 1,
            crate::model::StreamKind::Subtitle => subtitle += 1,
            crate::model::StreamKind::Unknown => {}
        }
    }
    (video, audio, subtitle)
}

/// The stream the picker should pre-highlight: the one marked `default`,
/// else the first text-based one, else the first subtitle (FR-7).
pub fn preferred_subtitle_index(streams: &[Stream]) -> Option<usize> {
    let subs: Vec<&Stream> = streams
        .iter()
        .filter(|s| s.kind == crate::model::StreamKind::Subtitle)
        .collect();
    if subs.is_empty() {
        return None;
    }
    if let Some(d) = subs.iter().find(|s| s.default) {
        return Some(d.index);
    }
    if let Some(t) = subs.iter().find(|s| s.subtitle_kind == Some(SubtitleKind::Text)) {
        return Some(t.index);
    }
    Some(subs[0].index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StreamKind;

    fn sub(index: usize, codec: &str, default: bool, forced: bool) -> Stream {
        Stream {
            index,
            kind: StreamKind::Subtitle,
            codec: codec.to_string(),
            language: Some("eng".to_string()),
            title: None,
            default,
            forced,
            subtitle_kind: classify_subtitle_codec(codec),
        }
    }

    fn other(index: usize) -> Stream {
        Stream {
            index,
            kind: StreamKind::Audio,
            codec: "aac".into(),
            language: None,
            title: None,
            default: false,
            forced: false,
            subtitle_kind: None,
        }
    }

    #[test]
    fn codec_classification() {
        assert_eq!(classify_subtitle_codec("subrip"), Some(SubtitleKind::Text));
        assert_eq!(classify_subtitle_codec("ass"), Some(SubtitleKind::Text));
        assert_eq!(classify_subtitle_codec("webvtt"), Some(SubtitleKind::Text));
        assert_eq!(classify_subtitle_codec("mov_text"), Some(SubtitleKind::Text));
        assert_eq!(
            classify_subtitle_codec("hdmv_pgs_subtitle"),
            Some(SubtitleKind::Image)
        );
        assert_eq!(classify_subtitle_codec("dvd_subtitle"), Some(SubtitleKind::Image));
        assert_eq!(classify_subtitle_codec("dvb_subtitle"), Some(SubtitleKind::Image));
        assert_eq!(classify_subtitle_codec("h264"), None);
    }

    #[test]
    fn no_subtitles() {
        let streams = vec![other(0), other(1)];
        assert_eq!(select_subtitle(&streams), Selection::None);
    }

    #[test]
    fn single_text_auto() {
        let streams = vec![other(0), sub(1, "subrip", false, false)];
        assert_eq!(
            select_subtitle(&streams),
            Selection::Auto { stream_index: 1 }
        );
    }

    #[test]
    fn single_image_only() {
        let streams = vec![other(0), sub(1, "hdmv_pgs_subtitle", false, false)];
        assert_eq!(select_subtitle(&streams), Selection::ImageOnly);
    }

    #[test]
    fn multiple_any_mix_picker() {
        // One text + one image still means "multiple subtitle streams" -> picker.
        let streams = vec![
            other(0),
            sub(1, "subrip", false, false),
            sub(2, "hdmv_pgs_subtitle", false, false),
        ];
        assert_eq!(select_subtitle(&streams), Selection::Picker);

        // Two text tracks -> picker.
        let streams = vec![other(0), sub(1, "subrip", true, false), sub(2, "ass", false, true)];
        assert_eq!(select_subtitle(&streams), Selection::Picker);
    }

    #[test]
    fn preferred_index_default_first() {
        let streams = vec![
            other(0),
            sub(1, "subrip", false, false),
            sub(2, "ass", true, false),
        ];
        assert_eq!(preferred_subtitle_index(&streams), Some(2));
    }

    #[test]
    fn preferred_index_first_text_when_no_default() {
        let streams = vec![
            other(0),
            sub(1, "hdmv_pgs_subtitle", false, false),
            sub(2, "subrip", false, false),
        ];
        assert_eq!(preferred_subtitle_index(&streams), Some(2));
    }

    #[test]
    fn display_label_includes_flags() {
        let s = sub(2, "subrip", false, true);
        let label = s.display_label();
        assert!(label.contains("#2"), "label was: {label}");
        assert!(label.contains("forced"), "label was: {label}");
        assert!(label.contains("text"), "label was: {label}");
    }
}
