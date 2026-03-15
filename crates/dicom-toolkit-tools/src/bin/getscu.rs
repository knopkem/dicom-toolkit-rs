//! getscu — DICOM C-GET SCU.
//!
//! Port of DCMTK's `getscu` utility.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_core::uid::sop_class;
use dicom_toolkit_data::{DicomWriter, FileFormat, FileMetaInformation};
use dicom_toolkit_net::{c_get, Association, AssociationConfig, GetRequest, ReceivedInstance};
use dicom_toolkit_tools::query_retrieve::{
    accepted_transfer_syntax, build_query, decode_dataset_with_fallback, print_dataset,
    qr_get_contexts, select_accepted_context, TS_EXPLICIT_VR_LE,
};

#[derive(Parser)]
#[command(
    name = "getscu",
    about = "Send a DICOM C-GET retrieve request and save received instances"
)]
struct Args {
    /// SCP hostname or IP address
    host: String,

    /// SCP TCP port
    port: u16,

    /// Calling AE title
    #[arg(short = 'a', long, default_value = "GETSCU", value_name = "AE")]
    aetitle: String,

    /// Called AE title
    #[arg(short = 'c', long, default_value = "ANY-SCP", value_name = "AE")]
    called_ae: String,

    /// Directory to save retrieved DICOM Part 10 files
    #[arg(short = 'd', long, default_value = ".", value_name = "DIR")]
    output_dir: PathBuf,

    /// Query attribute key=value pair (e.g. "0020,000D=1.2.3")
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

    if let Err(e) = fs::create_dir_all(&args.output_dir) {
        eprintln!(
            "Cannot create output directory {}: {}",
            args.output_dir.display(),
            e
        );
        process::exit(1);
    }

    let query = match build_query(&args.key, &args.level) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("Invalid query key: {}", e);
            process::exit(1);
        }
    };

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

    let contexts = qr_get_contexts();
    let config = AssociationConfig {
        local_ae_title: args.aetitle.clone(),
        ..Default::default()
    };

    let mut assoc =
        match Association::request(&addr, &args.called_ae, &args.aetitle, &contexts, &config).await
        {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Association failed: {}", e);
                process::exit(1);
            }
        };

    let (ctx_id, sop_class_uid) = match select_accepted_context(
        &assoc,
        &[sop_class::STUDY_ROOT_QR_GET, sop_class::PATIENT_ROOT_QR_GET],
    ) {
        Some(selection) => selection,
        None => {
            eprintln!("No Q/R GET SOP class accepted by remote AE");
            let _ = assoc.abort().await;
            process::exit(1);
        }
    };

    if args.verbose {
        eprintln!("Sending C-GET with level {} ...", args.level);
    }

    let req = GetRequest {
        sop_class_uid: sop_class_uid.to_string(),
        query: query_bytes,
        context_id: ctx_id,
        priority: 0,
    };

    let result = match c_get(&mut assoc, req).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("C-GET failed: {}", e);
            let _ = assoc.abort().await;
            process::exit(1);
        }
    };

    let response_ts = accepted_transfer_syntax(&assoc, ctx_id).unwrap_or(TS_EXPLICIT_VR_LE);

    let mut any_error = false;
    for (idx, instance) in result.instances.iter().enumerate() {
        match save_instance(instance, &args.output_dir, idx) {
            Ok(path) => {
                println!("Saved {}", path.display());
            }
            Err(e) => {
                eprintln!(
                    "Failed to save retrieved instance {}: {}",
                    instance.sop_instance_uid, e
                );
                any_error = true;
            }
        }
    }

    for response in &result.responses {
        if args.verbose {
            eprintln!(
                "C-GET-RSP status=0x{:04X} remaining={:?} completed={:?} failed={:?} warning={:?}",
                response.status,
                response.remaining,
                response.completed,
                response.failed,
                response.warning
            );
        }

        if args.verbose {
            if let Some(dataset) = &response.dataset {
                match decode_dataset_with_fallback(dataset, response_ts) {
                    Ok(ds) => {
                        eprintln!("Response identifier dataset:");
                        print_dataset(&ds, 1);
                    }
                    Err(e) => {
                        eprintln!("Could not decode response identifier dataset: {e}");
                    }
                }
            }
        }
    }

    let final_status = result.responses.last().map(|rsp| rsp.status);
    if let Some(status) = final_status {
        println!(
            "Retrieved {} instance(s); final C-GET status 0x{:04X}",
            result.instances.len(),
            status
        );
        if status != 0x0000 {
            any_error = true;
        }
    } else {
        eprintln!("C-GET completed without any response datasets");
        any_error = true;
    }

    if let Err(e) = assoc.release().await {
        eprintln!("Release failed: {}", e);
        any_error = true;
    }

    if any_error {
        process::exit(1);
    }
}

fn save_instance(
    instance: &ReceivedInstance,
    output_dir: &Path,
    index: usize,
) -> DcmResult<PathBuf> {
    let transfer_syntax_uid = instance.transfer_syntax_uid.trim_end_matches('\0');
    let dataset = decode_dataset_with_fallback(&instance.dataset, transfer_syntax_uid)?;

    let meta = FileMetaInformation::new(
        instance.sop_class_uid.trim_end_matches('\0'),
        instance.sop_instance_uid.trim_end_matches('\0'),
        transfer_syntax_uid,
    );
    let ff = FileFormat::new(meta, dataset);
    let path = unique_output_path(
        output_dir,
        instance.sop_instance_uid.trim_end_matches('\0'),
        index,
    );
    ff.save(&path)?;
    Ok(path)
}

fn unique_output_path(output_dir: &Path, sop_instance_uid: &str, index: usize) -> PathBuf {
    let mut stem = sanitize_filename_component(sop_instance_uid);
    if stem.is_empty() {
        stem = format!("instance-{:04}", index + 1);
    }

    let mut candidate = output_dir.join(format!("{stem}.dcm"));
    let mut suffix = 1usize;
    while candidate.exists() {
        candidate = output_dir.join(format!("{stem}-{suffix}.dcm"));
        suffix += 1;
    }
    candidate
}

fn sanitize_filename_component(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
