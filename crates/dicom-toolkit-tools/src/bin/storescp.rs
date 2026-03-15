//! storescp — DICOM C-STORE SCP.
//!
//! Port of DCMTK's `storescp` utility.
//!
//! This tool uses the [`DicomServer`](dicom_toolkit_net::server::DicomServer)
//! framework with a [`FileStoreProvider`](dicom_toolkit_net::server::FileStoreProvider)
//! that writes received instances as `.dcm` files.

use std::path::PathBuf;
use std::process;

use clap::Parser;

use dicom_toolkit_net::server::{DicomServer, FileStoreProvider};

#[derive(Parser)]
#[command(
    name = "storescp",
    about = "Listen for incoming DICOM C-STORE requests and save received files"
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

    if args.verbose {
        tracing_subscriber::fmt::init();
    }

    let server = match DicomServer::builder()
        .ae_title(&args.aetitle)
        .port(args.port)
        .store_provider(FileStoreProvider::new(&args.output_dir))
        .build()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cannot start server on port {}: {}", args.port, e);
            process::exit(1);
        }
    };

    println!("Listening on port {} (AE: {})", args.port, args.aetitle);

    if let Err(e) = server.run().await {
        eprintln!("Server error: {}", e);
        process::exit(1);
    }
}
