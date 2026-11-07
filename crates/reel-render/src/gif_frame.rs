//! Reducing a frame to a GIF's 256 colours (§43): its own palette, with Floyd-Steinberg dithering.

use color_quant::NeuQuant;

/// How many pixels the palette search looks at: one in this many. Higher is quicker and a bit rougher.
const QUANT_SAMPLE: i32 = 10;

/// Quantizes premultiplied, opaque RGBA into an indexed frame.
#[must_use]
pub fn quantize(rgba: &[u8], width: usize, height: usize) -> gif::Frame<'static> {
    let quant = NeuQuant::new(QUANT_SAMPLE, 256, rgba);
    let palette = quant.color_map_rgb();

    // Errors from earlier pixels are carried forward, a row at a time.
    let mut current = vec![[0_i32; 3]; width + 2];
    let mut next = vec![[0_i32; 3]; width + 2];
    let mut indices = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let at = (y * width + x) * 4;
            let carried = current[x + 1];
            let wanted = [
                (i32::from(rgba[at]) + carried[0] / 16).clamp(0, 255),
                (i32::from(rgba[at + 1]) + carried[1] / 16).clamp(0, 255),
                (i32::from(rgba[at + 2]) + carried[2] / 16).clamp(0, 255),
            ];
            let index = quant.index_of(&[wanted[0] as u8, wanted[1] as u8, wanted[2] as u8, 255]);
            indices.push(index as u8);
            let chosen = &palette[index * 3..index * 3 + 3];
            for channel in 0..3 {
                let error = wanted[channel] - i32::from(chosen[channel]);
                current[x + 2][channel] += error * 7;
                next[x][channel] += error * 3;
                next[x + 1][channel] += error * 5;
                next[x + 2][channel] += error;
            }
        }
        std::mem::swap(&mut current, &mut next);
        next.iter_mut().for_each(|cell| *cell = [0; 3]);
    }
    gif::Frame {
        palette: Some(palette),
        buffer: indices.into(),
        ..gif::Frame::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, color: [u8; 4]) -> Vec<u8> {
        color
            .iter()
            .copied()
            .cycle()
            .take(width * height * 4)
            .collect()
    }

    #[test]
    fn there_is_one_index_per_pixel() {
        let frame = quantize(&solid(40, 20, [200, 100, 50, 255]), 40, 20);
        assert_eq!(frame.buffer.len(), 800);
    }

    #[test]
    fn a_solid_colour_comes_out_close_to_itself() {
        let frame = quantize(&solid(64, 64, [200, 100, 50, 255]), 64, 64);
        let palette = frame.palette.as_ref().unwrap();
        let index = usize::from(frame.buffer[1000]);
        let found = &palette[index * 3..index * 3 + 3];
        for (got, want) in found.iter().zip([200_i32, 100, 50]) {
            assert!((i32::from(*got) - want).abs() < 16, "{found:?}");
        }
    }

    #[test]
    fn a_gradient_uses_many_colours_and_keeps_its_average() {
        let mut rgba = Vec::new();
        for _ in 0..64 {
            for x in 0..256_u32 {
                rgba.extend_from_slice(&[x as u8, 128, 255 - x as u8, 255]);
            }
        }
        let frame = quantize(&rgba, 256, 64);
        let palette = frame.palette.as_ref().unwrap();
        let mut seen: Vec<u8> = frame.buffer.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() > 30, "{} colours", seen.len());
        // Down one column, dithering should average out to about the true value.
        let column = 100;
        let total: f64 = (0..64)
            .map(|row| f64::from(palette[usize::from(frame.buffer[row * 256 + column]) * 3]))
            .sum();
        let average = total / 64.0;
        assert!((average - column as f64).abs() < 20.0, "{average}");
    }
}
