//! storescu — DICOM C-STORE SCU.
//!
//! Port of DCMTK's `storescu` utility.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process;

use clap::Parser;

use dicom_toolkit_data::{DicomWriter, FileFormat};
use dicom_toolkit_dict::tags;
use dicom_toolkit_net::{Association, AssociationConfig, PresentationContextRq, StoreRequest, c_store};

const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const TS_IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";

#[derive(Parser)]
#[command(
    name = "storescu",
    about = "Send DICOM SOP instances to a remote Storage SCP",
)]
struct Args {
    /// SCP hostname or IP address
    host: String,

    /// SCP TCP port
    port: u16,

    /// DICOM file(s) to send
    #[arg(required = true, value_name = "FILE")]
    files: Vec<PathBuf>,

    /// Calling AE title
    #[arg(short = 'a', long, default_value = "STORESCU", value_name = "AE")]
    aetitle: String,

    /// Called AE title
    #[arg(short = 'c', long, default_value = "ANY-SCP", value_name = "AE")]
    called_ae: String,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // --- Read all DICOM files up front ---
    let mut file_infos: Vec<(FileFormat, Vec<u8>)> = Vec::new();
    for path in &args.files {
        let ff = match FileFormat::open(path) {
            Ok(ff) => ff,
            Err(e) => {
                eprintln!("Cannot read {}: {}", path.display(), e);
                process::exit(1);
            }
        };
        // Encode dataset in Explicit VR LE for transfer
        let mut dataset_bytes = Vec::new();
        if let Err(e) = DicomWriter::new(&mut dataset_bytes)
            .write_dataset(&ff.dataset, TS_EXPLICIT_VR_LE)
        {
            eprintln!("Cannot encode {}: {}", path.display(), e);
            process::exit(1);
        }
        file_infos.push((ff, dataset_bytes));
    }

    // --- Build presentation contexts (one per unique SOP class) ---
    let mut sop_class_to_ctx_id: HashMap<String, u8> = HashMap::new();
    let mut contexts: Vec<PresentationContextRq> = Vec::new();
    let mut next_id: u8 = 1;

    for (ff, _) in &file_infos {
        let sop_class = ff
            .dataset
            .get_string(tags::SOP_CLASS_UID)
            .or_else(|| ff.meta.media_storage_sop_class_uid.as_str().into())
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        if sop_class.is_empty() {
            eprintln!("Warning: SOP Class UID missing in a file; skipping context");
            continue;
        }

        if !sop_class_to_ctx_id.contains_key(&sop_class) {
            sop_class_to_ctx_id.insert(sop_class.clone(), next_id);
            contexts.push(PresentationContextRq {
                id: next_id,
                abstract_syntax: sop_class,
                transfer_syntaxes: vec![
                    TS_EXPLICIT_VR_LE.to_string(),
                    TS_IMPLICIT_VR_LE.to_string(),
                ],
            });
            next_id = next_id.saturating_add(2);
            if next_id == 0 {
                // wrapped past 255
                eprintln!("Too many unique SOP classes (max 128)");
                process::exit(1);
            }
        }
    }

    if contexts.is_empty() {
        eprintln!("No valid DICOM files to send");
        process::exit(1);
    }

    let addr = format!("{}:{}", args.host, args.port);
    if args.verbose {
        eprintln!(
            "Connecting to {} (called AE: {}, calling AE: {})",
            addr, args.called_ae, args.aetitle
        );
    }

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

    // --- Send each file ---
    let mut any_error = false;
    for (idx, (ff, dataset_bytes)) in file_infos.into_iter().enumerate() {
        let path = &args.files[idx];

        let sop_class = ff
            .dataset
            .get_string(tags::SOP_CLASS_UID)
            .or_else(|| Some(ff.meta.media_storage_sop_class_uid.as_str()))
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        let sop_instance = ff
            .dataset
            .get_string(tags::SOP_INSTANCE_UID)
            .or_else(|| Some(ff.meta.media_storage_sop_instance_uid.as_str()))
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        let ctx_id = match sop_class_to_ctx_id.get(&sop_class) {
            Some(&id) => id,
            None => {
                eprintln!("{}: no presentation context for SOP class {}", path.display(), sop_class);
                any_error = true;
                continue;
            }
        };

        // Verify the context was accepted
        match assoc.context_by_id(ctx_id) {
            Some(pc) if pc.result.is_accepted() => {}
            _ => {
                eprintln!(
                    "{}: presentation context for {} was rejected",
                    path.display(),
                    sop_class
                );
                any_error = true;
                continue;
            }
        }

        if args.verbose {
            eprintln!("Sending {} ...", path.display());
        }

        let req = StoreRequest {
            sop_class_uid: sop_class.clone(),
            sop_instance_uid: sop_instance.clone(),
            priority: 0,
            dataset_bytes,
            context_id: ctx_id,
        };

        match c_store(&mut assoc, req).await {
            Ok(rsp) if rsp.status == 0x0000 => {
                println!("{}: OK", path.display());
            }
            Ok(rsp) => {
                eprintln!("{}: warning: SCP returned status 0x{:04X}", path.display(), rsp.status);
            }
            Err(e) => {
                eprintln!("{}: C-STORE failed: {}", path.display(), e);
                any_error = true;
            }
        }
    }

    if let Err(e) = assoc.release().await {
        eprintln!("Release failed: {}", e);
    }

    if any_error {
        process::exit(1);
    }
}
