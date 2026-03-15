//! Shared query/retrieve CLI helpers.

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_core::uid::{sop_class, transfer_syntax};
use dicom_toolkit_data::value::Value;
use dicom_toolkit_data::{DataSet, DicomReader};
use dicom_toolkit_dict::{tags, Tag, Vr};
use dicom_toolkit_net::{Association, PresentationContextRq};

pub const TS_EXPLICIT_VR_LE: &str = transfer_syntax::EXPLICIT_VR_LITTLE_ENDIAN;
pub const TS_IMPLICIT_VR_LE: &str = transfer_syntax::IMPLICIT_VR_LITTLE_ENDIAN;

const STUDY_ROOT_FIND: &str = sop_class::STUDY_ROOT_QR_FIND;
const PATIENT_ROOT_FIND: &str = sop_class::PATIENT_ROOT_QR_FIND;
const STUDY_ROOT_GET: &str = sop_class::STUDY_ROOT_QR_GET;
const PATIENT_ROOT_GET: &str = sop_class::PATIENT_ROOT_QR_GET;

const STORAGE_SOP_CLASSES: &[&str] = &[
    sop_class::CT_IMAGE_STORAGE,
    sop_class::ENHANCED_CT_IMAGE_STORAGE,
    sop_class::MR_IMAGE_STORAGE,
    sop_class::ENHANCED_MR_IMAGE_STORAGE,
    sop_class::ULTRASOUND_IMAGE_STORAGE,
    sop_class::SECONDARY_CAPTURE_IMAGE_STORAGE,
    sop_class::DIGITAL_XRAY_IMAGE_STORAGE_FOR_PRESENTATION,
    sop_class::DIGITAL_XRAY_IMAGE_STORAGE_FOR_PROCESSING,
    sop_class::DIGITAL_MAMMOGRAPHY_IMAGE_STORAGE_FOR_PRESENTATION,
    sop_class::CR_IMAGE_STORAGE,
    sop_class::NM_IMAGE_STORAGE,
    sop_class::PET_IMAGE_STORAGE,
    sop_class::RT_IMAGE_STORAGE,
    sop_class::RT_DOSE_STORAGE,
    sop_class::RT_STRUCTURE_SET_STORAGE,
    sop_class::RT_PLAN_STORAGE,
    sop_class::XA_IMAGE_STORAGE,
    sop_class::VL_PHOTOGRAPHIC_IMAGE_STORAGE,
    sop_class::VIDEO_ENDOSCOPIC_IMAGE_STORAGE,
    sop_class::ENCAPSULATED_PDF_STORAGE,
    sop_class::ENCAPSULATED_CDA_STORAGE,
    sop_class::BASIC_TEXT_SR_STORAGE,
    sop_class::ENHANCED_SR_STORAGE,
    sop_class::COMPREHENSIVE_SR_STORAGE,
    sop_class::SEGMENTATION_STORAGE,
];

const STORE_TRANSFER_SYNTAXES: &[&str] = &[
    transfer_syntax::EXPLICIT_VR_LITTLE_ENDIAN,
    transfer_syntax::IMPLICIT_VR_LITTLE_ENDIAN,
    transfer_syntax::ENCAPSULATED_UNCOMPRESSED,
    transfer_syntax::RLE_LOSSLESS,
    transfer_syntax::JPEG_BASELINE,
    transfer_syntax::JPEG_EXTENDED,
    transfer_syntax::JPEG_LOSSLESS_NON_HIERARCHICAL,
    transfer_syntax::JPEG_LOSSLESS_NON_HIERARCHICAL_FIRST_ORDER,
    transfer_syntax::JPEG_LS_LOSSLESS,
    transfer_syntax::JPEG_LS_LOSSY,
    transfer_syntax::JPEG_2000_LOSSLESS,
    transfer_syntax::JPEG_2000,
];

