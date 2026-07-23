/// Formats the engine can reason about. This list matches the "descoped"
/// v1 surface from the architecture review: RAR is decode-only (no legal
/// open-source RAR *encoder* exists — see workspace README), HEIC is
/// decode-only pending a patent-licensing decision, and EXE/MSI/DMG/ISO are
/// deliberately absent as first-class formats since they're just opaque
/// blobs routed through the archive codec, not a distinct domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    Png,
    Jpeg,
    WebP,
    Avif,
    Gzip,
    Zip,
    SevenZip,
    Zstd,
    Bzip2,
    Tar,
    RarDecodeOnly,
    Unknown,
}

/// Coarse domain a [`Format`] belongs to, used to route to the right
/// `sizer-codecs-*` crate and to pick an auto-selection strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Image,
    Video,
    Audio,
    Document,
    Archive,
    Unknown,
}

/// Identifies a byte stream's format from its magic bytes. Entropy
/// estimation (is this already compressed / encrypted, so compressing
/// again is wasted work?) is a separate, heavier pass layered on top in
/// `sizer-codecs-archive`, not part of core detection.
pub trait Detector: Send + Sync {
    /// `header` should be the first ~64 bytes of the stream — enough for
    /// every magic-number check below without requiring a seekable source.
    fn sniff(&self, header: &[u8]) -> Format;
}

#[derive(Debug, Default)]
pub struct MagicByteDetector;

impl Detector for MagicByteDetector {
    fn sniff(&self, header: &[u8]) -> Format {
        const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF];
        const GZIP: &[u8] = &[0x1F, 0x8B];
        const ZIP: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
        const SEVEN_Z: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
        const ZSTD: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD];
        const BZIP2: &[u8] = &[0x42, 0x5A, 0x68];
        const RAR: &[u8] = &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07];

        if header.starts_with(PNG) {
            Format::Png
        } else if header.starts_with(JPEG) {
            Format::Jpeg
        } else if header.starts_with(GZIP) {
            Format::Gzip
        } else if header.starts_with(ZIP) {
            Format::Zip
        } else if header.starts_with(SEVEN_Z) {
            Format::SevenZip
        } else if header.starts_with(ZSTD) {
            Format::Zstd
        } else if header.starts_with(BZIP2) {
            Format::Bzip2
        } else if header.starts_with(RAR) {
            Format::RarDecodeOnly
        } else if header.len() >= 12 && &header[8..12] == b"WEBP" {
            Format::WebP
        } else if header.len() >= 12 && &header[4..8] == b"ftyp" && &header[8..12] == b"avif" {
            Format::Avif
        } else {
            Format::Unknown
        }
    }
}

impl Format {
    pub fn kind(self) -> FileKind {
        match self {
            Format::Png | Format::Jpeg | Format::WebP | Format::Avif => FileKind::Image,
            Format::Gzip
            | Format::Zip
            | Format::SevenZip
            | Format::Zstd
            | Format::Bzip2
            | Format::Tar
            | Format::RarDecodeOnly => FileKind::Archive,
            Format::Unknown => FileKind::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_png() {
        let d = MagicByteDetector;
        assert_eq!(
            d.sniff(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]),
            Format::Png
        );
    }

    #[test]
    fn sniffs_zstd() {
        let d = MagicByteDetector;
        assert_eq!(d.sniff(&[0x28, 0xB5, 0x2F, 0xFD, 0, 0]), Format::Zstd);
    }

    #[test]
    fn rar_is_decode_only_but_still_detected() {
        let d = MagicByteDetector;
        assert_eq!(
            d.sniff(&[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0, 0]),
            Format::RarDecodeOnly
        );
    }

    #[test]
    fn unknown_bytes_are_unknown() {
        let d = MagicByteDetector;
        assert_eq!(d.sniff(&[0, 1, 2, 3]), Format::Unknown);
    }

    #[test]
    fn short_header_does_not_panic() {
        let d = MagicByteDetector;
        assert_eq!(d.sniff(&[]), Format::Unknown);
        assert_eq!(d.sniff(&[0x89]), Format::Unknown);
    }
}
