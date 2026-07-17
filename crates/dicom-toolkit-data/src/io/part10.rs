//! Bounded inspection of DICOM Part 10 files.
//!
//! These APIs read File Meta Information and the identifying dataset prefix
//! without materializing or decompressing Pixel Data.

use std::io::{Read, Seek, SeekFrom};

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_core::uid::Uid;
use dicom_toolkit_dict::{tags, Tag, Vr};

use crate::io::reader::DicomReader;
use crate::io::transfer::TransferSyntaxProperties;
use crate::meta_info::FileMetaInformation;

const PART10_PREAMBLE_AND_PREFIX_LENGTH: u64 = 132;
const FILE_META_GROUP_LENGTH_ELEMENT_LENGTH: u64 = 12;
const INSPECTION_LIMIT_ERROR_PREFIX: &str = "Part 10 inspection limit exceeded: ";

/// A resource ceiling applied while inspecting a Part 10 file.
///
/// These limits are policy controls, not statements about whether a DICOM file
/// is syntactically valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part10InspectionLimit {
    FileMetaBytes,
    DatasetPrefixBytes,
    SequenceDepth,
    UidValueBytes,
}

impl Part10InspectionLimit {
    const fn marker(self) -> &'static str {
        match self {
            Self::FileMetaBytes => "file-meta-bytes",
            Self::DatasetPrefixBytes => "dataset-prefix-bytes",
            Self::SequenceDepth => "sequence-depth",
            Self::UidValueBytes => "uid-value-bytes",
        }
    }

    fn from_marker(marker: &str) -> Option<Self> {
        match marker {
            "file-meta-bytes" => Some(Self::FileMetaBytes),
            "dataset-prefix-bytes" => Some(Self::DatasetPrefixBytes),
            "sequence-depth" => Some(Self::SequenceDepth),
            "uid-value-bytes" => Some(Self::UidValueBytes),
            _ => None,
        }
    }
}

/// Return the policy limit exceeded by a bounded Part 10 inspection error.
///
/// A `None` result means that the error represents malformed input, an
/// unsupported operation, or another non-limit failure. Existing inspection
/// APIs retain their `DcmResult` return type; this additive helper lets callers
/// distinguish a valid-but-too-expensive object from invalid Part 10 input.
pub fn part10_inspection_limit_from_error(error: &DcmError) -> Option<Part10InspectionLimit> {
    let DcmError::InvalidFile { reason } = error else {
        return None;
    };
    let marker = reason
        .strip_prefix(INSPECTION_LIMIT_ERROR_PREFIX)?
        .split_once(' ')?
        .0;
    Part10InspectionLimit::from_marker(marker)
}

/// Resource limits for bounded Part 10 inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part10ReadLimits {
    /// Maximum value accepted from File Meta Information Group Length.
    pub maximum_file_meta_bytes: u64,
    /// Maximum number of decompressed dataset-prefix bytes inspected while
    /// locating required identity attributes.
    pub maximum_dataset_prefix_bytes: u64,
    /// Maximum nesting depth while skipping undefined-length sequences.
    pub maximum_sequence_depth: usize,
    /// Maximum encoded byte length accepted for one UID value.
    pub maximum_uid_value_bytes: usize,
}

impl Default for Part10ReadLimits {
    fn default() -> Self {
        Self {
            maximum_file_meta_bytes: 1024 * 1024,
            maximum_dataset_prefix_bytes: 16 * 1024 * 1024,
            maximum_sequence_depth: 64,
            maximum_uid_value_bytes: 66,
        }
    }
}

/// File Meta Information and the exact encoded dataset region in a Part 10
/// file.
#[derive(Debug, Clone, PartialEq)]
pub struct Part10FileLayout {
    pub file_meta: FileMetaInformation,
    pub dataset_offset: u64,
    pub dataset_length: u64,
}

/// Required top-level identity attributes from a DICOM dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part10FileIdentity {
    pub sop_class_uid: Uid,
    pub sop_instance_uid: Uid,
    pub study_instance_uid: Uid,
    pub series_instance_uid: Uid,
}

