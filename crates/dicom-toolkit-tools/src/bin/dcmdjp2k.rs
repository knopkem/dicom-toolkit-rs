//! dcmdjp2k — Decompress JPEG 2000 encoded DICOM file.
//!
//! Port of DCMTK's `dcmdjp2k` utility.

use std::path::PathBuf;
use std::process;

use clap::Parser;

use dicom_toolkit_codec::jp2k::decoder::decode_jp2k;
use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::FileFormat;
use dicom_toolkit_dict::tags;

const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const TS_JPEG2000_LOSSLESS: &str = "1.2.840.10008.1.2.4.90";
const TS_JPEG2000: &str = "1.2.840.10008.1.2.4.91";

#[derive(Parser)]
#[command(
    name = "dcmdjp2k",
    about = "Decode JPEG 2000 compressed DICOM file",
    long_about = "Reads a DICOM file compressed with JPEG 2000 transfer syntax and\n\
                  decompresses the pixel data. Writes a DICOM Part 10 file using\n\
                  Explicit VR Little Endian transfer syntax."
)]
struct Args {
    /// Input DICOM file (JPEG 2000 compressed)
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Output DICOM file (uncompressed)
    #[arg(value_name = "OUTPUT")]
    output: PathBuf,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();

    let ff = match FileFormat::open(&args.input) {
        Ok(ff) => ff,
        Err(e) => {
            eprintln!("Error reading {}: {e}", args.input.display());
            process::exit(1);
        }
    };

    let ts = &ff.meta.transfer_syntax_uid;
    if ts != TS_JPEG2000_LOSSLESS && ts != TS_JPEG2000 {
        eprintln!(
            "Warning: input transfer syntax is not JPEG 2000 ({ts}), attempting decode anyway"
        );
    }

    let ds = &ff.dataset;
    let rows = ds.get_u16(tags::ROWS).unwrap_or(0);
    let cols = ds.get_u16(tags::COLUMNS).unwrap_or(0);
    let bits_allocated = ds.get_u16(tags::BITS_ALLOCATED).unwrap_or(8);
    let bits_stored = ds.get_u16(tags::BITS_STORED).unwrap_or(bits_allocated);
    let samples_per_pixel = ds.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1);

    if args.verbose {
        eprintln!(
            "Input: {}x{}, {} bit ({} stored), {} component(s), TS: {ts}",
            cols, rows, bits_allocated, bits_stored, samples_per_pixel
        );
    }

    let fragments = match ds.get(tags::PIXEL_DATA) {
        Some(elem) => match &elem.value {
            Value::PixelData(PixelData::Encapsulated { fragments, .. }) => fragments.clone(),
            Value::PixelData(PixelData::Native { .. }) => {
                if args.verbose {
                    eprintln!("Pixel data is already native (uncompressed), copying as-is");
                }
                let mut out_ff = ff.clone();
                out_ff.meta.transfer_syntax_uid = TS_EXPLICIT_VR_LE.to_string();
                match out_ff.save(&args.output) {
                    Ok(()) => {
                        eprintln!("Written: {} (already uncompressed)", args.output.display());
                    }
                    Err(e) => {
                        eprintln!("Error writing {}: {e}", args.output.display());
                        process::exit(1);
                    }
                }
                return;
            }
            _ => {
                eprintln!("Error: unexpected pixel data value type");
                process::exit(1);
            }
        },
        None => {
            eprintln!("Error: no pixel data (7FE0,0010) in input file");
            process::exit(1);
        }
    };

    if args.verbose {
        let total_bytes: usize = fragments.iter().map(|f| f.len()).sum();
        eprintln!(
            "Compressed pixel data: {} fragment(s), {} bytes total",
            fragments.len(),
            total_bytes
        );
    }

    let mut all_pixels = Vec::new();
    for (i, fragment) in fragments.iter().enumerate() {
        if fragment.is_empty() {
            continue;
        }
        match decode_jp2k(fragment) {
            Ok(decoded) => {
                if args.verbose && i == 0 {
                    eprintln!(
                        "Decoded frame: {}x{}, {} bit, {} component(s), {} bytes",
                        decoded.width,
                        decoded.height,
                        decoded.bits_per_sample,
                        decoded.components,
                        decoded.pixels.len()
                    );
                }
                all_pixels.extend_from_slice(&decoded.pixels);
            }
            Err(e) => {
                eprintln!("Error decoding JPEG 2000 fragment {i}: {e}");
                process::exit(1);
            }
        }
    }

    if args.verbose {
        eprintln!("Decompressed pixel data: {} bytes", all_pixels.len());
    }

    let mut out_ff = ff.clone();
    out_ff.meta.transfer_syntax_uid = TS_EXPLICIT_VR_LE.to_string();
    out_ff.dataset.insert(dicom_toolkit_data::Element {
        tag: tags::PIXEL_DATA,
        vr: if bits_allocated > 8 {
            dicom_toolkit_dict::Vr::OW
        } else {
            dicom_toolkit_dict::Vr::OB
        },
        value: Value::PixelData(PixelData::Native { bytes: all_pixels }),
    });

    match out_ff.save(&args.output) {
        Ok(()) => {
            if args.verbose {
                eprintln!(
                    "Written: {} (Explicit VR Little Endian)",
                    args.output.display()
                );
            }
        }
        Err(e) => {
            eprintln!("Error writing {}: {e}", args.output.display());
            process::exit(1);
        }
    }
}
