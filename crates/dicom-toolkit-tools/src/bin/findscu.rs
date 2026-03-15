//! findscu — DICOM C-FIND SCU.
//!
//! Port of DCMTK's `findscu` utility.

use std::process;

use clap::Parser;

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{DataSet, DicomReader, DicomWriter};
use dicom_toolkit_data::value::Value;
use dicom_toolkit_dict::{Vr, tags};
use dicom_toolkit_net::{Association, AssociationConfig, FindRequest, PresentationContextRq, c_find};

// Study Root Query/Retrieve - FIND SOP class
const STUDY_ROOT_FIND: &str = "1.2.840.10008.5.1.4.1.2.2.1";
// Patient Root Query/Retrieve - FIND SOP class
const PATIENT_ROOT_FIND: &str = "1.2.840.10008.5.1.4.1.2.1.1";

const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const TS_IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";

#[derive(Parser)]
#[command(
    name = "findscu",
    about = "Send a DICOM C-FIND query to a remote Query/Retrieve SCP",
)]
struct Args {
    /// SCP hostname or IP address
    host: String,

    /// SCP TCP port
    port: u16,

    /// Calling AE title
    #[arg(short = 'a', long, default_value = "FINDSCU", value_name = "AE")]
    aetitle: String,

    /// Called AE title
    #[arg(short = 'c', long, default_value = "ANY-SCP", value_name = "AE")]
    called_ae: String,

    /// Query attribute key=value pair (e.g. "0010,0010=Smith*")
    #[arg(short = 'k', long, value_name = "TAG=VALUE")]
    key: Vec<String>,

    /// Query/retrieve level: PATIENT, STUDY, SERIES, or IMAGE
    #[arg(short = 'L', long, default_value = "STUDY", value_name = "LEVEL")]
    level: String,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Build query dataset from -k key=value pairs
    let query = match build_query(&args.key, &args.level) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("Invalid query key: {}", e);
            process::exit(1);
        }
    };

    // Encode query dataset to bytes
    let mut query_bytes = Vec::new();
    if let Err(e) = DicomWriter::new(&mut query_bytes).write_dataset(&query, TS_EXPLICIT_VR_LE) {
        eprintln!("Failed to encode query: {}", e);
        process::exit(1);
    }

    let addr = format!("{}:{}", args.host, args.port);
    if args.verbose {
        eprintln!(
            "Connecting to {} (called AE: {}, calling AE: {})",
            addr, args.called_ae, args.aetitle
        );
    }

    let contexts = vec![
        PresentationContextRq {
            id: 1,
            abstract_syntax: STUDY_ROOT_FIND.to_string(),
            transfer_syntaxes: vec![
                TS_EXPLICIT_VR_LE.to_string(),
                TS_IMPLICIT_VR_LE.to_string(),
            ],
        },
        PresentationContextRq {
            id: 3,
            abstract_syntax: PATIENT_ROOT_FIND.to_string(),
            transfer_syntaxes: vec![
                TS_EXPLICIT_VR_LE.to_string(),
                TS_IMPLICIT_VR_LE.to_string(),
            ],
        },
    ];

    let mut config = AssociationConfig::default();
    config.local_ae_title = args.aetitle.clone();

    let mut assoc = match Association::request(
        &addr,
        &args.called_ae,
        &args.aetitle,
        &contexts,
        &config,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Association failed: {}", e);
            process::exit(1);
        }
    };

    // Prefer Study Root; fall back to Patient Root
    let ctx_id = match assoc
        .find_context(STUDY_ROOT_FIND)
        .filter(|pc| pc.result.is_accepted())
        .or_else(|| {
            assoc
                .find_context(PATIENT_ROOT_FIND)
                .filter(|pc| pc.result.is_accepted())
        }) {
        Some(pc) => pc.id,
        None => {
            eprintln!("No Q/R FIND SOP class accepted by remote AE");
            let _ = assoc.abort().await;
            process::exit(1);
        }
    };

    if args.verbose {
        eprintln!("Sending C-FIND with level {} ...", args.level);
    }

    let req = FindRequest {
        sop_class_uid: STUDY_ROOT_FIND.to_string(),
        query: query_bytes,
        context_id: ctx_id,
        priority: 0,
    };

    let results = match c_find(&mut assoc, req).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("C-FIND failed: {}", e);
            let _ = assoc.abort().await;
            process::exit(1);
        }
    };

    println!("Found {} result(s):", results.len());
    for (i, result_bytes) in results.iter().enumerate() {
        println!("\n# Result #{}", i + 1);
        match DicomReader::new(result_bytes.as_slice()).read_dataset(TS_EXPLICIT_VR_LE) {
            Ok(ds) => print_dataset(&ds, 0),
            Err(_) => {
                // Try Implicit VR LE fallback
                if let Ok(ds) =
                    DicomReader::new(result_bytes.as_slice()).read_dataset(TS_IMPLICIT_VR_LE)
                {
                    print_dataset(&ds, 0);
                } else {
                    eprintln!("  (could not decode result dataset)");
                }
            }
        }
    }

    if let Err(e) = assoc.release().await {
        eprintln!("Release failed: {}", e);
    }
}

// ── Query building ────────────────────────────────────────────────────────────

fn build_query(keys: &[String], level: &str) -> DcmResult<DataSet> {
    let mut ds = DataSet::new();

    // Set QueryRetrieveLevel
    ds.set_string(tags::QUERY_RETRIEVE_LEVEL, Vr::CS, level);

    for kv in keys {
        let (tag_str, value) = if let Some(pos) = kv.find('=') {
            (kv[..pos].trim(), &kv[pos + 1..])
        } else {
            (kv.trim(), "")
        };

        let tag = parse_tag(tag_str).map_err(|e| dicom_toolkit_core::error::DcmError::Other(e))?;
        // Use a generic string VR — the SCP will interpret based on the tag
        ds.set_string(tag, Vr::LO, value);
    }

    Ok(ds)
}

/// Parse a tag string like "0010,0010" or "00100010".
fn parse_tag(s: &str) -> Result<dicom_toolkit_dict::Tag, String> {
    let s = s.trim_matches(|c| c == '(' || c == ')');
    let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() == 8 {
        let group = u16::from_str_radix(&clean[..4], 16)
            .map_err(|_| format!("invalid tag: {}", s))?;
        let element = u16::from_str_radix(&clean[4..], 16)
            .map_err(|_| format!("invalid tag: {}", s))?;
        Ok(dicom_toolkit_dict::Tag::new(group, element))
    } else {
        Err(format!("invalid tag format: {}", s))
    }
}

// ── Printing ──────────────────────────────────────────────────────────────────

fn print_dataset(ds: &DataSet, indent: usize) {
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
