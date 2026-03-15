//! dcmdump — dump DICOM file contents in human-readable form.
//!
//! Port of DCMTK's `dcmdump` utility.

use std::path::PathBuf;
use std::process;

use clap::Parser;

use dicom_toolkit_data::value::Value;
use dicom_toolkit_data::{DataSet, Element, FileFormat};
use dicom_toolkit_dict::{tags, Vr};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "dcmdump",
    about = "Dump DICOM file contents in human-readable form"
)]
struct Args {
    /// DICOM file(s) to dump
    #[arg(required = true, value_name = "FILE")]
    files: Vec<PathBuf>,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,

    /// Also print File Meta Information header
    #[arg(short = 'M', long)]
    meta: bool,

    /// Do not limit string value output length
    #[arg(short = 'n', long)]
    no_limit: bool,

    /// Output as DICOM JSON
    #[arg(long)]
    json: bool,

    /// Output as DICOM XML
    #[arg(long)]
    xml: bool,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Exit cleanly when stdout is closed (e.g. piped to `head`).
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        if msg.contains("Broken pipe") || msg.contains("os error 32") {
            process::exit(0);
        }
        eprintln!("{msg}");
    }));

    let args = Args::parse();
    let mut any_error = false;

    for path in &args.files {
        if args.files.len() > 1 {
            println!("\n# {}", path.display());
        }
        if let Err(e) = dump_file(path, &args) {
            eprintln!("Error: {}: {}", path.display(), e);
            any_error = true;
        }
    }

    if any_error {
        process::exit(1);
    }
}

fn dump_file(path: &PathBuf, args: &Args) -> dicom_toolkit_core::error::DcmResult<()> {
    let ff = FileFormat::open(path)?;
    let max_len: Option<usize> = if args.no_limit { None } else { Some(64) };

    if args.json {
        println!("{}", dicom_toolkit_data::json::to_json_pretty(&ff.dataset)?);
        return Ok(());
    }

    if args.xml {
        println!("{}", dicom_toolkit_data::xml::to_xml(&ff.dataset)?);
        return Ok(());
    }

    println!("# Dicom-File-Format");

    if args.meta {
        let ts_name = ts_display_name(&ff.meta.transfer_syntax_uid);
        println!("\n# Dicom-Meta-Information-Header");
        println!("# Used TransferSyntax: {}", ts_name);
        println!();
        let meta_ds = meta_as_dataset(&ff.meta);
        print_dataset(&meta_ds, 0, max_len);
    }

    println!("\n# Dicom-Dataset");
    println!();
    print_dataset(&ff.dataset, 0, max_len);

    Ok(())
}

// ── Printing ──────────────────────────────────────────────────────────────────

fn print_dataset(ds: &DataSet, indent: usize, max_len: Option<usize>) {
    for (tag, elem) in ds.iter() {
        let prefix = "  ".repeat(indent);

        if let Value::Sequence(items) = &elem.value {
            let tag_str = format!("({:04X},{:04X})", tag.group, tag.element);
            println!(
                "{}{} SQ (Sequence with {} items) # -1, 1",
                prefix,
                tag_str,
                items.len()
            );
            for (i, item) in items.iter().enumerate() {
                println!("{}  (Item #{})", prefix, i + 1);
                print_dataset(item, indent + 2, max_len);
            }
        } else {
            // Element::Display produces the dcmdump-style line
            let line = format!("{}", elem);
            let line = maybe_truncate_value(line, max_len);
            println!("{}{}", prefix, line);
        }
    }
}

/// If max_len is set, truncate the bracketed value portion to avoid very long lines.
fn maybe_truncate_value(line: String, max_len: Option<usize>) -> String {
    let limit = match max_len {
        None => return line,
        Some(n) => n,
    };
    // The value is between the first `[` and matching `]`
    if let (Some(open), Some(close)) = (line.find('['), line.rfind(']')) {
        if open < close {
            let val = &line[open + 1..close];
            if val.len() > limit {
                let truncated = &val[..limit];
                return format!("{}[{}...]{}", &line[..open], truncated, &line[close + 1..]);
            }
        }
    }
    line
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ts_display_name(uid: &str) -> &str {
    match uid.trim_end_matches('\0') {
        "1.2.840.10008.1.2" => "Implicit VR Little Endian",
        "1.2.840.10008.1.2.1" => "Explicit VR Little Endian",
        "1.2.840.10008.1.2.2" => "Explicit VR Big Endian",
        "1.2.840.10008.1.2.1.99" => "Deflated Explicit VR Little Endian",
        "1.2.840.10008.1.2.4.50" => "JPEG Baseline",
        "1.2.840.10008.1.2.4.70" => "JPEG Lossless",
        "1.2.840.10008.1.2.4.80" => "JPEG-LS Lossless",
        "1.2.840.10008.1.2.4.90" => "JPEG 2000 Lossless",
        other => other,
    }
}

fn meta_as_dataset(meta: &dicom_toolkit_data::meta_info::FileMetaInformation) -> DataSet {
    let mut ds = DataSet::new();
    ds.insert(Element::bytes(
        tags::FILE_META_INFORMATION_VERSION,
        Vr::OB,
        vec![0x00, 0x01],
    ));
    ds.insert(Element::uid(
        tags::MEDIA_STORAGE_SOP_CLASS_UID,
        &meta.media_storage_sop_class_uid,
    ));
    ds.insert(Element::uid(
        tags::MEDIA_STORAGE_SOP_INSTANCE_UID,
        &meta.media_storage_sop_instance_uid,
    ));
    ds.insert(Element::uid(
        tags::TRANSFER_SYNTAX_UID,
        &meta.transfer_syntax_uid,
    ));
    ds.insert(Element::uid(
        tags::IMPLEMENTATION_CLASS_UID,
        &meta.implementation_class_uid,
    ));
    ds.insert(Element::string(
        tags::IMPLEMENTATION_VERSION_NAME,
        Vr::SH,
        &meta.implementation_version_name,
    ));
    ds
}
