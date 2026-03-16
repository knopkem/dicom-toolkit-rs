//! DICOM value types — the `Value` enum and its supporting types.
//!
//! Ports DCMTK's per-VR element classes into a single Rust enum, with dedicated
//! structs for the richer DICOM scalar types (dates, times, person names).

use crate::dataset::DataSet;
use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_dict::Tag;
use std::fmt;

// ── DicomDate ──────────────────────────────────────────────────────────────────

/// A DICOM DA (Date) value: YYYYMMDD, with optional month and day.
///
/// Partial dates are represented by leaving `month` and/or `day` as `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DicomDate {
    pub year: u16,
    /// 0 when not specified (partial date contains year only).
    pub month: u8,
    /// 0 when not specified (partial date contains year+month only).
    pub day: u8,
}

impl DicomDate {
    /// Parse a DICOM DA string: YYYYMMDD, YYYYMM, or YYYY.
    pub fn parse(s: &str) -> DcmResult<Self> {
        let s = s.trim();
        match s.len() {
            4 => {
                let year = parse_u16_str(&s[0..4])?;
                Ok(Self {
                    year,
                    month: 0,
                    day: 0,
                })
            }
            6 => {
                let year = parse_u16_str(&s[0..4])?;
                let month = parse_u8_str(&s[4..6])?;
                Ok(Self {
                    year,
                    month,
                    day: 0,
                })
            }
            8 => {
                let year = parse_u16_str(&s[0..4])?;
                let month = parse_u8_str(&s[4..6])?;
                let day = parse_u8_str(&s[6..8])?;
                Ok(Self { year, month, day })
            }
            _ => Err(DcmError::Other(format!("invalid DICOM date: {:?}", s))),
        }
    }

    /// Parse a DICOM DA string, also accepting the legacy "YYYY.MM.DD" format.
    pub fn from_da_str(s: &str) -> DcmResult<Self> {
        let s = s.trim();
        if s.len() == 10 && s.as_bytes().get(4) == Some(&b'.') && s.as_bytes().get(7) == Some(&b'.')
        {
            let year = parse_u16_str(&s[0..4])?;
            let month = parse_u8_str(&s[5..7])?;
            let day = parse_u8_str(&s[8..10])?;
            return Ok(Self { year, month, day });
        }
        Self::parse(s)
    }
}

impl fmt::Display for DicomDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.month == 0 {
            write!(f, "{:04}", self.year)
        } else if self.day == 0 {
            write!(f, "{:04}{:02}", self.year, self.month)
        } else {
            write!(f, "{:04}{:02}{:02}", self.year, self.month, self.day)
        }
    }
}

// ── DicomTime ──────────────────────────────────────────────────────────────────

/// A DICOM TM (Time) value: HHMMSS.FFFFFF, with optional components.
///
/// Partial times are allowed: `HH`, `HHMM`, `HHMMSS`, `HHMMSS.F{1-6}`.
/// Missing components are stored as `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DicomTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// Fractional seconds in microseconds (0–999999).
    pub fraction: u32,
}

impl DicomTime {
    /// Parse a DICOM TM string.
    pub fn parse(s: &str) -> DcmResult<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(DcmError::Other("empty DICOM time string".into()));
        }

        // Find the fractional part if present
        let (time_part, fraction) = if let Some(dot_pos) = s.find('.') {
            let frac_str = &s[dot_pos + 1..];
            // Pad or truncate to 6 digits
            let mut padded = String::from(frac_str);
            while padded.len() < 6 {
                padded.push('0');
            }
            let frac = parse_u32_str(&padded[..6])?;
            (&s[..dot_pos], frac)
        } else {
            (s, 0u32)
        };

        match time_part.len() {
            2 => {
                let hour = parse_u8_str(&time_part[0..2])?;
                if hour > 23 {
                    return Err(DcmError::Other(format!(
                        "invalid hour in DICOM time: {hour}"
                    )));
                }
                Ok(Self {
                    hour,
                    minute: 0,
                    second: 0,
                    fraction: 0,
                })
            }
            4 => {
                let hour = parse_u8_str(&time_part[0..2])?;
                let minute = parse_u8_str(&time_part[2..4])?;
                if hour > 23 {
                    return Err(DcmError::Other(format!(
                        "invalid hour in DICOM time: {hour}"
                    )));
                }
                if minute > 59 {
                    return Err(DcmError::Other(format!(
                        "invalid minute in DICOM time: {minute}"
                    )));
                }
                Ok(Self {
                    hour,
                    minute,
                    second: 0,
                    fraction: 0,
                })
            }
            6 => {
                let hour = parse_u8_str(&time_part[0..2])?;
                let minute = parse_u8_str(&time_part[2..4])?;
                let second = parse_u8_str(&time_part[4..6])?;
                if hour > 23 {
                    return Err(DcmError::Other(format!(
                        "invalid hour in DICOM time: {hour}"
                    )));
                }
                if minute > 59 {
                    return Err(DcmError::Other(format!(
                        "invalid minute in DICOM time: {minute}"
                    )));
                }
                if second > 59 {
                    return Err(DcmError::Other(format!(
                        "invalid second in DICOM time: {second}"
                    )));
                }
                Ok(Self {
                    hour,
                    minute,
                    second,
                    fraction,
                })
            }
            _ => Err(DcmError::Other(format!("invalid DICOM time: {:?}", s))),
        }
    }
}

