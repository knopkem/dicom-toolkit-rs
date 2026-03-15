//! echoscu — DICOM C-ECHO verification SCU.
//!
//! Port of DCMTK's `echoscu` utility.

use std::process;

use clap::Parser;

use dicom_toolkit_net::{c_echo, Association, AssociationConfig, PresentationContextRq};

const VERIFICATION_SOP_CLASS: &str = "1.2.840.10008.1.1";
const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const TS_IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";

#[derive(Parser)]
#[command(
    name = "echoscu",
    about = "Send DICOM C-ECHO verification request to a remote Application Entity"
)]
struct Args {
    /// SCP hostname or IP address
    host: String,

    /// SCP TCP port
    port: u16,

    /// Calling AE title
    #[arg(short = 'a', long, default_value = "ECHOSCU", value_name = "AE")]
    aetitle: String,

    /// Called AE title
    #[arg(short = 'c', long, default_value = "ANY-SCP", value_name = "AE")]
    called_ae: String,

    /// Number of C-ECHO requests to send
    #[arg(short = 'r', long, default_value = "1", value_name = "N")]
    repeat: u32,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let addr = format!("{}:{}", args.host, args.port);

    if args.verbose {
        eprintln!(
            "Connecting to {} (called AE: {}, calling AE: {})",
            addr, args.called_ae, args.aetitle
        );
    }

    let contexts = vec![PresentationContextRq {
        id: 1,
        abstract_syntax: VERIFICATION_SOP_CLASS.to_string(),
        transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string(), TS_IMPLICIT_VR_LE.to_string()],
    }];

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

    let ctx_id = match assoc.find_context(VERIFICATION_SOP_CLASS) {
        Some(pc) if pc.result.is_accepted() => pc.id,
        _ => {
            eprintln!("Verification SOP Class not accepted by remote AE");
            let _ = assoc.abort().await;
            process::exit(1);
        }
    };

    for i in 1..=args.repeat {
        if args.verbose {
            eprintln!("Sending C-ECHO #{} ...", i);
        }
        match c_echo(&mut assoc, ctx_id).await {
            Ok(()) => println!(
                "Sent C-ECHO to {}:{}, received response: OK",
                args.host, args.port
            ),
            Err(e) => {
                eprintln!("C-ECHO #{} failed: {}", i, e);
                let _ = assoc.abort().await;
                process::exit(1);
            }
        }
    }

    if let Err(e) = assoc.release().await {
        eprintln!("Release failed: {}", e);
        process::exit(1);
    }
}
