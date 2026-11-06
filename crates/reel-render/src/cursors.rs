//! Reel's own cursor artwork, drawn from shapes here.
//!
//! Recordly's cursors may be Apple's artwork, with their origin unconfirmed, so none of them are used
//! (D8). Each shape is drawn in a 100×100 square, placed so its hotspot lands where §34 says, which
//! means cursor positions from Recordly projects still point at the same thing.

use reel_motion::cursor::CursorShape;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

/// How many pixels a sprite is on each side.
pub const SPRITE_SIZE: u32 = 256;

/// A cursor image: premultiplied RGBA, with where its hotspot is, from 0 to 1.
pub struct Sprite {
    pub rgba: Vec<u8>,
    pub shadow: Vec<u8>,
    pub hotspot: (f64, f64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ink {
    Light,
    Dark,
}

fn outline(ink: Ink) -> (Color, Color) {
    let dark = Color::from_rgba8(15, 23, 42, 240);
    match ink {
        Ink::Light => (Color::WHITE, dark),
        Ink::Dark => (dark, Color::WHITE),
    }
}

fn polygon(points: &[(f32, f32)]) -> tiny_skia::Path {
    let mut builder = PathBuilder::new();
    builder.move_to(points[0].0, points[0].1);
    for point in &points[1..] {
        builder.line_to(point.0, point.1);
    }
    builder.close();
    builder.finish().expect("a cursor's polygon is never empty")
}

fn circle(x: f32, y: f32, radius: f32) -> tiny_skia::Path {
    PathBuilder::from_circle(x, y, radius).expect("a cursor's circle has a size")
}

/// Draws filled shapes with an outline around the lot, so overlapping parts read as one.
fn paint_shapes(pixmap: &mut Pixmap, shapes: &[tiny_skia::Path], ink: Ink, stroke_width: f32) {
    let scale = SPRITE_SIZE as f32 / 100.0;
    let transform = Transform::from_scale(scale, scale);
    let (fill, line) = outline(ink);
    let mut paint = Paint {
        anti_alias: true,
        ..Paint::default()
    };

    paint.set_color(line);
    let stroke = Stroke {
        width: stroke_width,
        line_join: LineJoin::Round,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    for shape in shapes {
        pixmap.stroke_path(shape, &paint, &stroke, transform, None);
    }
    paint.set_color(fill);
    for shape in shapes {
        pixmap.fill_path(shape, &paint, FillRule::Winding, transform, None);
    }
}

fn arrow_at(x: f32, y: f32, size: f32) -> tiny_skia::Path {
    let points: Vec<(f32, f32)> = [
        (0.0, 0.0),
        (0.0, 0.72),
        (0.18, 0.56),
        (0.3, 0.84),
        (0.43, 0.78),
        (0.31, 0.51),
        (0.54, 0.51),
    ]
    .iter()
    .map(|(px, py)| (x + px * size, y + py * size))
    .collect();
    polygon(&points)
}

fn double_arrow(horizontal: bool) -> tiny_skia::Path {
    let along = [
        (10.0, 50.0),
        (30.0, 32.0),
        (30.0, 43.0),
        (70.0, 43.0),
        (70.0, 32.0),
        (90.0, 50.0),
        (70.0, 68.0),
        (70.0, 57.0),
        (30.0, 57.0),
        (30.0, 68.0),
    ];
    let points: Vec<(f32, f32)> = along
        .iter()
        .map(|&(a, b)| if horizontal { (a, b) } else { (b, a) })
        .collect();
    polygon(&points)
}

fn hand(fingers: &[(f32, f32, f32)], palm: (f32, f32, f32)) -> Vec<tiny_skia::Path> {
    let mut shapes: Vec<_> = fingers
        .iter()
        .map(|&(x, y, radius)| circle(x, y, radius))
        .collect();
    shapes.push(circle(palm.0, palm.1, palm.2));
    shapes
}

fn paint_shape(pixmap: &mut Pixmap, shape: CursorShape, ink: Ink) {
    match shape {
        CursorShape::Arrow => paint_shapes(pixmap, &[arrow_at(14.0, 6.0, 90.0)], ink, 5.0),
        CursorShape::Text => {
            let beam = polygon(&[
                (36.0, 12.0),
                (44.0, 12.0),
                (50.0, 16.0),
                (56.0, 12.0),
                (64.0, 12.0),
                (64.0, 19.0),
                (54.0, 20.0),
                (54.0, 68.0),
                (64.0, 69.0),
                (64.0, 76.0),
                (56.0, 76.0),
                (50.0, 72.0),
                (44.0, 76.0),
                (36.0, 76.0),
                (36.0, 69.0),
                (46.0, 68.0),
                (46.0, 20.0),
                (36.0, 19.0),
            ]);
            paint_shapes(pixmap, &[beam], ink, 3.5);
        }
        CursorShape::Pointer => {
            let finger = polygon(&[
                (35.0, 14.0),
                (40.0, 9.0),
                (45.0, 14.0),
                (45.0, 44.0),
                (35.0, 44.0),
            ]);
            let mut shapes = hand(
                &[(53.0, 46.0, 7.5), (64.0, 49.0, 7.5), (74.0, 53.0, 6.5)],
                (53.0, 66.0, 22.0),
            );
            shapes.push(finger);
            paint_shapes(pixmap, &shapes, ink, 4.0);
        }
        CursorShape::Crosshair => {
            let bar =
                |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| polygon(&[a, b, c, d]);
            let shapes = [
                bar((47.0, 14.0), (53.0, 14.0), (53.0, 86.0), (47.0, 86.0)),
                bar((14.0, 47.0), (86.0, 47.0), (86.0, 53.0), (14.0, 53.0)),
            ];
            paint_shapes(pixmap, &shapes, ink, 3.0);
        }
        CursorShape::OpenHand => {
            let shapes = hand(
                &[
                    (34.0, 42.0, 7.0),
                    (46.0, 32.0, 7.5),
                    (58.0, 31.0, 7.5),
                    (70.0, 38.0, 7.0),
                    (26.0, 58.0, 7.0),
                ],
                (55.0, 60.0, 23.0),
            );
            paint_shapes(pixmap, &shapes, ink, 4.0);
        }
        CursorShape::ClosedHand => {
            let shapes = hand(
                &[
                    (36.0, 37.0, 7.5),
                    (47.0, 33.0, 7.5),
                    (58.0, 33.0, 7.5),
                    (68.0, 38.0, 7.0),
                ],
                (51.0, 50.0, 22.0),
            );
            paint_shapes(pixmap, &shapes, ink, 4.0);
        }
        CursorShape::ResizeEw => paint_shapes(pixmap, &[double_arrow(true)], ink, 4.0),
        CursorShape::ResizeNs => paint_shapes(pixmap, &[double_arrow(false)], ink, 4.0),
        CursorShape::NotAllowed => {
            let sign = circle(58.0, 66.0, 20.0);
            paint_shapes(pixmap, &[arrow_at(23.0, 0.0, 66.0), sign], ink, 4.0);
            let scale = SPRITE_SIZE as f32 / 100.0;
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            paint.set_color(Color::from_rgba8(220, 38, 38, 255));
            let line = polygon(&[(44.0, 54.0), (48.0, 50.0), (72.0, 78.0), (68.0, 82.0)]);
            pixmap.fill_path(
                &line,
                &paint,
                FillRule::Winding,
                Transform::from_scale(scale, scale),
                None,
            );
        }
    }
}

fn hotspot(shape: CursorShape) -> (f64, f64) {
    match shape {
        CursorShape::Arrow => (0.14, 0.06),
        CursorShape::Text => (0.50, 0.44),
        CursorShape::Pointer => (0.40, 0.10),
        CursorShape::Crosshair | CursorShape::ResizeEw => (0.50, 0.50),
        CursorShape::OpenHand => (0.55, 0.57),
        CursorShape::ClosedHand => (0.50, 0.46),
        CursorShape::ResizeNs => (0.50, 0.49),
        CursorShape::NotAllowed => (0.23, 0.0),
    }
}

/// A soft copy of the sprite's outline in black, for its shadow.
fn soften(rgba: &[u8]) -> Vec<u8> {
    let size = SPRITE_SIZE as usize;
    let radius = 8_i64;
    let alpha = |x: i64, y: i64, source: &[u8]| -> u32 {
        if x < 0 || y < 0 || x >= size as i64 || y >= size as i64 {
            return 0;
        }
        u32::from(source[y as usize * size + x as usize])
    };
    let mut current: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
    // Three box blurs come close to a Gaussian.
    for _ in 0..3 {
        let mut horizontal = vec![0_u8; size * size];
        for y in 0..size as i64 {
            for x in 0..size as i64 {
                let sum: u32 = (-radius..=radius).map(|d| alpha(x + d, y, &current)).sum();
                horizontal[y as usize * size + x as usize] = (sum / (2 * radius as u32 + 1)) as u8;
            }
        }
        let mut vertical = vec![0_u8; size * size];
        for y in 0..size as i64 {
            for x in 0..size as i64 {
                let sum: u32 = (-radius..=radius)
                    .map(|d| alpha(x, y + d, &horizontal))
                    .sum();
                vertical[y as usize * size + x as usize] = (sum / (2 * radius as u32 + 1)) as u8;
            }
        }
        current = vertical;
    }
    current.iter().flat_map(|&a| [0, 0, 0, a]).collect()
}

/// Draws a cursor. Unknown styles get the standard arrow set.
#[must_use]
pub fn sprite(style: &str, shape: CursorShape) -> Sprite {
    let mut pixmap = Pixmap::new(SPRITE_SIZE, SPRITE_SIZE).expect("the size is fixed and valid");
    let mut spot = hotspot(shape);
    match style {
        "dot" => {
            // A dot is the same for every shape, centred on the true position.
            paint_shapes(&mut pixmap, &[circle(50.0, 50.0, 23.0)], Ink::Light, 9.0);
            spot = (0.5, 0.5);
        }
        "minimal" | "figma" => {
            paint_shapes(
                &mut pixmap,
                &[polygon(&[
                    (14.0, 6.0),
                    (14.0, 74.0),
                    (32.0, 56.0),
                    (60.0, 54.0),
                ])],
                Ink::Light,
                4.0,
            );
            spot = (0.14, 0.06);
        }
        "tahoe-inverted" => paint_shape(&mut pixmap, shape, Ink::Dark),
        _ => paint_shape(&mut pixmap, shape, Ink::Light),
    }
    let rgba = pixmap.data().to_vec();
    let shadow = soften(&rgba);
    Sprite {
        rgba,
        shadow,
        hotspot: spot,
    }
}

/// Where a style's hotspot is, without drawing it.
#[must_use]
pub fn sprite_hotspot(style: &str, shape: CursorShape) -> (f64, f64) {
    match style {
        "dot" => (0.5, 0.5),
        "minimal" | "figma" => (0.14, 0.06),
        _ => hotspot(shape),
    }
}

/// Whether a style draws a shadow. Minimal doesn't (§34).
#[must_use]
pub fn has_shadow(style: &str) -> bool {
    !matches!(style, "minimal" | "figma")
}

/// The sprite's height for a cursor of a given height, since the art doesn't fill its whole square.
pub const SPRITE_PER_CURSOR: f64 = 1.22;

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [CursorShape; 9] = [
        CursorShape::Arrow,
        CursorShape::Text,
        CursorShape::Pointer,
        CursorShape::Crosshair,
        CursorShape::OpenHand,
        CursorShape::ClosedHand,
        CursorShape::ResizeEw,
        CursorShape::ResizeNs,
        CursorShape::NotAllowed,
    ];

    fn opaque_pixels(sprite: &Sprite) -> usize {
        sprite
            .rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 200)
            .count()
    }

    #[test]
    fn every_shape_draws_something() {
        for shape in ALL {
            assert!(
                opaque_pixels(&sprite("tahoe", shape)) > 1000,
                "{shape:?} is blank"
            );
        }
    }

    #[test]
    fn the_arrows_tip_is_at_its_hotspot() {
        let cursor = sprite("tahoe", CursorShape::Arrow);
        let size = SPRITE_SIZE as usize;
        let (hx, hy) = (
            (cursor.hotspot.0 * size as f64) as usize,
            (cursor.hotspot.1 * size as f64) as usize,
        );
        // Just inside the tip it's solid.
        let at = ((hy + 12) * size + hx + 3) * 4;
        assert!(cursor.rgba[at + 3] > 200, "the tip should be solid");
        // Well above and to the left of it, there's nothing.
        let outside = ((hy - 12) * size + hx - 12) * 4;
        assert_eq!(cursor.rgba[outside + 3], 0);
    }

    #[test]
    fn inverted_swaps_the_colours() {
        let (light, dark) = (
            sprite("tahoe", CursorShape::Arrow),
            sprite("tahoe-inverted", CursorShape::Arrow),
        );
        let size = SPRITE_SIZE as usize;
        let at = (((0.3 * size as f64) as usize) * size + (0.2 * size as f64) as usize) * 4;
        assert!(light.rgba[at] > 200, "white body");
        assert!(dark.rgba[at] < 60, "dark body");
    }

    #[test]
    fn a_dot_is_centred_whatever_the_shape() {
        assert_eq!(sprite("dot", CursorShape::Text).hotspot, (0.5, 0.5));
        assert_eq!(sprite("dot", CursorShape::Arrow).hotspot, (0.5, 0.5));
    }

    #[test]
    fn the_shadow_is_soft_and_black() {
        let cursor = sprite("tahoe", CursorShape::Arrow);
        assert!(
            cursor
                .shadow
                .chunks_exact(4)
                .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0)
        );
        let partial = cursor
            .shadow
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 10 && pixel[3] < 200)
            .count();
        assert!(partial > 1000, "the edge should fade");
    }

    #[test]
    fn minimal_has_no_shadow() {
        assert!(!has_shadow("minimal"));
        assert!(has_shadow("tahoe"));
    }

    #[test]
    fn unknown_styles_fall_back_to_the_standard_set() {
        assert_eq!(
            sprite("nonsense", CursorShape::Arrow).rgba,
            sprite("tahoe", CursorShape::Arrow).rgba
        );
    }
}
