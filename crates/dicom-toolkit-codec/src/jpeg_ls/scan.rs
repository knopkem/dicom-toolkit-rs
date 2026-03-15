//! Core JPEG-LS scan decoder and encoder.
//!
//! Processes a scan line-by-line using the JPEG-LS algorithm:
//! prediction → context modeling → Golomb-Rice decoding/encoding.

use dicom_toolkit_core::error::{DcmError, DcmResult};

use super::bitstream::{BitReader, BitWriter};
use super::context::{JlsContext, RunModeContext};
use super::golomb;
use super::params::DerivedTraits;
use super::prediction::{
    apply_sign, bitwise_sign, build_quantization_lut, compute_context_id, get_predicted_value,
    quantize_from_lut, sign, J,
};

/// Number of regular-mode contexts (365: half of 729 after sign normalization + context 0).
const NUM_CONTEXTS: usize = 365;

/// Scan decoder: decodes a single JPEG-LS scan to pixel data.
pub struct ScanDecoder<'a> {
    reader: BitReader<'a>,
    traits: DerivedTraits,
    #[allow(dead_code)]
    t1: i32,
    #[allow(dead_code)]
    t2: i32,
    #[allow(dead_code)]
    t3: i32,
    width: usize,
    height: usize,
    contexts: Vec<JlsContext>,
    run_contexts: [RunModeContext; 2],
    run_index: usize,
    quant_lut: Vec<i8>,
    quant_range: i32,
}

impl<'a> ScanDecoder<'a> {
    pub fn new(
        data: &'a [u8],
        traits: DerivedTraits,
        t1: i32,
        t2: i32,
        t3: i32,
        width: usize,
        height: usize,
    ) -> Self {
        let a_init = (traits.range + 32).max(2) / 64;
        let a_init = a_init.max(2);

        let contexts = vec![JlsContext::new(a_init); NUM_CONTEXTS];
        let run_contexts = [
            RunModeContext::new(a_init, traits.reset),
            RunModeContext::new(a_init, traits.reset),
        ];

        let quant_lut = build_quantization_lut(traits.bpp, t1, t2, t3, traits.near);
        let quant_range = 1i32 << traits.bpp;

        Self {
            reader: BitReader::new(data),
            traits,
            t1,
            t2,
            t3,
            width,
            height,
            contexts,
            run_contexts,
            run_index: 0,
            quant_lut,
            quant_range,
        }
    }

    /// Decode the entire scan, returning pixel data as i32 values.
    pub fn decode(&mut self) -> DcmResult<Vec<i32>> {
        let w = self.width;
        let h = self.height;
        let stride = w + 2; // 1 extra on each side for edge pixels

        // Line buffers: previous and current (with 1-pixel padding on each side).
        let mut prev_line = vec![0i32; stride];
        let mut curr_line = vec![0i32; stride];

        let mut output = Vec::with_capacity(w * h);

        for _line in 0..h {
            // Edge initialization: left edge of current = first pixel of previous.
            curr_line[0] = prev_line[1];
            // Right edge of previous = last pixel of previous.
            prev_line[w + 1] = prev_line[w];

            self.decode_line(&prev_line, &mut curr_line, w)?;

            // Copy decoded pixels to output (indices 1..=w).
            for val in curr_line.iter().take(w + 1).skip(1) {
                output.push(*val);
            }

            // Swap lines.
            std::mem::swap(&mut prev_line, &mut curr_line);
        }

        Ok(output)
    }

    /// Decode a single scan line.
    fn decode_line(&mut self, prev: &[i32], curr: &mut [i32], width: usize) -> DcmResult<()> {
        let mut index = 0usize;
        // Rb (above) and Rd (above-right) are tracked across pixels.
        let mut rb = prev[index]; // prev[0] for index=0
        let mut rd = prev[index + 1]; // prev[1]

        while index < width {
            let ra = curr[index]; // left pixel (curr[0] for first pixel = edge)
            let rc = rb;
            rb = rd;
            rd = prev[index + 2]; // above-right

            let d1 = rd - rb;
            let d2 = rb - rc;
            let d3 = rc - ra;

            let q1 = quantize_from_lut(&self.quant_lut, d1, self.quant_range);
            let q2 = quantize_from_lut(&self.quant_lut, d2, self.quant_range);
            let q3 = quantize_from_lut(&self.quant_lut, d3, self.quant_range);

            let qs = compute_context_id(q1, q2, q3);

            if qs != 0 {
                // Regular mode.
                let val = self.do_regular_decode(qs, ra, rb, rc)?;
                curr[index + 1] = val;
                index += 1;
            } else {
                // Run mode.
                let count = self.do_run_mode_decode(curr, prev, index, width)?;
                index += count;
                if index < width {
                    rb = prev[index];
                    rd = prev[index + 1];
                }
            }
        }

        Ok(())
    }