/// Bounded Part 10 layout and validated dataset identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Part10FileIndex {
    pub layout: Part10FileLayout,
    pub identity: Part10FileIdentity,
}

/// Read only the bounded File Meta Information of a standard Part 10 file.
///
/// The reader is positioned at `dataset_offset` when this function succeeds.
pub fn read_part10_file_layout<R: Read + Seek>(
    reader: &mut R,
    limits: Part10ReadLimits,
) -> DcmResult<Part10FileLayout> {
    validate_limits(limits)?;
    reader.seek(SeekFrom::Start(0))?;

    let mut preamble_and_prefix = [0_u8; PART10_PREAMBLE_AND_PREFIX_LENGTH as usize];
    reader.read_exact(&mut preamble_and_prefix)?;
    if &preamble_and_prefix[128..] != b"DICM" {
        return invalid_file("missing DICM prefix at byte offset 128");
    }

    let mut group_length_element = [0_u8; FILE_META_GROUP_LENGTH_ELEMENT_LENGTH as usize];
    reader.read_exact(&mut group_length_element)?;
    if group_length_element[0..4] != [0x02, 0x00, 0x00, 0x00]
        || group_length_element[4..6] != *b"UL"
        || group_length_element[6..8] != [0x04, 0x00]
    {
        return invalid_file(
            "File Meta Information must begin with (0002,0000) UL with a four-byte value",
        );
    }

    let file_meta_length = u32::from_le_bytes([
        group_length_element[8],
        group_length_element[9],
        group_length_element[10],
        group_length_element[11],
    ]) as u64;
    if file_meta_length == 0 {
        return invalid_file("File Meta Information Group Length must not be zero");
    }
    if file_meta_length > limits.maximum_file_meta_bytes {
        return Err(inspection_limit_error(
            Part10InspectionLimit::FileMetaBytes,
            limits.maximum_file_meta_bytes,
        ));
    }

    let dataset_offset = PART10_PREAMBLE_AND_PREFIX_LENGTH
        .checked_add(FILE_META_GROUP_LENGTH_ELEMENT_LENGTH)
        .and_then(|offset| offset.checked_add(file_meta_length))
        .ok_or_else(|| DcmError::InvalidFile {
            reason: "Part 10 dataset offset overflow".into(),
        })?;
    let file_length = reader.seek(SeekFrom::End(0))?;
    if dataset_offset > file_length {
        return invalid_file(format!(
            "File Meta Information ends at {dataset_offset}, beyond file length {file_length}"
        ));
    }

    reader.seek(SeekFrom::Start(
        PART10_PREAMBLE_AND_PREFIX_LENGTH + FILE_META_GROUP_LENGTH_ELEMENT_LENGTH,
    ))?;
    let file_meta_length_usize =
        usize::try_from(file_meta_length).map_err(|_| DcmError::InvalidFile {
            reason: "File Meta Information length cannot be represented on this platform".into(),
        })?;
    let mut encoded_file_meta = vec![0_u8; file_meta_length_usize];
    reader.read_exact(&mut encoded_file_meta)?;
    let file_meta_dataset =
        DicomReader::new(encoded_file_meta.as_slice()).read_dataset("1.2.840.10008.1.2.1")?;
    if file_meta_dataset
        .tags()
        .any(|tag| tag.group != 0x0002 || tag == tags::FILE_META_INFORMATION_GROUP_LENGTH)
    {
        return invalid_file("File Meta Information Group Length crosses a non-0002 element");
    }

    let file_meta = FileMetaInformation::from_dataset(&file_meta_dataset)?;
    validate_required_meta_uid(
        "Media Storage SOP Class UID",
        &file_meta.media_storage_sop_class_uid,
    )?;
    validate_required_meta_uid(
        "Media Storage SOP Instance UID",
        &file_meta.media_storage_sop_instance_uid,
    )?;
    validate_required_meta_uid("Transfer Syntax UID", &file_meta.transfer_syntax_uid)?;

    reader.seek(SeekFrom::Start(dataset_offset))?;
    Ok(Part10FileLayout {
        file_meta,
        dataset_offset,
        dataset_length: file_length - dataset_offset,
    })
}