pub fn qr_find_contexts() -> Vec<PresentationContextRq> {
    vec![
        PresentationContextRq {
            id: 1,
            abstract_syntax: STUDY_ROOT_FIND.to_string(),
            transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string(), TS_IMPLICIT_VR_LE.to_string()],
        },
        PresentationContextRq {
            id: 3,
            abstract_syntax: PATIENT_ROOT_FIND.to_string(),
            transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string(), TS_IMPLICIT_VR_LE.to_string()],
        },
    ]
}

pub fn qr_get_contexts() -> Vec<PresentationContextRq> {
    let mut contexts = vec![
        PresentationContextRq {
            id: 1,
            abstract_syntax: STUDY_ROOT_GET.to_string(),
            transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string(), TS_IMPLICIT_VR_LE.to_string()],
        },
        PresentationContextRq {
            id: 3,
            abstract_syntax: PATIENT_ROOT_GET.to_string(),
            transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string(), TS_IMPLICIT_VR_LE.to_string()],
        },
    ];

    let mut next_id: u8 = 5;
    for sop_class_uid in STORAGE_SOP_CLASSES {
        contexts.push(PresentationContextRq {
            id: next_id,
            abstract_syntax: (*sop_class_uid).to_string(),
            transfer_syntaxes: STORE_TRANSFER_SYNTAXES
                .iter()
                .map(|ts| (*ts).to_string())
                .collect(),
        });
        next_id = next_id.saturating_add(2);
    }

    contexts
}

pub fn select_accepted_context(
    assoc: &Association,
    preferred_sop_classes: &[&'static str],
) -> Option<(u8, &'static str)> {
    preferred_sop_classes.iter().find_map(|&sop_class_uid| {
        assoc
            .find_context(sop_class_uid)
            .map(|pc| (pc.id, sop_class_uid))
    })
}

pub fn accepted_transfer_syntax(assoc: &Association, context_id: u8) -> Option<&str> {
    assoc
        .context_by_id(context_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0'))
}

pub fn decode_dataset_with_fallback(bytes: &[u8], primary_ts: &str) -> DcmResult<DataSet> {
    DicomReader::new(bytes)
        .read_dataset(primary_ts)
        .or_else(|primary_err| {
            let fallback_ts = if primary_ts == TS_IMPLICIT_VR_LE {
                TS_EXPLICIT_VR_LE
            } else {
                TS_IMPLICIT_VR_LE
            };
            DicomReader::new(bytes)
                .read_dataset(fallback_ts)
                .map_err(|_| primary_err)
        })
}

pub fn build_query(keys: &[String], level: &str) -> DcmResult<DataSet> {
    let mut ds = DataSet::new();
    ds.set_string(tags::QUERY_RETRIEVE_LEVEL, Vr::CS, level);

    for kv in keys {
        let (tag_str, value) = if let Some(pos) = kv.find('=') {
            (kv[..pos].trim(), &kv[pos + 1..])
        } else {
            (kv.trim(), "")
        };

        let tag = parse_tag(tag_str).map_err(DcmError::Other)?;
        ds.set_string(tag, Vr::LO, value);
    }

    Ok(ds)
}

pub fn parse_tag(s: &str) -> Result<Tag, String> {
    let s = s.trim_matches(|c| c == '(' || c == ')');
    let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() == 8 {
        let group =
            u16::from_str_radix(&clean[..4], 16).map_err(|_| format!("invalid tag: {s}"))?;
        let element =
            u16::from_str_radix(&clean[4..], 16).map_err(|_| format!("invalid tag: {s}"))?;
        Ok(Tag::new(group, element))
    } else {
        Err(format!("invalid tag format: {s}"))
    }
}

pub fn print_dataset(ds: &DataSet, indent: usize) {
    let prefix = "  ".repeat(indent);
    for (tag, elem) in ds.iter() {
        let tag_str = format!("({:04X},{:04X})", tag.group, tag.element);
        if let Value::Sequence(items) = &elem.value {
            println!(
                "{}{} SQ (Sequence with {} items) # -1, 1",
                prefix,
                tag_str,
                items.len()
            );
            for item in items.iter() {
                print_dataset(item, indent + 1);
            }
        } else {
            println!("{}{}", prefix, elem);
        }
    }
}