    /// Decode a regular-mode sample.
    fn do_regular_decode(&mut self, qs: i32, ra: i32, rb: i32, rc: i32) -> DcmResult<i32> {
        let sign_val = bitwise_sign(qs);
        let ctx_idx = apply_sign(qs, sign_val) as usize;
        let ctx = &mut self.contexts[ctx_idx];

        let k = ctx.get_golomb();
        let px = self.traits.correct_prediction(
            get_predicted_value(ra, rb, rc) + apply_sign(ctx.c as i32, sign_val),
        );

        // Decode the error value.
        let mapped_err =
            golomb::decode_mapped_value(&mut self.reader, k, self.traits.limit, self.traits.qbpp)?;
        let mut err_val = golomb::unmap_err_val(mapped_err);

        if err_val.abs() > 65535 {
            return Err(DcmError::DecompressionError {
                reason: "JPEG-LS: error value out of range".into(),
            });
        }

        // Apply error correction for lossless mode.
        if self.traits.near == 0 {
            err_val ^= ctx.get_error_correction(k);
        }

        ctx.update_variables(err_val, self.traits.near, self.traits.reset);
        let err_val = apply_sign(err_val, sign_val);

        Ok(self.traits.compute_reconstructed(px, err_val))
    }

    /// Decode run mode: decode run length + optional run interruption sample.
    fn do_run_mode_decode(
        &mut self,
        curr: &mut [i32],
        prev: &[i32],
        start_index: usize,
        width: usize,
    ) -> DcmResult<usize> {
        let ra = curr[start_index]; // left pixel

        // Decode run pixels.
        let run_length = self.decode_run_pixels(ra, curr, start_index, width)?;
        let end_index = start_index + run_length;

        if end_index == width {
            return Ok(run_length);
        }

        // Run interruption: decode the interruption sample.
        let rb = prev[end_index + 1]; // above pixel at interruption point
        let val = self.decode_ri_pixel(ra, rb)?;
        curr[end_index + 1] = val;
        self.run_index = self.run_index.saturating_sub(1);

        Ok(run_length + 1)
    }

    /// Decode run-length encoded pixels.
    fn decode_run_pixels(
        &mut self,
        ra: i32,
        curr: &mut [i32],
        start: usize,
        width: usize,
    ) -> DcmResult<usize> {
        let max_run = width - start;
        let mut count = 0usize;

        while self.reader.read_bit()? {
            let j_val = J[self.run_index] as usize;
            let run_len = (1usize << j_val).min(max_run - count);
            count += run_len;

            if run_len == (1usize << j_val) {
                self.run_index = (self.run_index + 1).min(31);
            }

            if count == max_run {
                break;
            }
        }

        if count < max_run {
            // Incomplete run — read remaining length.
            let j_val = J[self.run_index];
            if j_val > 0 {
                count += self.reader.read_value(j_val)? as usize;
            }
        }

        if count > max_run {
            return Err(DcmError::DecompressionError {
                reason: "JPEG-LS: run length exceeds line width".into(),
            });
        }

        // Fill pixels with Ra.
        for i in 0..count {
            curr[start + 1 + i] = ra;
        }

        Ok(count)
    }

    /// Decode a run-interruption pixel.
    fn decode_ri_pixel(&mut self, ra: i32, rb: i32) -> DcmResult<i32> {
        let ctx_idx = if (ra - rb).abs() <= self.traits.near {
            1
        } else {
            0
        };

        let err_val = self.decode_ri_error(ctx_idx)?;

        if ctx_idx == 1 {
            Ok(self.traits.compute_reconstructed(ra, err_val))
        } else {
            Ok(self
                .traits
                .compute_reconstructed(rb, err_val * sign(rb - ra)))
        }
    }

