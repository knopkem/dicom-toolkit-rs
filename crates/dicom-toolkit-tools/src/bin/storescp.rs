//! storescp — DICOM C-STORE SCP.
//!
//! Port of DCMTK's `storescp` utility.

use std::path::PathBuf;
use std::process;
use std::sync::Arc;

use clap::Parser;
use tokio::net::TcpListener;

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{DataSet, DicomReader, FileFormat};
use dicom_toolkit_dict::tags;
use dicom_toolkit_net::{Association, AssociationConfig};

#[derive(Parser)]
#[command(
    name = "storescp",
    about = "Listen for incoming DICOM C-STORE requests and save received files",
)]
struct Args {
    /// TCP port to listen on
    port: u16,

    /// Called AE title
    #[arg(short = 'a', long, default_value = "STORESCP", value_name = "AE")]
    aetitle: String,

    /// Directory to save received DICOM files
    #[arg(short = 'd', long, default_value = ".", value_name = "DIR")]
    output_dir: PathBuf,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if !args.output_dir.is_dir() {
        eprintln!(
            "Output directory does not exist: {}",
            args.output_dir.display()
        );
        process::exit(1);
    }

    let listener = match TcpListener::bind(("0.0.0.0", args.port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Cannot bind to port {}: {}", args.port, e);
            process::exit(1);
        }
    };

    println!("Listening on port {} (AE: {})", args.port, args.aetitle);

    let output_dir = Arc::new(args.output_dir.clone());
    let verbose = args.verbose;

    let mut config = AssociationConfig::default();
    config.local_ae_title = args.aetitle.clone();
    config.accept_all_transfer_syntaxes = true;
    // Empty accepted_abstract_syntaxes means accept all SOP classes
    let config = Arc::new(config);

    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Accept error: {}", e);
                continue;
            }
        };

        if verbose {
            eprintln!("Connection from {}", peer_addr);
        }

        let output_dir = Arc::clone(&output_dir);
        let config = Arc::clone(&config);

        tokio::spawn(async move {
            match handle_connection(stream, &output_dir, &config, verbose).await {
                Ok(n) => {
                    if verbose {
                        eprintln!("{}: handled {} C-STORE(s)", peer_addr, n);
                    }
                }
                Err(e) => {
                    eprintln!("{}: error: {}", peer_addr, e);
                }
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    output_dir: &PathBuf,
    config: &AssociationConfig,
    verbose: bool,
) -> DcmResult<u32> {
    let mut assoc = Association::accept(stream, config).await?;
    let mut stored = 0u32;

    loop {
        let (ctx_id, cmd) = match assoc.recv_dimse_command().await {
            Ok(c) => c,
            Err(_) => break, // peer released or aborted
        };

        let command_field = cmd.get_u16(tags::COMMAND_FIELD).unwrap_or(0);

        match command_field {
            0x0001 => {
                // C-STORE-RQ
                let sop_class = cmd
                    .get_string(tags::AFFECTED_SOP_CLASS_UID)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .to_string();
                let sop_instance = cmd
                    .get_string(tags::AFFECTED_SOP_INSTANCE_UID)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    .to_string();
                let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

                let data = assoc.recv_dimse_data().await?;

                // Decode dataset and wrap in a DICOM Part 10 file
                let ts_uid = assoc
                    .context_by_id(ctx_id)
                    .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
                    .unwrap_or_else(|| "1.2.840.10008.1.2.1".to_string());

                let dataset =
                    DicomReader::new(data.as_slice()).read_dataset(&ts_uid).unwrap_or_else(|_| {
                        // Fall back to empty dataset on decode error
                        DataSet::new()
                    });

                let ff = FileFormat::from_dataset(&sop_class, &sop_instance, dataset);

                let safe_instance = sop_instance
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == '.' { c } else { '_' })
                    .collect::<String>();
                let filename = format!("{}.dcm", safe_instance);
                let dest = output_dir.join(&filename);

                match ff.save(&dest) {
                    Ok(()) => {
                        if verbose {
                            eprintln!("Saved {}", dest.display());
                        }
                        stored += 1;
                    }
                    Err(e) => {
                        eprintln!("Failed to save {}: {}", dest.display(), e);
                    }
                }

                // Send C-STORE-RSP
                let mut rsp = DataSet::new();
                rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
                rsp.set_u16(tags::COMMAND_FIELD, 0x8001); // C-STORE-RSP
                rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
                rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
                rsp.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &sop_instance);
                rsp.set_u16(tags::STATUS, 0x0000); // success

                assoc.send_dimse_command(ctx_id, &rsp).await?;
            }
            0x0030 => {
                // C-ECHO-RQ — respond with success so verification tools work
                let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);
                let sop_class = cmd
                    .get_string(tags::AFFECTED_SOP_CLASS_UID)
                    .unwrap_or("1.2.840.10008.1.1")
                    .trim_end_matches('\0')
                    .to_string();
                if verbose {
                    eprintln!("C-ECHO-RQ from peer");
                }
                let mut rsp = DataSet::new();
                rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
                rsp.set_u16(tags::COMMAND_FIELD, 0x8030); // C-ECHO-RSP
                rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
                rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
                rsp.set_u16(tags::STATUS, 0x0000); // success
                assoc.send_dimse_command(ctx_id, &rsp).await?;
            }
            _ => {
                // Unrecognised command — end of association
                break;
            }
        }
    }

    let _ = assoc.release().await;
    Ok(stored)
}