/// Read bounded File Meta Information and the required dataset identity.
///
/// The identity scan stops after Series Instance UID and never reads Pixel
/// Data. Defined-length values are consumed through a fixed-size scratch
/// buffer so the same limits also apply to deflated transfer syntaxes.
pub fn read_part10_file_index<R: Read + Seek>(
    reader: &mut R,
    limits: Part10ReadLimits,
) -> DcmResult<Part10FileIndex> {
    let layout = read_part10_file_layout(reader, limits)?;
    if layout.dataset_length == 0 {
        return invalid_file("Part 10 file contains no dataset");
    }
    if dicom_toolkit_dict::transfer_syntaxes::by_uid(&layout.file_meta.transfer_syntax_uid)
        .is_none()
    {
        return invalid_file(format!(
            "unsupported Transfer Syntax UID {}",
            layout.file_meta.transfer_syntax_uid
        ));
    }

    let properties = TransferSyntaxProperties::from_uid(&layout.file_meta.transfer_syntax_uid);
    let dataset_reader = Read::take(reader, layout.dataset_length);
    let identity = if properties.is_deflated {
        let decoder = flate2::read::DeflateDecoder::new(dataset_reader);
        scan_dataset_identity(
            decoder,
            true,
            true,
            limits.maximum_dataset_prefix_bytes,
            limits.maximum_sequence_depth,
            limits.maximum_uid_value_bytes,
        )?
    } else {
        scan_dataset_identity(
            dataset_reader,
            properties.is_explicit_vr(),
            properties.is_little_endian(),
            limits.maximum_dataset_prefix_bytes,
            limits.maximum_sequence_depth,
            limits.maximum_uid_value_bytes,
        )?
    };

    if identity.sop_class_uid.as_str() != layout.file_meta.media_storage_sop_class_uid {
        return invalid_file("dataset SOP Class UID does not match File Meta Information");
    }
    if identity.sop_instance_uid.as_str() != layout.file_meta.media_storage_sop_instance_uid {
        return invalid_file("dataset SOP Instance UID does not match File Meta Information");
    }

    Ok(Part10FileIndex { layout, identity })
}

fn validate_limits(limits: Part10ReadLimits) -> DcmResult<()> {
    if limits.maximum_file_meta_bytes == 0
        || limits.maximum_dataset_prefix_bytes == 0
        || limits.maximum_sequence_depth == 0
        || limits.maximum_uid_value_bytes == 0
    {
        return Err(DcmError::Other(
            "Part 10 read limits must all be greater than zero".into(),
        ));
    }
    Ok(())
}

fn validate_required_meta_uid(name: &str, value: &str) -> DcmResult<()> {
    Uid::new(value)
        .map(|_| ())
        .map_err(|error| DcmError::InvalidFile {
            reason: format!("invalid or missing {name}: {error}"),
        })
}

fn invalid_file<T>(reason: impl Into<String>) -> DcmResult<T> {
    Err(DcmError::InvalidFile {
        reason: reason.into(),
    })
}

fn inspection_limit_error(limit: Part10InspectionLimit, maximum: u64) -> DcmError {
    DcmError::InvalidFile {
        reason: format!(
            "{INSPECTION_LIMIT_ERROR_PREFIX}{} maximum {maximum}",
            limit.marker()
        ),
    }
}

struct PrefixReader<R> {
    inner: R,
    consumed: u64,
    limit: u64,
}