    /// Decode a run-interruption error value.
    fn decode_ri_error(&mut self, ctx_idx: usize) -> DcmResult<i32> {
        let ctx = &self.run_contexts[ctx_idx];
        let k = ctx.get_golomb();
        let limit = self.traits.limit - J[self.run_index] - 1;

        let em_err_val = golomb::decode_mapped_value(&mut self.reader, k, limit, self.traits.qbpp)?;

        let ctx = &mut self.run_contexts[ctx_idx];
        let ri_type = ctx.ri_type;
        let err_val = compute_ri_err_val(em_err_val + ri_type, k, ctx);
        ctx.update_variables(err_val, em_err_val);

        Ok(err_val)
    }
}

/// Compute the actual error value from the mapped run-interruption error value.
fn compute_ri_err_val(temp: i32, k: i32, ctx: &RunModeContext) -> i32 {
    let map = temp & 1;
    let err_abs = (temp + map) / 2;

    let condition = if k != 0 || 2 * ctx.nn >= ctx.n { 1 } else { 0 };

    if condition == map {
        -err_abs
    } else {
        err_abs
    }
}

// ── Scan encoder ──────────────────────────────────────────────────────────────

/// Scan encoder: encodes pixel data into a JPEG-LS scan.
pub struct ScanEncoder {
    writer: BitWriter,
    traits: DerivedTraits,
    #[allow(dead_code)]
    t1: i32,
    #[allow(dead_code)]
    t2: i32,
    #[allow(dead_code)]
    t3: i32,
    width: usize,
    height: usize,
    contexts: Vec<JlsContext>,
    run_contexts: [RunModeContext; 2],
    run_index: usize,
    quant_lut: Vec<i8>,
    quant_range: i32,
}

impl ScanEncoder {
    pub fn new(
        traits: DerivedTraits,
        t1: i32,
        t2: i32,
        t3: i32,
        width: usize,
        height: usize,
    ) -> Self {
        let a_init = (traits.range + 32).max(2) / 64;
        let a_init = a_init.max(2);

        let contexts = vec![JlsContext::new(a_init); NUM_CONTEXTS];
        let run_contexts = [
            RunModeContext::new(a_init, traits.reset),
            RunModeContext::new(a_init, traits.reset),
        ];

        let quant_lut = build_quantization_lut(traits.bpp, t1, t2, t3, traits.near);
        let quant_range = 1i32 << traits.bpp;

        Self {
            writer: BitWriter::new(),
            traits,
            t1,
            t2,
            t3,
            width,
            height,
            contexts,
            run_contexts,
            run_index: 0,
            quant_lut,
            quant_range,
        }
    }

    /// Encode pixel data (i32 values) into a JPEG-LS scan bitstream.
    pub fn encode(&mut self, pixels: &[i32]) -> DcmResult<Vec<u8>> {
        let w = self.width;
        let h = self.height;

        if pixels.len() != w * h {
            return Err(DcmError::CompressionError {
                reason: format!("expected {} pixels, got {}", w * h, pixels.len()),
            });
        }

        let stride = w + 2;
        let mut prev_line = vec![0i32; stride];
        let mut curr_line = vec![0i32; stride];

        for line in 0..h {
            // Load pixel data into current line (indices 1..=w).
            for i in 0..w {
                curr_line[i + 1] = pixels[line * w + i];
            }

            // Edge initialization.
            curr_line[0] = prev_line[1];
            prev_line[w + 1] = prev_line[w];

            self.encode_line(&prev_line, &mut curr_line, w)?;

            std::mem::swap(&mut prev_line, &mut curr_line);
        }

        self.writer.end_scan();
        // Take the writer output.
        let mut result_writer = BitWriter::new();
        std::mem::swap(&mut self.writer, &mut result_writer);
        Ok(result_writer.into_bytes())
    }