impl fmt::Display for DicomTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}{:02}{:02}", self.hour, self.minute, self.second)?;
        if self.fraction > 0 {
            write!(f, ".{:06}", self.fraction)?;
        }
        Ok(())
    }
}

// ── DicomDateTime ─────────────────────────────────────────────────────────────

/// A DICOM DT (DateTime) value: YYYYMMDDHHMMSS.FFFFFF+ZZZZ.
#[derive(Debug, Clone, PartialEq)]
pub struct DicomDateTime {
    pub date: DicomDate,
    pub time: Option<DicomTime>,
    /// UTC offset in minutes, e.g. +0530 → 330, -0500 → -300.
    pub offset_minutes: Option<i16>,
}

impl DicomDateTime {
    /// Parse a DICOM DT string.
    pub fn parse(s: &str) -> DcmResult<Self> {
        let s = s.trim();
        if s.len() < 4 {
            return Err(DcmError::Other(format!("invalid DICOM datetime: {:?}", s)));
        }

        // Separate UTC offset: find trailing +/- that belong to timezone
        // Timezone is the last +HHMM or -HHMM
        let (dt_part, offset_minutes) = extract_tz_offset(s)?;

        // Date is always the first 8 chars (YYYYMMDD), but may be shorter
        let date_len = dt_part.len().min(8);
        let date_str = &dt_part[..date_len];
        // Pad date string to at least 4 chars
        let date = DicomDate::parse(date_str)?;

        let time = if dt_part.len() > 8 {
            Some(DicomTime::parse(&dt_part[8..])?)
        } else {
            None
        };

        Ok(Self {
            date,
            time,
            offset_minutes,
        })
    }
}

/// Extracts an optional trailing timezone offset (+HHMM or -HHMM) from a DT string.
fn extract_tz_offset(s: &str) -> DcmResult<(&str, Option<i16>)> {
    // Look for + or - that is not part of the date/time portion.
    // The date+time part is at most 21 chars (YYYYMMDDHHMMSS.FFFFFF),
    // so any sign after position 4 is a potential offset.
    let bytes = s.as_bytes();
    for i in (1..s.len()).rev() {
        if bytes[i] == b'+' || bytes[i] == b'-' {
            let tz_str = &s[i..];
            if tz_str.len() == 5 {
                let sign: i16 = if bytes[i] == b'+' { 1 } else { -1 };
                let hh = parse_u8_str(&tz_str[1..3])? as i16;
                let mm = parse_u8_str(&tz_str[3..5])? as i16;
                return Ok((&s[..i], Some(sign * (hh * 60 + mm))));
            }
        }
    }
    Ok((s, None))
}

impl fmt::Display for DicomDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.date)?;
        if let Some(ref t) = self.time {
            write!(f, "{}", t)?;
        }
        if let Some(offset) = self.offset_minutes {
            let sign = if offset >= 0 { '+' } else { '-' };
            let abs = offset.unsigned_abs();
            write!(f, "{}{:02}{:02}", sign, abs / 60, abs % 60)?;
        }
        Ok(())
    }
}

// ── PersonName ────────────────────────────────────────────────────────────────

/// A DICOM PN (Person Name) value.
///
/// Each name consists of up to three component groups (alphabetic, ideographic,
/// phonetic) separated by `=`. Within each group, the five name components
/// (family, given, middle, prefix, suffix) are separated by `^`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonName {
    pub alphabetic: String,
    pub ideographic: String,
    pub phonetic: String,
}

impl PersonName {
    /// Parse a DICOM PN string.
    pub fn parse(s: &str) -> Self {
        let mut parts = s.splitn(3, '=');
        PersonName {
            alphabetic: parts.next().unwrap_or("").to_string(),
            ideographic: parts.next().unwrap_or("").to_string(),
            phonetic: parts.next().unwrap_or("").to_string(),
        }
    }

    fn component(group: &str, index: usize) -> &str {
        group.split('^').nth(index).unwrap_or("")
    }

    pub fn last_name(&self) -> &str {
        Self::component(&self.alphabetic, 0)
    }