impl<R: Read> PrefixReader<R> {
    fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            consumed: 0,
            limit,
        }
    }

    fn read_exact(&mut self, buffer: &mut [u8]) -> DcmResult<()> {
        let length = buffer.len() as u64;
        let next = self
            .consumed
            .checked_add(length)
            .ok_or_else(|| DcmError::InvalidFile {
                reason: "dataset prefix byte count overflow".into(),
            })?;
        if next > self.limit {
            return Err(inspection_limit_error(
                Part10InspectionLimit::DatasetPrefixBytes,
                self.limit,
            ));
        }
        self.inner.read_exact(buffer)?;
        self.consumed = next;
        Ok(())
    }

    fn discard(&mut self, mut length: u64) -> DcmResult<()> {
        let mut scratch = [0_u8; 8192];
        while length > 0 {
            let chunk_length = usize::try_from(length.min(scratch.len() as u64)).map_err(|_| {
                DcmError::InvalidFile {
                    reason: "dataset element length cannot be represented on this platform".into(),
                }
            })?;
            self.read_exact(&mut scratch[..chunk_length])?;
            length -= chunk_length as u64;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct ElementHeader {
    tag: Tag,
    vr: Vr,
    length: u32,
}

impl ElementHeader {
    fn has_undefined_length(self) -> bool {
        self.length == u32::MAX
    }
}

fn scan_dataset_identity<R: Read>(
    reader: R,
    explicit_vr: bool,
    little_endian: bool,
    maximum_prefix_bytes: u64,
    maximum_sequence_depth: usize,
    maximum_uid_value_bytes: usize,
) -> DcmResult<Part10FileIdentity> {
    let mut reader = PrefixReader::new(reader, maximum_prefix_bytes);
    let mut previous_tag = None;
    let mut sop_class_uid = None;
    let mut sop_instance_uid = None;
    let mut study_instance_uid = None;
    let mut series_instance_uid = None;

    loop {
        let header = read_element_header(&mut reader, explicit_vr, little_endian)?;
        if header.tag.is_delimiter() {
            return invalid_file("unexpected delimiter in top-level dataset");
        }
        if previous_tag.is_some_and(|tag| header.tag <= tag) {
            return invalid_file("top-level dataset elements are not in ascending tag order");
        }
        previous_tag = Some(header.tag);

        if header.tag > tags::SERIES_INSTANCE_UID {
            break;
        }

        let target = match header.tag {
            tag if tag == tags::SOP_CLASS_UID => &mut sop_class_uid,
            tag if tag == tags::SOP_INSTANCE_UID => &mut sop_instance_uid,
            tag if tag == tags::STUDY_INSTANCE_UID => &mut study_instance_uid,
            tag if tag == tags::SERIES_INSTANCE_UID => &mut series_instance_uid,
            _ => {
                skip_element_value(
                    &mut reader,
                    header,
                    explicit_vr,
                    little_endian,
                    0,
                    maximum_sequence_depth,
                )?;
                continue;
            }
        };
        *target = Some(read_uid_value(
            &mut reader,
            header,
            maximum_uid_value_bytes,
        )?);

        if series_instance_uid.is_some() {
            break;
        }
    }

    Ok(Part10FileIdentity {
        sop_class_uid: required_identity_uid("SOP Class UID", sop_class_uid)?,
        sop_instance_uid: required_identity_uid("SOP Instance UID", sop_instance_uid)?,
        study_instance_uid: required_identity_uid("Study Instance UID", study_instance_uid)?,
        series_instance_uid: required_identity_uid("Series Instance UID", series_instance_uid)?,
    })
}

fn required_identity_uid(name: &str, value: Option<Uid>) -> DcmResult<Uid> {
    value.ok_or_else(|| DcmError::InvalidFile {
        reason: format!("missing required top-level {name}"),
    })
}

fn read_uid_value<R: Read>(
    reader: &mut PrefixReader<R>,
    header: ElementHeader,
    maximum_uid_value_bytes: usize,
) -> DcmResult<Uid> {
    if header.vr != Vr::UI || header.has_undefined_length() {
        return invalid_file(format!(
            "identity element ({:04X},{:04X}) must have defined UI encoding",
            header.tag.group, header.tag.element
        ));
    }
    let length = header.length as usize;
    if length == 0 || length > maximum_uid_value_bytes {
        if length > maximum_uid_value_bytes {
            return Err(inspection_limit_error(
                Part10InspectionLimit::UidValueBytes,
                maximum_uid_value_bytes as u64,
            ));
        }
        return invalid_file("identity UID must not be empty");
    }
    let mut encoded = vec![0_u8; length];
    reader.read_exact(&mut encoded)?;
    let value = std::str::from_utf8(&encoded)
        .map_err(|_| DcmError::InvalidFile {
            reason: "identity UID is not ASCII/UTF-8".into(),
        })?
        .trim_end_matches('\0');
    Uid::new(value).map_err(|error| DcmError::InvalidFile {
        reason: format!("invalid identity UID: {error}"),
    })
}

fn read_element_header<R: Read>(
    reader: &mut PrefixReader<R>,
    explicit_vr: bool,
    little_endian: bool,
) -> DcmResult<ElementHeader> {
    let group = read_u16(reader, little_endian)?;
    let element = read_u16(reader, little_endian)?;
    let tag = Tag::new(group, element);
    if tag.is_delimiter() {
        return Ok(ElementHeader {
            tag,
            vr: Vr::UN,
            length: read_u32(reader, little_endian)?,
        });
    }

    if explicit_vr {
        let mut encoded_vr = [0_u8; 2];
        reader.read_exact(&mut encoded_vr)?;
        let vr = Vr::from_bytes(encoded_vr).ok_or_else(|| DcmError::InvalidFile {
            reason: format!(
                "invalid explicit VR {:?} for ({:04X},{:04X})",
                encoded_vr, tag.group, tag.element
            ),
        })?;
        let length = if vr.has_long_explicit_length() {
            let reserved = read_u16(reader, little_endian)?;
            if reserved != 0 {
                return invalid_file(format!(
                    "non-zero reserved bytes for ({:04X},{:04X})",
                    tag.group, tag.element
                ));
            }
            read_u32(reader, little_endian)?
        } else {
            read_u16(reader, little_endian)? as u32
        };
        Ok(ElementHeader { tag, vr, length })
    } else {
        Ok(ElementHeader {
            tag,
            vr: crate::io::transfer::implicit_vr_for_tag(tag),
            length: read_u32(reader, little_endian)?,
        })
    }
}

fn skip_element_value<R: Read>(
    reader: &mut PrefixReader<R>,
    header: ElementHeader,
    explicit_vr: bool,
    little_endian: bool,
    depth: usize,
    maximum_depth: usize,
) -> DcmResult<()> {
    if !header.has_undefined_length() {
        return reader.discard(header.length as u64);
    }
    if !matches!(header.vr, Vr::SQ | Vr::UN) {
        return invalid_file(format!(
            "undefined length is invalid for VR {} at ({:04X},{:04X})",
            header.vr.code(),
            header.tag.group,
            header.tag.element
        ));
    }
    skip_undefined_sequence(reader, explicit_vr, little_endian, depth + 1, maximum_depth)
}

fn skip_undefined_sequence<R: Read>(
    reader: &mut PrefixReader<R>,
    explicit_vr: bool,
    little_endian: bool,
    depth: usize,
    maximum_depth: usize,
) -> DcmResult<()> {
    require_depth(depth, maximum_depth)?;
    loop {
        let header = read_element_header(reader, explicit_vr, little_endian)?;
        if header.tag.is_sequence_delimitation() {
            return require_zero_delimiter_length(header);
        }
        if !header.tag.is_item() {
            return invalid_file("undefined-length sequence contains a non-item element");
        }
        if header.has_undefined_length() {
            skip_undefined_item(reader, explicit_vr, little_endian, depth + 1, maximum_depth)?;
        } else {
            reader.discard(header.length as u64)?;
        }
    }
}

fn skip_undefined_item<R: Read>(
    reader: &mut PrefixReader<R>,
    explicit_vr: bool,
    little_endian: bool,
    depth: usize,
    maximum_depth: usize,
) -> DcmResult<()> {
    require_depth(depth, maximum_depth)?;
    loop {
        let header = read_element_header(reader, explicit_vr, little_endian)?;
        if header.tag.is_item_delimitation() {
            return require_zero_delimiter_length(header);
        }
        if header.tag.is_delimiter() {
            return invalid_file("unexpected delimiter inside undefined-length item");
        }
        skip_element_value(
            reader,
            header,
            explicit_vr,
            little_endian,
            depth,
            maximum_depth,
        )?;
    }
}

fn require_depth(depth: usize, maximum_depth: usize) -> DcmResult<()> {
    if depth > maximum_depth {
        return Err(inspection_limit_error(
            Part10InspectionLimit::SequenceDepth,
            maximum_depth as u64,
        ));
    }
    Ok(())
}

fn require_zero_delimiter_length(header: ElementHeader) -> DcmResult<()> {
    if header.length != 0 {
        return invalid_file("sequence or item delimiter has a non-zero length");
    }
    Ok(())
}

fn read_u16<R: Read>(reader: &mut PrefixReader<R>, little_endian: bool) -> DcmResult<u16> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(if little_endian {
        u16::from_le_bytes(bytes)
    } else {
        u16::from_be_bytes(bytes)
    })
}

fn read_u32<R: Read>(reader: &mut PrefixReader<R>, little_endian: bool) -> DcmResult<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(if little_endian {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom};

    use dicom_toolkit_core::uid::{sop_class, transfer_syntax};

    use super::*;
    use crate::{DataSet, DicomWriter, Element, FileFormat};

    const SOP_INSTANCE_UID: &str = "1.2.826.0.1.3680043.10.543.1";
    const STUDY_INSTANCE_UID: &str = "1.2.826.0.1.3680043.10.543.2";
    const SERIES_INSTANCE_UID: &str = "1.2.826.0.1.3680043.10.543.3";

    fn test_file(transfer_syntax_uid: &str) -> FileFormat {
        let mut referenced_item = DataSet::new();
        referenced_item.set_uid(tags::REFERENCED_SOP_INSTANCE_UID, "1.2.3");

        let mut dataset = DataSet::new();
        dataset.set_uid(tags::SOP_CLASS_UID, sop_class::CT_IMAGE_STORAGE);
        dataset.set_uid(tags::SOP_INSTANCE_UID, SOP_INSTANCE_UID);
        dataset.insert(Element::sequence(
            tags::REFERENCED_SOP_SEQUENCE,
            vec![referenced_item],
        ));
        dataset.set_uid(tags::STUDY_INSTANCE_UID, STUDY_INSTANCE_UID);
        dataset.set_uid(tags::SERIES_INSTANCE_UID, SERIES_INSTANCE_UID);
        dataset.set_bytes(tags::PIXEL_DATA, Vr::OB, vec![7; 1024]);

        let mut file =
            FileFormat::from_dataset(sop_class::CT_IMAGE_STORAGE, SOP_INSTANCE_UID, dataset);
        file.meta.transfer_syntax_uid = transfer_syntax_uid.to_owned();
        file
    }

    fn encoded_file(transfer_syntax_uid: &str) -> Vec<u8> {
        let mut encoded = Vec::new();
        DicomWriter::new(&mut encoded)
            .write_file(&test_file(transfer_syntax_uid))
            .unwrap();
        encoded
    }

    #[test]
    fn reads_layout_and_identity_without_pixel_data() {
        let encoded = encoded_file(transfer_syntax::EXPLICIT_VR_LITTLE_ENDIAN);
        let mut reader = std::io::Cursor::new(encoded.clone());

        let index = read_part10_file_index(&mut reader, Part10ReadLimits::default()).unwrap();

        assert_eq!(
            index.layout.file_meta.media_storage_sop_instance_uid,
            SOP_INSTANCE_UID
        );
        assert_eq!(
            index.identity.study_instance_uid.as_str(),
            STUDY_INSTANCE_UID
        );
        assert_eq!(
            index.identity.series_instance_uid.as_str(),
            SERIES_INSTANCE_UID
        );
        assert!(reader.position() < encoded.len() as u64 - 1024);
    }

    #[test]
    fn reads_deflated_identity_with_the_same_bounded_api() {
        let encoded = encoded_file(transfer_syntax::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN);
        let mut reader = std::io::Cursor::new(encoded);

        let index = read_part10_file_index(&mut reader, Part10ReadLimits::default()).unwrap();

        assert_eq!(index.identity.sop_instance_uid.as_str(), SOP_INSTANCE_UID);
        assert_eq!(
            index.identity.study_instance_uid.as_str(),
            STUDY_INSTANCE_UID
        );
    }

    #[test]
    fn reads_implicit_little_endian_and_explicit_big_endian_identities() {
        for transfer_syntax_uid in [
            transfer_syntax::IMPLICIT_VR_LITTLE_ENDIAN,
            transfer_syntax::EXPLICIT_VR_BIG_ENDIAN,
        ] {
            let encoded = encoded_file(transfer_syntax_uid);
            let index = read_part10_file_index(
                &mut std::io::Cursor::new(encoded),
                Part10ReadLimits::default(),
            )
            .unwrap();

            assert_eq!(
                index.identity.sop_class_uid.as_str(),
                sop_class::CT_IMAGE_STORAGE
            );
            assert_eq!(
                index.identity.series_instance_uid.as_str(),
                SERIES_INSTANCE_UID
            );
        }
    }

    #[test]
    fn sparse_large_file_does_not_require_reading_its_payload() {
        const LARGE_FILE_LENGTH: u64 = 900 * 1024 * 1024;
        let mut temporary_file = tempfile::NamedTempFile::new().unwrap();
        DicomWriter::new(temporary_file.as_file_mut())
            .write_file(&test_file(transfer_syntax::EXPLICIT_VR_LITTLE_ENDIAN))
            .unwrap();
        temporary_file.as_file().set_len(LARGE_FILE_LENGTH).unwrap();
        temporary_file
            .as_file_mut()
            .seek(SeekFrom::Start(0))
            .unwrap();

        let index =
            read_part10_file_index(temporary_file.as_file_mut(), Part10ReadLimits::default())
                .unwrap();

        assert_eq!(
            index.layout.dataset_length,
            LARGE_FILE_LENGTH - index.layout.dataset_offset
        );
        assert!(temporary_file.as_file_mut().stream_position().unwrap() < 4096);
    }

    #[test]
    fn rejects_file_meta_and_dataset_identity_mismatch() {
        let mut file = test_file(transfer_syntax::EXPLICIT_VR_LITTLE_ENDIAN);
        file.meta.media_storage_sop_instance_uid = "1.2.3.999".to_owned();
        let mut encoded = Vec::new();
        DicomWriter::new(&mut encoded).write_file(&file).unwrap();

        let error = read_part10_file_index(
            &mut std::io::Cursor::new(encoded),
            Part10ReadLimits::default(),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("dataset SOP Instance UID does not match"));
    }

    #[test]
    fn enforces_configured_file_meta_and_prefix_limits() {
        let encoded = encoded_file(transfer_syntax::EXPLICIT_VR_LITTLE_ENDIAN);
        let meta_limited = Part10ReadLimits {
            maximum_file_meta_bytes: 1,
            ..Part10ReadLimits::default()
        };
        let meta_error = read_part10_file_layout(&mut std::io::Cursor::new(&encoded), meta_limited)
            .expect_err("the File Meta Information must exceed the configured limit");
        assert_eq!(
            part10_inspection_limit_from_error(&meta_error),
            Some(Part10InspectionLimit::FileMetaBytes)
        );

        let prefix_limited = Part10ReadLimits {
            maximum_dataset_prefix_bytes: 8,
            ..Part10ReadLimits::default()
        };
        let prefix_error =
            read_part10_file_index(&mut std::io::Cursor::new(encoded), prefix_limited)
                .expect_err("the identity must exceed the configured prefix limit");
        assert_eq!(
            part10_inspection_limit_from_error(&prefix_error),
            Some(Part10InspectionLimit::DatasetPrefixBytes)
        );
    }

    #[test]
    fn classifies_only_bounded_inspection_limits() {
        for limit in [
            Part10InspectionLimit::FileMetaBytes,
            Part10InspectionLimit::DatasetPrefixBytes,
            Part10InspectionLimit::SequenceDepth,
            Part10InspectionLimit::UidValueBytes,
        ] {
            let error = inspection_limit_error(limit, 1);
            assert_eq!(part10_inspection_limit_from_error(&error), Some(limit));
        }
        let malformed_error = DcmError::InvalidFile {
            reason: "missing DICM prefix at byte offset 128".to_string(),
        };
        assert_eq!(part10_inspection_limit_from_error(&malformed_error), None);
    }

    #[test]
    fn rejects_non_part10_input() {
        let mut input = std::io::Cursor::new(vec![0_u8; 256]);
        assert!(read_part10_file_layout(&mut input, Part10ReadLimits::default()).is_err());
    }
}
