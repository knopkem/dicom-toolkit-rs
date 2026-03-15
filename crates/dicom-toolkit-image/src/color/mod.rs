//! Color model types and conversions.
//!
//! Provides the [`PhotometricInterpretation`] enum and sub-modules for each
//! supported color model.

pub mod palette;
pub mod rgb;
pub mod ycbcr;

pub use palette::PaletteColorLut;

// ── PhotometricInterpretation ─────────────────────────────────────────────────

/// DICOM photometric interpretation (tag `(0028,0004)`).
///
/// Determines how pixel data is rendered and whether color conversions are
/// needed before display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotometricInterpretation {
    /// Black is maximum, white is minimum (inverted grayscale).
    Monochrome1,
    /// Black is minimum, white is maximum (normal grayscale).
    Monochrome2,
    /// Red, Green, Blue — pixel-interleaved or plane-interleaved.
    Rgb,
    /// YBR full-range, pixel-interleaved (3 bytes/pixel).
    YbrFull,
    /// YBR 4:2:2 chroma-subsampled (2 bytes/pixel on average).
    YbrFull422,
    /// Palette color — pixel values are indices into R/G/B LUT tables.
    PaletteColor,
    /// Any other string not recognised above.
    Unknown(String),
}

impl PhotometricInterpretation {
    /// Parse the DICOM string value of `(0028,0004)`.
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "MONOCHROME1" => Self::Monochrome1,
            "MONOCHROME2" => Self::Monochrome2,
            "RGB" => Self::Rgb,
            "YBR_FULL" => Self::YbrFull,
            "YBR_FULL_422" => Self::YbrFull422,
            "PALETTE COLOR" | "PALETTE_COLOR" => Self::PaletteColor,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// Returns `true` if this is a grayscale interpretation.
    pub fn is_grayscale(&self) -> bool {
        matches!(self, Self::Monochrome1 | Self::Monochrome2)
    }

    /// Returns `true` if this is a color interpretation.
    pub fn is_color(&self) -> bool {
        matches!(
            self,
            Self::Rgb | Self::YbrFull | Self::YbrFull422 | Self::PaletteColor
        )
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photometric_from_str() {
        assert_eq!(
            PhotometricInterpretation::parse("MONOCHROME1"),
            PhotometricInterpretation::Monochrome1
        );
        assert_eq!(
            PhotometricInterpretation::parse("MONOCHROME2"),
            PhotometricInterpretation::Monochrome2
        );
        assert_eq!(
            PhotometricInterpretation::parse("RGB"),
            PhotometricInterpretation::Rgb
        );
        assert_eq!(
            PhotometricInterpretation::parse("YBR_FULL"),
            PhotometricInterpretation::YbrFull
        );
        assert_eq!(
            PhotometricInterpretation::parse("YBR_FULL_422"),
            PhotometricInterpretation::YbrFull422
        );
        assert_eq!(
            PhotometricInterpretation::parse("PALETTE COLOR"),
            PhotometricInterpretation::PaletteColor
        );
        assert!(matches!(
            PhotometricInterpretation::parse("OTHER"),
            PhotometricInterpretation::Unknown(_)
        ));
    }

    #[test]
    fn photometric_grayscale_color() {
        assert!(PhotometricInterpretation::Monochrome1.is_grayscale());
        assert!(PhotometricInterpretation::Monochrome2.is_grayscale());
        assert!(!PhotometricInterpretation::Rgb.is_grayscale());
        assert!(PhotometricInterpretation::Rgb.is_color());
        assert!(!PhotometricInterpretation::Monochrome2.is_color());
    }
}
