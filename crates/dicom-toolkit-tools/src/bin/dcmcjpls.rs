//! dcmcjpls — Compress DICOM file to JPEG-LS transfer syntax.
//!
//! Port of DCMTK's `dcmcjpls` utility.

use std::path::PathBuf;
use std::process;

use clap::Parser;

use dicom_toolkit_codec::jpeg_ls::encoder::encode_jpeg_ls;
use dicom_toolkit_codec::registry::GLOBAL_REGISTRY;
use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::FileFormat;
use dicom_toolkit_dict::tags;

const TS_JPEGLS_LOSSLESS: &str = "1.2.840.10008.1.2.4.80";
const TS_JPEGLS_LOSSY: &str = "1.2.840.10008.1.2.4.81";

#[derive(Parser)]
#[command(
    name = "dcmcjpls",
    about = "Encode DICOM file with JPEG-LS transfer syntax",
    long_about = "Reads a DICOM file and re-encodes the pixel data using JPEG-LS\n\
                  compression (lossless or near-lossless). Writes a DICOM Part 10 file\n\
                  with JPEG-LS Lossless or JPEG-LS Lossy transfer syntax."
)]
struct Args {
    /// Input DICOM file
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Output DICOM file
    #[arg(value_name = "OUTPUT")]
    output: PathBuf,

    /// Near-lossless: maximum allowed pixel error (0 = lossless, default)
    #[arg(long = "max-deviation", short = 'n', default_value = "0")]
    near: i32,

    /// Force lossless encoding (NEAR=0)
    #[arg(
        long = "encode-lossless",
        short = 'l',
        conflicts_with = "near_lossless"
    )]
    lossless: bool,

    /// Force near-lossless encoding with default NEAR=2
    #[arg(long = "encode-nearlossless", conflicts_with = "lossless")]
    near_lossless: bool,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();

    let near = if args.near_lossless {
        2 // DCMTK default
    } else if args.lossless {
        0
    } else {
        args.near
    };

    // Open input DICOM file.
    let ff = match FileFormat::open(&args.input) {
        Ok(ff) => ff,
        Err(e) => {
            eprintln!("Error reading {}: {e}", args.input.display());
            process::exit(1);
        }
    };

    let ds = &ff.dataset;

    // Extract image parameters.
    let rows = ds.get_u16(tags::ROWS).unwrap_or(0);
    let cols = ds.get_u16(tags::COLUMNS).unwrap_or(0);
    let bits_allocated = ds.get_u16(tags::BITS_ALLOCATED).unwrap_or(8) as u8;
    let bits_stored = ds
        .get_u16(tags::BITS_STORED)
        .unwrap_or(bits_allocated as u16) as u8;
    let samples_per_pixel = ds.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1) as u8;

    if rows == 0 || cols == 0 {
        eprintln!("Error: image has zero dimensions ({cols}x{rows})");
        process::exit(1);
    }

    if args.verbose {
        eprintln!(
            "Input: {}x{}, {} bit ({} stored), {} component(s), TS: {}",
            cols, rows, bits_allocated, bits_stored, samples_per_pixel, ff.meta.transfer_syntax_uid
        );
    }

    // Get uncompressed pixel data.
    let raw_pixels = match ds.get(tags::PIXEL_DATA) {
        Some(elem) => match &elem.value {
            Value::PixelData(PixelData::Native { bytes }) => bytes.clone(),
            Value::PixelData(PixelData::Encapsulated { .. }) => {
                // Already compressed — try to decompress first.
                let ts_uid = &ff.meta.transfer_syntax_uid;
                match GLOBAL_REGISTRY.find(ts_uid) {
                    Some(codec) => {
                        match codec.decode(
                            match &elem.value {
                                Value::PixelData(pd) => pd,
                                _ => unreachable!(),
                            },
                            rows,
                            cols,
                            samples_per_pixel,
                            bits_allocated,
                        ) {
                            Ok(raw) => raw,
                            Err(e) => {
                                eprintln!("Error decompressing input ({ts_uid}): {e}");
                                process::exit(1);
                            }
                        }
                    }
                    None => {
                        eprintln!(
                            "Error: input is compressed ({ts_uid}) and no codec is available to decompress it"
                        );
                        process::exit(1);
                    }
                }
            }
            Value::U8(bytes) => bytes.clone(),
            Value::U16(words) => {
                // Convert u16 slice to bytes (LE).
                let mut bytes = Vec::with_capacity(words.len() * 2);
                for w in words {
                    bytes.extend_from_slice(&w.to_le_bytes());
                }
                bytes
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
        eprintln!("Uncompressed pixel data: {} bytes", raw_pixels.len());
    }

    // Encode with JPEG-LS.
    let encoded = match encode_jpeg_ls(
        &raw_pixels,
        cols as u32,
        rows as u32,
        bits_stored,
        samples_per_pixel,
        near,
    ) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error encoding JPEG-LS: {e}");
            process::exit(1);
        }
    };

    if args.verbose {
        let ratio = raw_pixels.len() as f64 / encoded.len() as f64;
        eprintln!(
            "JPEG-LS encoded: {} bytes (compression ratio {ratio:.1}:1, NEAR={near})",
            encoded.len()
        );
    }

    // Build the output DICOM file.
    let ts_uid = if near == 0 {
        TS_JPEGLS_LOSSLESS
    } else {
        TS_JPEGLS_LOSSY
    };

    let mut out_ff = ff.clone();
    out_ff.meta.transfer_syntax_uid = ts_uid.to_string();

    // Replace pixel data with encapsulated JPEG-LS fragment.
    let pixel_data = PixelData::Encapsulated {
        offset_table: vec![0],
        fragments: vec![encoded],
    };
    out_ff.dataset.insert(dicom_toolkit_data::Element {
        tag: tags::PIXEL_DATA,
        vr: dicom_toolkit_dict::Vr::OB,
        value: Value::PixelData(pixel_data),
    });

    // Update lossy compression attributes if near-lossless.
    if near > 0 {
        out_ff.dataset.set_string(
            tags::LOSSY_IMAGE_COMPRESSION,
            dicom_toolkit_dict::Vr::CS,
            "01",
        );
    }

    // Save.
    match out_ff.save(&args.output) {
        Ok(()) => {
            if args.verbose {
                eprintln!(
                    "Written: {} (TS: {})",
                    args.output.display(),
                    if near == 0 {
                        "JPEG-LS Lossless"
                    } else {
                        "JPEG-LS Near-lossless"
                    }
                );
            }
        }
        Err(e) => {
            eprintln!("Error writing {}: {e}", args.output.display());
            process::exit(1);
        }
    }
}