    pub fn first_name(&self) -> &str {
        Self::component(&self.alphabetic, 1)
    }

    pub fn middle_name(&self) -> &str {
        Self::component(&self.alphabetic, 2)
    }

    pub fn prefix(&self) -> &str {
        Self::component(&self.alphabetic, 3)
    }

    pub fn suffix(&self) -> &str {
        Self::component(&self.alphabetic, 4)
    }
}

impl fmt::Display for PersonName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Emit only as many groups as are non-empty, trimming trailing empty groups.
        if !self.phonetic.is_empty() {
            write!(
                f,
                "{}={}={}",
                self.alphabetic, self.ideographic, self.phonetic
            )
        } else if !self.ideographic.is_empty() {
            write!(f, "{}={}", self.alphabetic, self.ideographic)
        } else {
            write!(f, "{}", self.alphabetic)
        }
    }
}

// ── PixelData ─────────────────────────────────────────────────────────────────

/// One compressed frame worth of encapsulated Pixel Data fragments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncapsulatedFrame {
    pub fragments: Vec<Vec<u8>>,
}

/// Pixel data stored either as native (uncompressed) bytes or encapsulated
/// (compressed) fragments.
#[derive(Debug, Clone, PartialEq)]
pub enum PixelData {
    /// Uncompressed pixel data.
    Native { bytes: Vec<u8> },
    /// Encapsulated (compressed) pixel data with optional offset table.
    Encapsulated {
        offset_table: Vec<u32>,
        fragments: Vec<Vec<u8>>,
    },
}

impl PixelData {
    /// Split encapsulated pixel data into per-frame compressed payloads.
    pub fn encapsulated_frames(&self, number_of_frames: u32) -> DcmResult<Vec<Vec<u8>>> {
        encapsulated_frames(self, number_of_frames)
    }
}

/// Build encapsulated Pixel Data from per-frame compressed fragments.
///
/// Basic Offset Table entries are generated on fragment-item boundaries, using
/// the same `8 + fragment.len()` accounting expected by [`encapsulated_frames`].
pub fn build_encapsulated_pixel_data(frames: &[EncapsulatedFrame]) -> DcmResult<PixelData> {
    if frames.is_empty() {
        return Err(DcmError::Other(
            "build_encapsulated_pixel_data requires at least one frame".into(),
        ));
    }

    let mut offset_table = Vec::with_capacity(frames.len());
    let mut fragments = Vec::new();
    let mut offset = 0u32;

    for (frame_index, frame) in frames.iter().enumerate() {
        if frame.fragments.is_empty() {
            return Err(DcmError::Other(format!(
                "encapsulated frame {} has no fragments",
                frame_index + 1
            )));
        }

        offset_table.push(offset);
        for fragment in &frame.fragments {
            offset = offset
                .checked_add(fragment_item_length(fragment)?)
                .ok_or_else(|| {
                    DcmError::Other("fragment stream exceeds u32 offset range".into())
                })?;
            fragments.push(fragment.clone());
        }
    }

    Ok(PixelData::Encapsulated {
        offset_table,
        fragments,
    })
}

/// Convenience helper for the common one-fragment-per-frame case.
pub fn encapsulated_pixel_data_from_frames(frames: &[Vec<u8>]) -> DcmResult<PixelData> {
    let frames: Vec<EncapsulatedFrame> = frames
        .iter()
        .cloned()
        .map(|fragment| EncapsulatedFrame {
            fragments: vec![fragment],
        })
        .collect();
    build_encapsulated_pixel_data(&frames)
}