    fn encode_line(&mut self, prev: &[i32], curr: &mut [i32], width: usize) -> DcmResult<()> {
        let mut index = 0usize;
        let mut rb = prev[index];
        let mut rd = prev[index + 1];

        while index < width {
            let ra = curr[index];
            let rc = rb;
            rb = rd;
            rd = prev[index + 2];

            let d1 = rd - rb;
            let d2 = rb - rc;
            let d3 = rc - ra;

            let q1 = quantize_from_lut(&self.quant_lut, d1, self.quant_range);
            let q2 = quantize_from_lut(&self.quant_lut, d2, self.quant_range);
            let q3 = quantize_from_lut(&self.quant_lut, d3, self.quant_range);

            let qs = compute_context_id(q1, q2, q3);

            if qs != 0 {
                let x = curr[index + 1];
                let recon = self.do_regular_encode(qs, x, ra, rb, rc)?;
                curr[index + 1] = recon;
                index += 1;
            } else {
                let count = self.do_run_mode_encode(curr, prev, index, width)?;
                index += count;
                if index < width {
                    rb = prev[index];
                    rd = prev[index + 1];
                }
            }
        }

        Ok(())
    }

    fn do_regular_encode(&mut self, qs: i32, x: i32, ra: i32, rb: i32, rc: i32) -> DcmResult<i32> {
        let sign_val = bitwise_sign(qs);
        let ctx_idx = apply_sign(qs, sign_val) as usize;
        let ctx = &mut self.contexts[ctx_idx];

        let k = ctx.get_golomb();
        let px = self.traits.correct_prediction(
            get_predicted_value(ra, rb, rc) + apply_sign(ctx.c as i32, sign_val),
        );

        let err_val = self.traits.compute_error_val(apply_sign(x - px, sign_val));

        let mapped_err =
            golomb::get_mapped_err_val(ctx.get_error_correction(k | self.traits.near) ^ err_val);
        golomb::encode_mapped_value(
            &mut self.writer,
            k,
            mapped_err,
            self.traits.limit,
            self.traits.qbpp,
        );

        ctx.update_variables(err_val, self.traits.near, self.traits.reset);

        Ok(self
            .traits
            .compute_reconstructed(px, apply_sign(err_val, sign_val)))
    }

    fn do_run_mode_encode(
        &mut self,
        curr: &mut [i32],
        prev: &[i32],
        start_index: usize,
        width: usize,
    ) -> DcmResult<usize> {
        let ra = curr[start_index];
        let max_run = width - start_index;

        // Count the run.
        let mut run_length = 0usize;
        while run_length < max_run {
            let px = curr[start_index + 1 + run_length];
            if self.traits.near == 0 {
                if px != ra {
                    break;
                }
            } else if (px - ra).abs() > self.traits.near {
                break;
            }
            // Reconstruct as Ra for near-lossless.
            curr[start_index + 1 + run_length] = ra;
            run_length += 1;
        }

        let end_of_line = run_length == max_run;
        self.encode_run_pixels(run_length as i32, end_of_line);

        if end_of_line {
            return Ok(run_length);
        }

        // Run interruption: encode the interruption sample.
        let x = curr[start_index + 1 + run_length];
        let rb = prev[start_index + 1 + run_length];
        let recon = self.encode_ri_pixel(x, ra, rb)?;
        curr[start_index + 1 + run_length] = recon;
        self.run_index = self.run_index.saturating_sub(1);

        Ok(run_length + 1)
    }

    fn encode_run_pixels(&mut self, mut run_length: i32, end_of_line: bool) {
        while run_length >= (1 << J[self.run_index]) {
            self.writer.append_ones(1);
            run_length -= 1 << J[self.run_index];
            self.run_index = (self.run_index + 1).min(31);
        }

        if end_of_line {
            if run_length != 0 {
                self.writer.append_ones(1);
            }
        } else {
            self.writer.append(run_length, J[self.run_index] + 1);
        }
    }

    fn encode_ri_pixel(&mut self, x: i32, ra: i32, rb: i32) -> DcmResult<i32> {
        if (ra - rb).abs() <= self.traits.near {
            let err_val = self.traits.compute_error_val(x - ra);
            self.encode_ri_error(1, err_val);
            Ok(self.traits.compute_reconstructed(ra, err_val))
        } else {
            let err_val = self.traits.compute_error_val((x - rb) * sign(rb - ra));
            self.encode_ri_error(0, err_val);
            Ok(self
                .traits
                .compute_reconstructed(rb, err_val * sign(rb - ra)))
        }
    }

