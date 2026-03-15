//! img2dcm — convert PNG/grayscale images to DICOM Secondary Capture.
//!
//! Port of DCMTK's `img2dcm` utility (subset: PNG input, SC Image output).

use std::fs;
use std::io::BufReader;
use std::path::PathBuf;
use std::process;

use clap::Parser;

use dicom_toolkit_core::uid::{sop_class, Uid};
use dicom_toolkit_data::{DataSet, FileFormat};
use dicom_toolkit_dict::{tags, Vr};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "img2dcm",
    about = "Convert PNG image to DICOM Secondary Capture",
    long_about = "Reads a PNG (or raw grayscale) image and wraps it in a DICOM Part 10\n\
                  file using the Secondary Capture Image Storage SOP class."
)]
struct Args {
    /// Input PNG file
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Output DICOM file [default: <input>.dcm]
    #[arg(value_name = "OUTPUT")]
    output: Option<PathBuf>,

    /// Patient name to embed [default: "Anonymous"]
    #[arg(short = 'p', long, default_value = "Anonymous")]
    patient_name: String,

    /// Patient ID to embed
    #[arg(short = 'P', long)]
    patient_id: Option<String>,

    /// Study description
    #[arg(short = 's', long)]
    study_description: Option<String>,

    /// Series description
    #[arg(short = 'S', long)]
    series_description: Option<String>,

    /// Force SOP Class UID (default: Secondary Capture Image Storage)
    #[arg(long)]
    sop_class: Option<String>,

    /// Force SOP Instance UID (default: auto-generated)
    #[arg(long)]
    sop_instance: Option<String>,

    /// Verbose output
    #[arg(short = 'v', long)]
    verbose: bool,
}

// ── Entrypoint ────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if let Err(e) = run(args) {
        eprintln!("img2dcm: error: {e}");
        process::exit(1);
    }
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let output = args
        .output
        .unwrap_or_else(|| args.input.with_extension("dcm"));

    if args.verbose {
        eprintln!("Reading PNG: {}", args.input.display());
    }

    // Decode PNG.
    let file = fs::File::open(&args.input)?;
    let reader = BufReader::new(file);
    let decoder = png::Decoder::new(reader);
    let mut png_reader = decoder.read_info()?;
    let mut img_data = vec![0u8; png_reader.output_buffer_size()];
    let frame_info = png_reader.next_frame(&mut img_data)?;

    let width = frame_info.width as u16;
    let height = frame_info.height as u16;

    let (samples_per_pixel, photometric, bits_allocated, bits_stored) = match frame_info.color_type
    {
        png::ColorType::Grayscale => (1u16, "MONOCHROME2", 8u16, 8u16),
        png::ColorType::GrayscaleAlpha => {
            // Drop alpha channel.
            img_data = img_data.chunks_exact(2).map(|c| c[0]).collect();
            (1u16, "MONOCHROME2", 8u16, 8u16)
        }
        png::ColorType::Rgb => (3u16, "RGB", 8u16, 8u16),
        png::ColorType::Rgba => {
            // Drop alpha channel.
            img_data = img_data
                .chunks_exact(4)
                .flat_map(|c| [c[0], c[1], c[2]])
                .collect();
            (3u16, "RGB", 8u16, 8u16)
        }
        png::ColorType::Indexed => {
            return Err("indexed-color PNG is not supported; convert to RGB first".into());
        }
    };

    // Trim img_data to actual pixels (row stride may differ from width×channels).
    let row_bytes = width as usize * samples_per_pixel as usize;
    let expected = row_bytes * height as usize;
    img_data.truncate(expected);

    if args.verbose {
        eprintln!(
            "Image: {}×{}, {} channel(s), {} bit(s), {} bytes",
            width,
            height,
            samples_per_pixel,
            bits_stored,
            img_data.len()
        );
    }

    // Build the DICOM dataset.
    let sop_class_uid = args
        .sop_class
        .as_deref()
        .unwrap_or(sop_class::SECONDARY_CAPTURE_IMAGE_STORAGE);
    let sop_instance_uid = args
        .sop_instance
        .clone()
        .unwrap_or_else(|| Uid::generate("2.25").unwrap().to_string());

    let mut ds = DataSet::new();

    // Patient Module
    ds.set_string(tags::PATIENT_NAME, Vr::PN, &args.patient_name);
    if let Some(pid) = &args.patient_id {
        ds.set_string(tags::PATIENT_ID, Vr::LO, pid);
    }

    // General Study Module
    ds.set_uid(tags::SOP_CLASS_UID, sop_class_uid);
    ds.set_uid(tags::SOP_INSTANCE_UID, &sop_instance_uid);
    if let Some(desc) = &args.study_description {
        ds.set_string(tags::STUDY_DESCRIPTION, Vr::LO, desc);
    }

    // General Series Module
    if let Some(desc) = &args.series_description {
        ds.set_string(tags::SERIES_DESCRIPTION, Vr::LO, desc);
    }

    // Image Pixel Module
    ds.set_u16(tags::SAMPLES_PER_PIXEL, samples_per_pixel);
    ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, photometric);
    ds.set_u16(tags::ROWS, height);
    ds.set_u16(tags::COLUMNS, width);
    ds.set_u16(tags::BITS_ALLOCATED, bits_allocated);
    ds.set_u16(tags::BITS_STORED, bits_stored);
    ds.set_u16(tags::HIGH_BIT, bits_stored - 1);
    ds.set_u16(tags::PIXEL_REPRESENTATION, 0); // unsigned
    ds.set_u16(tags::PLANAR_CONFIGURATION, 0); // interleaved
    ds.set_bytes(tags::PIXEL_DATA, Vr::OB, img_data);

    // Save as DICOM Part 10 file.
    let ff = FileFormat::from_dataset(sop_class_uid, &sop_instance_uid, ds);
    ff.save(&output)?;

    println!(
        "Wrote {} → {} ({} bytes)",
        args.input.display(),
        output.display(),
        fs::metadata(&output)?.len()
    );

    Ok(())
}