/// Split encapsulated pixel data into per-frame compressed payloads.
///
/// Supports:
/// - single-frame encapsulated objects
/// - multi-frame data with one fragment per frame and an empty BOT
/// - multi-fragment-per-frame data described by the Basic Offset Table
pub fn encapsulated_frames(
    pixel_data: &PixelData,
    number_of_frames: u32,
) -> DcmResult<Vec<Vec<u8>>> {
    if number_of_frames == 0 {
        return Err(DcmError::Other(
            "number_of_frames must be at least 1 for encapsulated Pixel Data".into(),
        ));
    }

    let PixelData::Encapsulated {
        offset_table,
        fragments,
    } = pixel_data
    else {
        return Err(DcmError::Other(
            "encapsulated_frames requires encapsulated Pixel Data".into(),
        ));
    };

    if fragments.is_empty() {
        return Err(DcmError::Other(
            "encapsulated Pixel Data has no fragments".into(),
        ));
    }

    if number_of_frames == 1 {
        return Ok(vec![fragments.concat()]);
    }

    if offset_table.is_empty() {
        if fragments.len() == number_of_frames as usize {
            return Ok(fragments.clone());
        }
        return Err(DcmError::Other(format!(
            "encapsulated Pixel Data for {number_of_frames} frames requires a Basic Offset Table or one fragment per frame, found {} fragment(s)",
            fragments.len()
        )));
    }

    if offset_table.len() != number_of_frames as usize {
        return Err(DcmError::Other(format!(
            "Basic Offset Table has {} entries, expected {number_of_frames}",
            offset_table.len()
        )));
    }

    let fragment_offsets = fragment_start_offsets(fragments)?;
    let total_length = total_fragment_stream_length(fragments)?;
    let mut frames = Vec::with_capacity(number_of_frames as usize);

    for frame_index in 0..number_of_frames as usize {
        let start_offset = offset_table[frame_index];
        let start_fragment = fragment_offsets
            .iter()
            .position(|&offset| offset == start_offset)
            .ok_or_else(|| {
                DcmError::Other(format!(
                    "Basic Offset Table entry {} does not align to a fragment boundary",
                    frame_index + 1
                ))
            })?;

        let end_fragment = if let Some(&next_offset) = offset_table.get(frame_index + 1) {
            if next_offset < start_offset {
                return Err(DcmError::Other(format!(
                    "Basic Offset Table entry {} points before the current frame start",
                    frame_index + 2
                )));
            }
            fragment_offsets
                .iter()
                .position(|&offset| offset == next_offset)
                .ok_or_else(|| {
                    DcmError::Other(format!(
                        "Basic Offset Table entry {} does not align to a fragment boundary",
                        frame_index + 2
                    ))
                })?
        } else {
            if start_offset > total_length {
                return Err(DcmError::Other(
                    "Basic Offset Table points beyond the fragment stream".into(),
                ));
            }
            fragments.len()
        };

        if end_fragment <= start_fragment {
            return Err(DcmError::Other(format!(
                "frame {} resolves to an empty fragment range",
                frame_index + 1
            )));
        }

        let mut frame = Vec::new();
        for fragment in &fragments[start_fragment..end_fragment] {
            frame.extend_from_slice(fragment);
        }
        frames.push(frame);
    }

    Ok(frames)
}

// ── Value ─────────────────────────────────────────────────────────────────────

/// The value held by a DICOM data element.
///
/// Each variant corresponds to one or more DICOM VRs. Numeric string VRs
/// (DS, IS) are stored already decoded as `f64`/`i64`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// No value (zero-length element).
    Empty,
    /// AE, CS, LO, LT, SH, ST, UC, UR, UT — multi-valued via backslash.
    Strings(Vec<String>),
    /// PN — person name with up to three component groups.
    PersonNames(Vec<PersonName>),
    /// UI — UID string.
    Uid(String),
    /// DA — date values.
    Date(Vec<DicomDate>),
    /// TM — time values.
    Time(Vec<DicomTime>),
    /// DT — datetime values.
    DateTime(Vec<DicomDateTime>),
    /// IS — integer string, decoded.
    Ints(Vec<i64>),
    /// DS — decimal string, decoded.
    Decimals(Vec<f64>),
    /// OB, UN — raw bytes.
    U8(Vec<u8>),
    /// US, OW — raw 16-bit words (interpret by VR).
    U16(Vec<u16>),
    /// SS — signed 16-bit integers.
    I16(Vec<i16>),
    /// UL, OL — 32-bit unsigned integers.
    U32(Vec<u32>),
    /// SL — 32-bit signed integers.
    I32(Vec<i32>),
    /// UV, OV — 64-bit unsigned integers.
    U64(Vec<u64>),
    /// SV — 64-bit signed integers.
    I64(Vec<i64>),
    /// FL, OF — 32-bit floats.
    F32(Vec<f32>),
    /// FD, OD — 64-bit floats.
    F64(Vec<f64>),
    /// AT — attribute tag pairs.
    Tags(Vec<Tag>),
    /// SQ — sequence of items (datasets).
    Sequence(Vec<DataSet>),
    /// Pixel data — (7FE0,0010).
    PixelData(PixelData),
}

