//! Subtitle text encoding detection and normalization (FR-9, §6.5).
//!
//! Older SRT files (Cyrillic/CP1251, Shift-JIS anime fan releases, etc.) are not
//! always UTF-8. We auto-detect the charset and transcode to UTF-8 before handing
//! the file to llm-subtrans, which expects clean UTF-8.

use crate::model::MediaError;
use chardetng::EncodingDetector;
use std::path::Path;

/// Result of normalizing a byte buffer to UTF-8.
#[derive(Debug, Clone)]
pub struct NormalizedText {
    /// The UTF-8 bytes.
    pub bytes: Vec<u8>,
    /// True when the input was already valid UTF-8 (no transcode performed).
    pub was_utf8: bool,
    /// Detected source charset label (e.g. `windows-1251`, `shift_jis`), or
    /// `None` when the input was already UTF-8.
    pub detected_encoding: Option<String>,
}

/// Normalize an arbitrary byte buffer to UTF-8.
///
/// 1. If already valid UTF-8, return as-is.
/// 2. Otherwise detect the charset with [`chardetng`] and transcode via
///    [`encoding_rs`], falling back to Windows-1252 if detection fails.
pub fn normalize_to_utf8(data: &[u8]) -> NormalizedText {
    if std::str::from_utf8(data).is_ok() {
        return NormalizedText {
            bytes: data.to_vec(),
            was_utf8: true,
            detected_encoding: None,
        };
    }

    let mut detector = EncodingDetector::new();
    // We already know the data is not valid UTF-8, so don't allow a UTF-8 guess.
    detector.feed(data, true);
    let enc = detector.guess(None, false);
    let detected = Some(enc.name().to_string());

    let (decoded, _reenc, _had_errors) = enc.decode(data);
    NormalizedText {
        bytes: decoded.into_owned().into_bytes(),
        was_utf8: false,
        detected_encoding: detected,
    }
}

/// Read a file, normalize it to UTF-8, and write it back in place **only if** it
/// was not already UTF-8. Returns the normalization report.
pub fn normalize_file_in_place(path: &Path) -> Result<NormalizedText, MediaError> {
    let data = std::fs::read(path)?;
    let report = normalize_to_utf8(&data);
    if !report.was_utf8 {
        std::fs::write(path, &report.bytes)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_utf8_untouched() {
        let s = "hello — 日本語 — привет";
        let out = normalize_to_utf8(s.as_bytes());
        assert!(out.was_utf8);
        assert_eq!(out.detected_encoding, None);
        assert_eq!(String::from_utf8(out.bytes).unwrap(), s);
    }

    #[test]
    fn cp1251_cyrillic_transcoded() {
        let original = "Привет, мир! Это субтитры.";
        let (encoded, _enc, _lossy) = encoding_rs::WINDOWS_1251.encode(original);
        let bytes = encoded.into_owned();
        // Sanity: the encoded bytes are NOT valid UTF-8.
        assert!(std::str::from_utf8(&bytes).is_err());

        let out = normalize_to_utf8(&bytes);
        assert!(!out.was_utf8);
        let decoded = String::from_utf8(out.bytes).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn shift_jis_transcoded() {
        let original = "こんにちは、世界。これはテストです。";
        let (encoded, _enc, _lossy) = encoding_rs::SHIFT_JIS.encode(original);
        let bytes = encoded.into_owned();
        assert!(std::str::from_utf8(&bytes).is_err());

        let out = normalize_to_utf8(&bytes);
        assert!(!out.was_utf8);
        let decoded = String::from_utf8(out.bytes).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub.srt");
        let original = "1\n00:00:01,000 --> 00:00:02,000\nЗдравствуйте\n";
        let (encoded, _enc, _lossy) = encoding_rs::WINDOWS_1251.encode(original);
        std::fs::write(&path, encoded.as_ref()).unwrap();

        let report = normalize_file_in_place(&path).unwrap();
        assert!(!report.was_utf8);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, original);

        // Second pass: now it's UTF-8, so no change.
        let report2 = normalize_file_in_place(&path).unwrap();
        assert!(report2.was_utf8);
    }
}
