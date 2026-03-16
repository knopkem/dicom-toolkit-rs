//! dcmcjp2k — Compress DICOM file to JPEG 2000 transfer syntax.
//!
//! Port of DCMTK's `dcmcjp2k` utility.

use std::path::PathBuf;
use std::process;

use clap::Parser;

use dicom_toolkit_codec::jp2k::encoder::{encode_htj2k, encode_jp2k};
use dicom_toolkit_codec::registry::GLOBAL_REGISTRY;
use dicom_toolkit_core::uid::transfer_syntax;
use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::FileFormat;
use dicom_toolkit_dict::tags;

const TS_JPEG2000_LOSSLESS: &str = transfer_syntax::JPEG_2000_LOSSLESS;
const TS_JPEG2000_LOSSY: &str = transfer_syntax::JPEG_2000;
const TS_HTJ2K_LOSSLESS: &str = transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY;
const TS_HTJ2K: &str = transfer_syntax::HIGH_THROUGHPUT_JPEG_2000;

#[derive(Parser)]
#[command(
    name = "dcmcjp2k",
    about = "Encode DICOM file with JPEG 2000 transfer syntax",
    long_about = "Reads a DICOM file and re-encodes the pixel data using JPEG 2000\n\
                  or High-Throughput JPEG 2000 compression. Writes a DICOM Part 10\n\
                  file with the matching JPEG 2000 or HTJ2K transfer syntax."
)]
struct Args {
    /// Input DICOM file
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Output DICOM file
    #[arg(value_name = "OUTPUT")]
    output: PathBuf,

    /// Force lossless encoding (default)
    #[arg(long = "encode-lossless", short = 'l', conflicts_with = "lossy")]
    lossless: bool,

    /// Use irreversible JPEG 2000 encoding
    #[arg(long = "encode-lossy", conflicts_with = "lossless")]
    lossy: bool,

    /// Use High-Throughput JPEG 2000 block coding (HTJ2K / Part 15)
    #[arg(long = "htj2k")]
    htj2k: bool,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();
    let lossless = !args.lossy;
    let (ts_uid, compression_name) = match (args.htj2k, lossless) {
        (false, true) => (TS_JPEG2000_LOSSLESS, "JPEG 2000 Lossless"),
        (false, false) => (TS_JPEG2000_LOSSY, "JPEG 2000"),
        (true, true) => (TS_HTJ2K_LOSSLESS, "HTJ2K Lossless"),
        (true, false) => (TS_HTJ2K, "HTJ2K"),
    };

    let ff = match FileFormat::open(&args.input) {
        Ok(ff) => ff,
        Err(e) => {
            eprintln!("Error reading {}: {e}", args.input.display());
            process::exit(1);
        }
    };

    let ds = &ff.dataset;

    let rows = ds.get_u16(tags::ROWS).unwrap_or(0);
    let cols = ds.get_u16(tags::COLUMNS).unwrap_or(0);
    let bits_allocated = ds.get_u16(tags::BITS_ALLOCATED).unwrap_or(8) as u8;
    let bits_stored = ds
        .get_u16(tags::BITS_STORED)
        .unwrap_or(bits_allocated as u16) as u8;
    let samples_per_pixel = ds.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1) as u8;
    let number_of_frames = ds
        .get_string(tags::NUMBER_OF_FRAMES)
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1);

    if rows == 0 || cols == 0 {
        eprintln!("Error: image has zero dimensions ({cols}x{rows})");
        process::exit(1);
    }

    if bits_allocated != 8 && bits_allocated != 16 {
        eprintln!(
            "Error: JPEG 2000 tool currently expects 8- or 16-bit allocated samples, got {bits_allocated}"
        );
        process::exit(1);
    }

    if args.verbose {
        eprintln!(
            "Input: {}x{}, {} bit allocated ({} stored), {} component(s), {} frame(s), TS: {}, target: {}",
            cols,
            rows,
            bits_allocated,
            bits_stored,
            samples_per_pixel,
            number_of_frames,
            ff.meta.transfer_syntax_uid,
            compression_name,
        );
    }

    let raw_pixels = match ds.get(tags::PIXEL_DATA) {
        Some(elem) => match &elem.value {
            Value::PixelData(PixelData::Native { bytes }) => bytes.clone(),
            Value::PixelData(PixelData::Encapsulated { .. }) => {
                let ts_uid = &ff.meta.transfer_syntax_uid;
                match GLOBAL_REGISTRY.find_decoder(ts_uid) {
                    Some(codec) => match codec.decode(
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
                    },
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

    let bytes_per_sample = if bits_allocated <= 8 { 1usize } else { 2usize };
    let frame_size = rows as usize * cols as usize * samples_per_pixel as usize * bytes_per_sample;
    if raw_pixels.len() != frame_size * number_of_frames {
        eprintln!(
            "Error: pixel data size mismatch: got {} bytes, expected {} ({} frame(s) × {} bytes/frame)",
            raw_pixels.len(),
            frame_size * number_of_frames,
            number_of_frames,
            frame_size
        );
        process::exit(1);
    }

    let mut fragments = Vec::with_capacity(number_of_frames);
    for frame_idx in 0..number_of_frames {
        let start = frame_idx * frame_size;
        let end = start + frame_size;
        let frame_pixels = &raw_pixels[start..end];
        let encoded = match if args.htj2k {
            encode_htj2k(
                frame_pixels,
                cols as u32,
                rows as u32,
                bits_stored,
                samples_per_pixel,
                lossless,
            )
        } else {
            encode_jp2k(
                frame_pixels,
                cols as u32,
                rows as u32,
                bits_stored,
                samples_per_pixel,
                lossless,
            )
        } {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Error encoding {compression_name} frame {frame_idx}: {e}");
                process::exit(1);
            }
        };
        fragments.push(encoded);
    }

    if args.verbose {
        let total_encoded: usize = fragments.iter().map(|f| f.len()).sum();
        let ratio = raw_pixels.len() as f64 / total_encoded.max(1) as f64;
        eprintln!(
            "{compression_name} encoded: {} frame(s), {} bytes total (compression ratio {ratio:.1}:1)",
            fragments.len(),
            total_encoded
        );
    }

    let mut out_ff = ff.clone();
    out_ff.meta.transfer_syntax_uid = ts_uid.to_string();
    out_ff.dataset.insert(dicom_toolkit_data::Element {
        tag: tags::PIXEL_DATA,
        vr: dicom_toolkit_dict::Vr::OB,
        value: Value::PixelData(PixelData::Encapsulated {
            offset_table: vec![],
            fragments,
        }),
    });

    if !lossless {
        out_ff.dataset.set_string(
            tags::LOSSY_IMAGE_COMPRESSION,
            dicom_toolkit_dict::Vr::CS,
            "01",
        );
    }

    match out_ff.save(&args.output) {
        Ok(()) => {
            if args.verbose {
                eprintln!(
                    "Written: {} (TS: {})",
                    args.output.display(),
                    compression_name
                );
            }
        }
        Err(e) => {
            eprintln!("Error writing {}: {e}", args.output.display());
            process::exit(1);
        }
    }
}