    fn encode_ri_error(&mut self, ctx_idx: usize, err_val: i32) {
        let ctx = &self.run_contexts[ctx_idx];
        let k = ctx.get_golomb();
        let map = ctx.compute_map(err_val, k);
        let em_err_val = 2 * err_val.abs() - ctx.ri_type - map;

        let limit = self.traits.limit - J[self.run_index] - 1;
        golomb::encode_mapped_value(&mut self.writer, k, em_err_val, limit, self.traits.qbpp);

        let ctx = &mut self.run_contexts[ctx_idx];
        ctx.update_variables(err_val, em_err_val);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jpeg_ls::params::{compute_default, BASIC_RESET};

    fn make_traits_8bit() -> (DerivedTraits, i32, i32, i32) {
        let max_val = 255;
        let near = 0;
        let defaults = compute_default(max_val, near);
        let traits = DerivedTraits::new(max_val, near, BASIC_RESET);
        (traits, defaults.t1, defaults.t2, defaults.t3)
    }

    #[test]
    fn encode_decode_roundtrip_constant() {
        let (traits, t1, t2, t3) = make_traits_8bit();
        let w = 8;
        let h = 4;
        let pixels: Vec<i32> = vec![128; w * h];

        let mut encoder = ScanEncoder::new(traits, t1, t2, t3, w, h);
        let encoded = encoder.encode(&pixels).unwrap();

        let traits2 = DerivedTraits::new(255, 0, BASIC_RESET);
        let mut decoder = ScanDecoder::new(&encoded, traits2, t1, t2, t3, w, h);
        let decoded = decoder.decode().unwrap();

        assert_eq!(decoded, pixels);
    }

    #[test]
    fn encode_decode_roundtrip_gradient() {
        let (traits, t1, t2, t3) = make_traits_8bit();
        let w = 16;
        let h = 8;
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                pixels.push(((x * 16 + y * 8) % 256) as i32);
            }
        }

        let mut encoder = ScanEncoder::new(traits, t1, t2, t3, w, h);
        let encoded = encoder.encode(&pixels).unwrap();

        let traits2 = DerivedTraits::new(255, 0, BASIC_RESET);
        let mut decoder = ScanDecoder::new(&encoded, traits2, t1, t2, t3, w, h);
        let decoded = decoder.decode().unwrap();

        assert_eq!(decoded, pixels);
    }

    #[test]
    fn encode_decode_roundtrip_random_like() {
        let (traits, t1, t2, t3) = make_traits_8bit();
        let w = 32;
        let h = 16;
        let mut pixels = Vec::with_capacity(w * h);
        // Pseudo-random deterministic data.
        let mut val: u32 = 42;
        for _ in 0..(w * h) {
            val = val.wrapping_mul(1103515245).wrapping_add(12345);
            pixels.push(((val >> 16) & 0xFF) as i32);
        }

        let mut encoder = ScanEncoder::new(traits, t1, t2, t3, w, h);
        let encoded = encoder.encode(&pixels).unwrap();

        let traits2 = DerivedTraits::new(255, 0, BASIC_RESET);
        let mut decoder = ScanDecoder::new(&encoded, traits2, t1, t2, t3, w, h);
        let decoded = decoder.decode().unwrap();

        assert_eq!(decoded, pixels);
    }

    #[test]
    fn encode_decode_roundtrip_1x1() {
        let (traits, t1, t2, t3) = make_traits_8bit();
        let pixels = vec![200i32];

        let mut encoder = ScanEncoder::new(traits, t1, t2, t3, 1, 1);
        let encoded = encoder.encode(&pixels).unwrap();

        let traits2 = DerivedTraits::new(255, 0, BASIC_RESET);
        let mut decoder = ScanDecoder::new(&encoded, traits2, t1, t2, t3, 1, 1);
        let decoded = decoder.decode().unwrap();

        assert_eq!(decoded, pixels);
    }
}