impl Value {
    /// Returns the number of values (VM).
    pub fn multiplicity(&self) -> usize {
        match self {
            Value::Empty => 0,
            Value::Strings(v) => v.len(),
            Value::PersonNames(v) => v.len(),
            Value::Uid(_) => 1,
            Value::Date(v) => v.len(),
            Value::Time(v) => v.len(),
            Value::DateTime(v) => v.len(),
            Value::Ints(v) => v.len(),
            Value::Decimals(v) => v.len(),
            Value::U8(v) => v.len(),
            Value::U16(v) => v.len(),
            Value::I16(v) => v.len(),
            Value::U32(v) => v.len(),
            Value::I32(v) => v.len(),
            Value::U64(v) => v.len(),
            Value::I64(v) => v.len(),
            Value::F32(v) => v.len(),
            Value::F64(v) => v.len(),
            Value::Tags(v) => v.len(),
            Value::Sequence(v) => v.len(),
            Value::PixelData(_) => 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.multiplicity() == 0
    }

    /// Returns the first string value, if this is a `Strings` or `Uid` variant.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::Strings(v) => v.first().map(|s| s.as_str()),
            Value::Uid(s) => Some(s.as_str()),
            Value::PersonNames(v) => v.first().map(|p| p.alphabetic.as_str()),
            _ => None,
        }
    }

    pub fn as_strings(&self) -> Option<&[String]> {
        match self {
            Value::Strings(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    pub fn as_u16(&self) -> Option<u16> {
        match self {
            Value::U16(v) => v.first().copied(),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Value::U32(v) => v.first().copied(),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Value::I32(v) => v.first().copied(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::F64(v) => v.first().copied(),
            Value::Decimals(v) => v.first().copied(),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::U8(v) => Some(v.as_slice()),
            Value::PixelData(PixelData::Native { bytes }) => Some(bytes.as_slice()),
            _ => None,
        }
    }

    /// Returns a human-readable string representation (like dcmdump output).
    pub fn to_display_string(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Strings(v) => v.join("\\"),
            Value::PersonNames(v) => v
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::Uid(s) => s.clone(),
            Value::Date(v) => v
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::Time(v) => v
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::DateTime(v) => v
                .iter()
                .map(|dt| dt.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::Ints(v) => v
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::Decimals(v) => v
                .iter()
                .map(|n| format_f64(*n))
                .collect::<Vec<_>>()
                .join("\\"),
            Value::U8(v) => format!("({} bytes)", v.len()),
            Value::U16(v) => v
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::I16(v) => v
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::U32(v) => v
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::I32(v) => v
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::U64(v) => v
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::I64(v) => v
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join("\\"),
            Value::F32(v) => v
                .iter()
                .map(|n| format!("{}", n))
                .collect::<Vec<_>>()
                .join("\\"),
            Value::F64(v) => v
                .iter()
                .map(|n| format_f64(*n))
                .collect::<Vec<_>>()
                .join("\\"),
            Value::Tags(v) => v
                .iter()
                .map(|t| format!("({:04X},{:04X})", t.group, t.element))
                .collect::<Vec<_>>()
                .join("\\"),
            Value::Sequence(v) => format!("(Sequence with {} item(s))", v.len()),
            Value::PixelData(PixelData::Native { bytes }) => {
                format!("(PixelData, {} bytes)", bytes.len())
            }
            Value::PixelData(PixelData::Encapsulated { fragments, .. }) => {
                format!("(PixelData, {} fragment(s))", fragments.len())
            }
        }
    }

    /// Approximate encoded byte length (for dcmdump `# length` field).
    pub(crate) fn encoded_len(&self) -> usize {
        match self {
            Value::Empty => 0,
            Value::Strings(v) => {
                let total: usize = v.iter().map(|s| s.len()).sum();
                total + v.len().saturating_sub(1)
            }
            Value::PersonNames(v) => {
                let total: usize = v.iter().map(|p| p.to_string().len()).sum();
                total + v.len().saturating_sub(1)
            }
            Value::Uid(s) => s.len(),
            Value::Date(v) => v.len() * 8,
            Value::Time(v) => v.len() * 14,
            Value::DateTime(v) => v.len() * 26,
            Value::Ints(v) => {
                v.iter().map(|n| n.to_string().len()).sum::<usize>() + v.len().saturating_sub(1)
            }
            Value::Decimals(v) => {
                v.iter().map(|n| format_f64(*n).len()).sum::<usize>() + v.len().saturating_sub(1)
            }
            Value::U8(v) => v.len(),
            Value::U16(v) => v.len() * 2,
            Value::I16(v) => v.len() * 2,
            Value::U32(v) => v.len() * 4,
            Value::I32(v) => v.len() * 4,
            Value::U64(v) => v.len() * 8,
            Value::I64(v) => v.len() * 8,
            Value::F32(v) => v.len() * 4,
            Value::F64(v) => v.len() * 8,
            Value::Tags(v) => v.len() * 4,
            Value::Sequence(_) => 0,
            Value::PixelData(PixelData::Native { bytes }) => bytes.len(),
            Value::PixelData(PixelData::Encapsulated { fragments, .. }) => {
                fragments.iter().map(|f| f.len()).sum()
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_u8_str(s: &str) -> DcmResult<u8> {
    s.parse::<u8>()
        .map_err(|_| DcmError::Other(format!("expected u8, got {:?}", s)))
}

fn parse_u16_str(s: &str) -> DcmResult<u16> {
    s.parse::<u16>()
        .map_err(|_| DcmError::Other(format!("expected u16, got {:?}", s)))
}

fn parse_u32_str(s: &str) -> DcmResult<u32> {
    s.parse::<u32>()
        .map_err(|_| DcmError::Other(format!("expected u32, got {:?}", s)))
}

fn fragment_start_offsets(fragments: &[Vec<u8>]) -> DcmResult<Vec<u32>> {
    let mut offsets = Vec::with_capacity(fragments.len());
    let mut cursor = 0u32;
    for fragment in fragments {
        offsets.push(cursor);
        cursor = cursor
            .checked_add(fragment_item_length(fragment)?)
            .ok_or_else(|| DcmError::Other("fragment stream exceeds u32 offset range".into()))?;
    }
    Ok(offsets)
}

fn total_fragment_stream_length(fragments: &[Vec<u8>]) -> DcmResult<u32> {
    fragments.iter().try_fold(0u32, |total, fragment| {
        total
            .checked_add(fragment_item_length(fragment)?)
            .ok_or_else(|| DcmError::Other("fragment stream exceeds u32 offset range".into()))
    })
}

fn fragment_item_length(fragment: &[u8]) -> DcmResult<u32> {
    let len = u32::try_from(fragment.len())
        .map_err(|_| DcmError::Other("fragment length exceeds u32 range".into()))?;
    len.checked_add(8)
        .ok_or_else(|| DcmError::Other("fragment item length exceeds u32 range".into()))
}

/// Format an f64 without trailing zeros but with at least one decimal place.
fn format_f64(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{:.1}", v)
    } else {
        format!("{}", v)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DicomDate ───────────────────────────────────────────────────────

    #[test]
    fn date_full_parse() {
        let d = DicomDate::parse("20231215").unwrap();
        assert_eq!(d.year, 2023);
        assert_eq!(d.month, 12);
        assert_eq!(d.day, 15);
    }

    #[test]
    fn date_year_only() {
        let d = DicomDate::parse("2023").unwrap();
        assert_eq!(d.year, 2023);
        assert_eq!(d.month, 0);
        assert_eq!(d.day, 0);
    }

    #[test]
    fn date_year_month() {
        let d = DicomDate::parse("202312").unwrap();
        assert_eq!(d.year, 2023);
        assert_eq!(d.month, 12);
        assert_eq!(d.day, 0);
    }

    #[test]
    fn date_display_full() {
        let d = DicomDate {
            year: 2023,
            month: 12,
            day: 15,
        };
        assert_eq!(d.to_string(), "20231215");
    }

    #[test]
    fn date_display_partial_year() {
        let d = DicomDate {
            year: 2023,
            month: 0,
            day: 0,
        };
        assert_eq!(d.to_string(), "2023");
    }

    #[test]
    fn date_display_partial_year_month() {
        let d = DicomDate {
            year: 2023,
            month: 12,
            day: 0,
        };
        assert_eq!(d.to_string(), "202312");
    }

    #[test]
    fn date_legacy_format() {
        let d = DicomDate::from_da_str("2023.12.15").unwrap();
        assert_eq!(d.year, 2023);
        assert_eq!(d.month, 12);
        assert_eq!(d.day, 15);
    }

    #[test]
    fn date_invalid() {
        assert!(DicomDate::parse("20231").is_err());
        assert!(DicomDate::parse("2023121").is_err());
        assert!(DicomDate::parse("abcdefgh").is_err());
    }

    // ── DicomTime ───────────────────────────────────────────────────────

    #[test]
    fn time_full_parse() {
        let t = DicomTime::parse("143022.500000").unwrap();
        assert_eq!(t.hour, 14);
        assert_eq!(t.minute, 30);
        assert_eq!(t.second, 22);
        assert_eq!(t.fraction, 500000);
    }

    #[test]
    fn time_partial_hour() {
        let t = DicomTime::parse("14").unwrap();
        assert_eq!(t.hour, 14);
        assert_eq!(t.minute, 0);
        assert_eq!(t.second, 0);
        assert_eq!(t.fraction, 0);
    }

    #[test]
    fn time_partial_hour_minute() {
        let t = DicomTime::parse("1430").unwrap();
        assert_eq!(t.hour, 14);
        assert_eq!(t.minute, 30);
        assert_eq!(t.second, 0);
    }

    #[test]
    fn time_partial_no_fraction() {
        let t = DicomTime::parse("143022").unwrap();
        assert_eq!(t.hour, 14);
        assert_eq!(t.minute, 30);
        assert_eq!(t.second, 22);
        assert_eq!(t.fraction, 0);
    }

    #[test]
    fn time_fraction_short() {
        // Short fraction is zero-padded on the right
        let t = DicomTime::parse("143022.5").unwrap();
        assert_eq!(t.fraction, 500000);
    }

    #[test]
    fn time_display() {
        let t = DicomTime {
            hour: 14,
            minute: 30,
            second: 22,
            fraction: 500000,
        };
        assert_eq!(t.to_string(), "143022.500000");
    }

    #[test]
    fn time_display_no_fraction() {
        let t = DicomTime {
            hour: 14,
            minute: 30,
            second: 22,
            fraction: 0,
        };
        assert_eq!(t.to_string(), "143022");
    }

    // ── DicomDateTime ───────────────────────────────────────────────────

    #[test]
    fn datetime_full_parse() {
        let dt = DicomDateTime::parse("20231215143022.000000+0530").unwrap();
        assert_eq!(dt.date.year, 2023);
        assert_eq!(dt.date.month, 12);
        assert_eq!(dt.date.day, 15);
        let t = dt.time.unwrap();
        assert_eq!(t.hour, 14);
        assert_eq!(t.minute, 30);
        assert_eq!(t.second, 22);
        assert_eq!(dt.offset_minutes, Some(330)); // +05:30 = 5*60+30 = 330
    }

    #[test]
    fn datetime_negative_offset() {
        let dt = DicomDateTime::parse("20231215143022.000000-0500").unwrap();
        assert_eq!(dt.offset_minutes, Some(-300));
    }

    #[test]
    fn datetime_no_time() {
        let dt = DicomDateTime::parse("20231215").unwrap();
        assert_eq!(dt.date.year, 2023);
        assert!(dt.time.is_none());
        assert!(dt.offset_minutes.is_none());
    }

    #[test]
    fn datetime_display_roundtrip() {
        // Use non-zero fraction so Display includes it, enabling exact round-trip.
        let s = "20231215143022.500000+0530";
        let dt = DicomDateTime::parse(s).unwrap();
        assert_eq!(dt.to_string(), s);
    }

    #[test]
    fn datetime_display_roundtrip_no_fraction() {
        // Without a fractional second the display omits the decimal.
        let s = "20231215143022+0530";
        let dt = DicomDateTime::parse(s).unwrap();
        assert_eq!(dt.to_string(), s);
    }

    // ── PersonName ──────────────────────────────────────────────────────

    #[test]
    fn pn_simple() {
        let pn = PersonName::parse("Eichelberg^Marco^^Dr.");
        assert_eq!(pn.last_name(), "Eichelberg");
        assert_eq!(pn.first_name(), "Marco");
        assert_eq!(pn.middle_name(), "");
        assert_eq!(pn.prefix(), "Dr.");
        assert_eq!(pn.suffix(), "");
    }

    #[test]
    fn pn_multi_component() {
        let pn = PersonName::parse("Smith^John=\u{5C71}\u{7530}^\u{592A}\u{90CE}=\u{3084}\u{307E}\u{3060}^\u{305F}\u{308D}\u{3046}");
        assert_eq!(pn.last_name(), "Smith");
        assert_eq!(pn.first_name(), "John");
        assert!(!pn.ideographic.is_empty());
        assert!(!pn.phonetic.is_empty());
    }

    #[test]
    fn pn_display_single_group() {
        let pn = PersonName::parse("Smith^John");
        assert_eq!(pn.to_string(), "Smith^John");
    }

    #[test]
    fn pn_display_two_groups() {
        let pn = PersonName::parse("Smith^John=SJ");
        assert_eq!(pn.to_string(), "Smith^John=SJ");
    }

    // ── Value ───────────────────────────────────────────────────────────

    #[test]
    fn value_multiplicity() {
        assert_eq!(Value::Empty.multiplicity(), 0);
        assert_eq!(
            Value::Strings(vec!["a".into(), "b".into()]).multiplicity(),
            2
        );
        assert_eq!(Value::U16(vec![1, 2, 3]).multiplicity(), 3);
        assert_eq!(Value::Uid("1.2.3".into()).multiplicity(), 1);
        assert_eq!(Value::Sequence(vec![]).multiplicity(), 0);
    }

    #[test]
    fn value_is_empty() {
        assert!(Value::Empty.is_empty());
        assert!(Value::Strings(vec![]).is_empty());
        assert!(!Value::Strings(vec!["x".into()]).is_empty());
    }

    #[test]
    fn value_as_string() {
        let v = Value::Strings(vec!["hello".into(), "world".into()]);
        assert_eq!(v.as_string(), Some("hello"));
        assert_eq!(v.as_strings().unwrap().len(), 2);
    }

    #[test]
    fn value_as_uid() {
        let v = Value::Uid("1.2.840.10008.1.1".into());
        assert_eq!(v.as_string(), Some("1.2.840.10008.1.1"));
    }

    #[test]
    fn value_as_numeric() {
        let v = Value::U16(vec![512]);
        assert_eq!(v.as_u16(), Some(512));

        let v = Value::U32(vec![65536]);
        assert_eq!(v.as_u32(), Some(65536));

        let v = Value::I32(vec![-1]);
        assert_eq!(v.as_i32(), Some(-1));

        let v = Value::F64(vec![2.78]);
        assert_eq!(v.as_f64(), Some(2.78));
    }

    #[test]
    fn value_to_display_string_strings() {
        let v = Value::Strings(vec!["foo".into(), "bar".into()]);
        assert_eq!(v.to_display_string(), "foo\\bar");
    }

    #[test]
    fn value_to_display_string_u16() {
        let v = Value::U16(vec![512, 256]);
        assert_eq!(v.to_display_string(), "512\\256");
    }

    #[test]
    fn value_to_display_string_sequence() {
        let v = Value::Sequence(vec![]);
        assert_eq!(v.to_display_string(), "(Sequence with 0 item(s))");
    }

    #[test]
    fn value_as_bytes() {
        let v = Value::U8(vec![1, 2, 3]);
        assert_eq!(v.as_bytes(), Some(&[1u8, 2, 3][..]));
    }

    #[test]
    fn encapsulated_frames_single_frame_concatenates_fragments() {
        let pixel_data = PixelData::Encapsulated {
            offset_table: vec![0],
            fragments: vec![vec![1, 2], vec![3, 4]],
        };

        let frames = encapsulated_frames(&pixel_data, 1).unwrap();
        assert_eq!(frames, vec![vec![1, 2, 3, 4]]);
    }

    #[test]
    fn encapsulated_frames_handles_empty_bot_one_fragment_per_frame() {
        let pixel_data = PixelData::Encapsulated {
            offset_table: vec![],
            fragments: vec![vec![1, 2], vec![3, 4]],
        };

        let frames = encapsulated_frames(&pixel_data, 2).unwrap();
        assert_eq!(frames, vec![vec![1, 2], vec![3, 4]]);
    }

    #[test]
    fn encapsulated_frames_uses_basic_offset_table_for_multi_fragment_frames() {
        let pixel_data = PixelData::Encapsulated {
            offset_table: vec![0, 22],
            fragments: vec![vec![1, 2], vec![3, 4, 5, 6], vec![7, 8, 9]],
        };

        let frames = encapsulated_frames(&pixel_data, 2).unwrap();
        assert_eq!(frames, vec![vec![1, 2, 3, 4, 5, 6], vec![7, 8, 9]]);
    }

    #[test]
    fn encapsulated_frames_rejects_malformed_offset_table() {
        let pixel_data = PixelData::Encapsulated {
            offset_table: vec![0, 99],
            fragments: vec![vec![1, 2], vec![3, 4]],
        };

        let err = encapsulated_frames(&pixel_data, 2).unwrap_err();
        assert!(err.to_string().contains("does not align"));
    }

    #[test]
    fn build_encapsulated_pixel_data_uses_fragment_item_boundaries() {
        let pixel_data = encapsulated_pixel_data_from_frames(&[vec![1, 2, 3], vec![4, 5]]).unwrap();

        match pixel_data {
            PixelData::Encapsulated {
                offset_table,
                fragments,
            } => {
                assert_eq!(offset_table, vec![0, 11]);
                assert_eq!(fragments, vec![vec![1, 2, 3], vec![4, 5]]);
            }
            PixelData::Native { .. } => panic!("expected encapsulated pixel data"),
        }
    }

    #[test]
    fn build_encapsulated_pixel_data_handles_multi_fragment_frames() {
        let pixel_data = build_encapsulated_pixel_data(&[
            EncapsulatedFrame {
                fragments: vec![vec![1, 2], vec![3, 4, 5, 6]],
            },
            EncapsulatedFrame {
                fragments: vec![vec![7, 8, 9]],
            },
        ])
        .unwrap();

        match &pixel_data {
            PixelData::Encapsulated { offset_table, .. } => {
                assert_eq!(offset_table, &vec![0, 22]);
            }
            PixelData::Native { .. } => panic!("expected encapsulated pixel data"),
        }

        let frames = encapsulated_frames(&pixel_data, 2).unwrap();
        assert_eq!(frames, vec![vec![1, 2, 3, 4, 5, 6], vec![7, 8, 9]]);
    }

    #[test]
    fn build_encapsulated_pixel_data_rejects_empty_frames() {
        assert!(build_encapsulated_pixel_data(&[]).is_err());
        assert!(build_encapsulated_pixel_data(&[EncapsulatedFrame { fragments: vec![] }]).is_err());
    }
}
