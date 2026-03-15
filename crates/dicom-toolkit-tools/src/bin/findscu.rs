//! findscu — DICOM C-FIND SCU.
//!
//! Port of DCMTK's `findscu` utility.

use std::process;

use clap::Parser;

use dicom_toolkit_data::DicomWriter;
use dicom_toolkit_net::{c_find, Association, AssociationConfig, FindRequest};
use dicom_toolkit_tools::query_retrieve::{
    accepted_transfer_syntax, build_query, decode_dataset_with_fallback, print_dataset,
    qr_find_contexts, select_accepted_context, TS_EXPLICIT_VR_LE,
};

#[derive(Parser)]
#[command(
    name = "findscu",
    about = "Send a DICOM C-FIND query to a remote Query/Retrieve SCP"
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

    let contexts = qr_find_contexts();

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

    // Prefer Study Root; fall back to Patient Root
    let (ctx_id, sop_class_uid) = match select_accepted_context(
        &assoc,
        &[
            dicom_toolkit_core::uid::sop_class::STUDY_ROOT_QR_FIND,
            dicom_toolkit_core::uid::sop_class::PATIENT_ROOT_QR_FIND,
        ],
    ) {
        Some(selection) => selection,
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
        sop_class_uid: sop_class_uid.to_string(),
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

    let response_ts = accepted_transfer_syntax(&assoc, ctx_id).unwrap_or(TS_EXPLICIT_VR_LE);

    println!("Found {} result(s):", results.len());
    for (i, result_bytes) in results.iter().enumerate() {
        println!("\n# Result #{}", i + 1);
        match decode_dataset_with_fallback(result_bytes.as_slice(), response_ts) {
            Ok(ds) => print_dataset(&ds, 0),
            Err(_) => eprintln!("  (could not decode result dataset)"),
        }
    }

    if let Err(e) = assoc.release().await {
        eprintln!("Release failed: {}", e);
    }
}
