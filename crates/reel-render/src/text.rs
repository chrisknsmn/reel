//! Text, shaped and rasterised on the CPU, so preview and export can't draw it differently.

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight,
};
use reel_core::timeline::{TextAlign, TextStyle};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

use crate::gradient::parse_color;

/// Holds the font database, which is slow to build, so make one and keep it.
pub struct TextPainter {
    fonts: FontSystem,
    glyphs: SwashCache,
}

impl Default for TextPainter {
    fn default() -> Self {
        Self::new()
    }
}

/// CSS font stacks name several in order. The first this machine has is used.
fn family_of(stack: &str) -> Option<&str> {
    stack
        .split(',')
        .map(|name| name.trim().trim_matches(['"', '\'']))
        .find(|name| {
            !matches!(
                name.to_ascii_lowercase().as_str(),
                "system-ui" | "sans-serif" | "serif" | "monospace" | ""
            )
        })
}

/// A rounded rectangle as a path, or `None` if it has no size.
fn rounded_rect(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Option<tiny_skia::Path> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let r = radius.clamp(0.0, width.min(height) / 2.0);
    if r <= 0.0 {
        return Rect::from_xywh(x, y, width, height).map(PathBuilder::from_rect);
    }
    // Bézier corners with the usual constant for a quarter circle.
    let c = r * 0.552_284_8;
    let (right, bottom) = (x + width, y + height);
    let mut builder = PathBuilder::new();
    builder.move_to(x + r, y);
    builder.line_to(right - r, y);
    builder.cubic_to(right - r + c, y, right, y + r - c, right, y + r);
    builder.line_to(right, bottom - r);
    builder.cubic_to(
        right,
        bottom - r + c,
        right - r + c,
        bottom,
        right - r,
        bottom,
    );
    builder.line_to(x + r, bottom);
    builder.cubic_to(x + r - c, bottom, x, bottom - r + c, x, bottom - r);
    builder.line_to(x, y + r);
    builder.cubic_to(x, y + r - c, x + r - c, y, x + r, y);
    builder.close();
    builder.finish()
}

/// Lays out and draws text, returning premultiplied RGBA.
pub struct TextImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl TextPainter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
        }
    }

    fn shape(
        &mut self,
        text: &str,
        style: &TextStyle,
        font_px: f32,
        line_height: f32,
        width: Option<f32>,
    ) -> Buffer {
        let mut buffer = Buffer::new(
            &mut self.fonts,
            Metrics::new(font_px, font_px * line_height),
        );
        buffer.set_size(width, None);
        let family = family_of(&style.font_family);
        let attrs = Attrs::new()
            .family(family.map_or(Family::SansSerif, Family::Name))
            .weight(if style.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            })
            .style(if style.italic {
                Style::Italic
            } else {
                Style::Normal
            });
        let align = match style.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        };
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    /// Text in a box of a given size, with its background, as an annotation has it.
    pub fn paint_box(
        &mut self,
        text: &str,
        style: &TextStyle,
        size: (u32, u32),
        font_scale: f32,
    ) -> TextImage {
        let (width, height) = (size.0.max(1), size.1.max(1));
        let mut pixmap = Pixmap::new(width, height).expect("the size is at least one pixel");
        let background = parse_color(&style.background).unwrap_or([0.0; 4]);
        if background[3] > 0.0
            && let Some(path) = rounded_rect(
                0.0,
                0.0,
                width as f32,
                height as f32,
                style.corner_radius as f32 * font_scale,
            )
        {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            paint.set_color_rgba8(
                (background[0] * 255.0) as u8,
                (background[1] * 255.0) as u8,
                (background[2] * 255.0) as u8,
                (background[3] * 255.0) as u8,
            );
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        let font_px = (style.font_size as f32 * font_scale).max(1.0);
        let padding = 8.0 * font_scale;
        let mut buffer = self.shape(
            text,
            style,
            font_px,
            1.4,
            Some((width as f32 - padding * 2.0).max(1.0)),
        );
        // Text sits in the middle of its box, as it does in Recordly.
        let total: f32 = buffer.layout_runs().map(|run| run.line_height).sum();
        let top = ((height as f32 - total) / 2.0).max(0.0);
        let color = parse_color(&style.color).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        self.draw(&mut buffer, &mut pixmap, color, (padding, top));
        if style.underline {
            let runs: Vec<(f32, f32, f32)> = buffer
                .layout_runs()
                .map(|run| (run.line_w, run.line_y, run.line_height))
                .collect();
            let mut paint = Paint::default();
            paint.set_color_rgba8(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
                255,
            );
            for (line_width, y, _) in runs {
                let inner = width as f32 - padding * 2.0;
                let x = padding
                    + match style.align {
                        TextAlign::Left => 0.0,
                        TextAlign::Center => (inner - line_width) / 2.0,
                        TextAlign::Right => inner - line_width,
                    };
                if let Some(rect) = Rect::from_xywh(
                    x,
                    top + y + font_px * 0.12,
                    line_width,
                    (font_px / 16.0).max(1.0),
                ) {
                    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                }
            }
        }
        TextImage {
            width,
            height,
            rgba: pixmap.data().to_vec(),
        }
    }

    /// Lines of a caption in a rounded box sized to fit them. The `dim` colour is for words not yet
    /// spoken, which isn't used yet.
    pub fn paint_caption(
        &mut self,
        lines: &[String],
        style: &TextStyle,
        font_px: f32,
        padding: (f32, f32),
        radius: f32,
        background_alpha: f32,
    ) -> TextImage {
        let text = lines.join("\n");
        let mut buffer = self.shape(&text, style, font_px, 1.32, None);
        let content_width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        let content_height: f32 = buffer.layout_runs().map(|run| run.line_height).sum();
        let width = (content_width + padding.0 * 2.0).ceil().max(1.0) as u32;
        let height = (content_height + padding.1 * 2.0).ceil().max(1.0) as u32;
        buffer.set_size(Some(content_width.ceil()), None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let mut pixmap = Pixmap::new(width, height).expect("the size is at least one pixel");
        if let Some(path) = rounded_rect(0.0, 0.0, width as f32, height as f32, radius) {
            let mut paint = Paint {
                anti_alias: true,
                ..Paint::default()
            };
            paint.set_color_rgba8(0, 0, 0, (background_alpha.clamp(0.0, 1.0) * 255.0) as u8);
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        let color = parse_color(&style.color).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        self.draw(&mut buffer, &mut pixmap, color, padding);
        TextImage {
            width,
            height,
            rgba: pixmap.data().to_vec(),
        }
    }

    /// How wide a word is, for working out where lines wrap.
    pub fn measure(&mut self, text: &str, style: &TextStyle, font_px: f32) -> f32 {
        let buffer = self.shape(text, style, font_px, 1.32, None);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max)
    }

    fn draw(
        &mut self,
        buffer: &mut Buffer,
        pixmap: &mut Pixmap,
        color: [f32; 4],
        offset: (f32, f32),
    ) {
        let (width, height) = (i64::from(pixmap.width()), i64::from(pixmap.height()));
        let (ox, oy) = (offset.0.round() as i64, offset.1.round() as i64);
        let pixels = pixmap.data_mut();
        let base = Color::rgba(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
            (color[3] * 255.0) as u8,
        );
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            base,
            |x, y, w, h, drawn| {
                let alpha = u32::from(drawn.a());
                if alpha == 0 {
                    return;
                }
                for py in 0..i64::from(h) {
                    for px in 0..i64::from(w) {
                        let (tx, ty) = (i64::from(x) + px + ox, i64::from(y) + py + oy);
                        if tx < 0 || ty < 0 || tx >= width || ty >= height {
                            continue;
                        }
                        let at = (ty * width + tx) as usize * 4;
                        let inverse = 255 - alpha;
                        let over = |source: u8, existing: u8| {
                            ((u32::from(source) * alpha + u32::from(existing) * inverse) / 255)
                                as u8
                        };
                        pixels[at] = over(drawn.r(), pixels[at]);
                        pixels[at + 1] = over(drawn.g(), pixels[at + 1]);
                        pixels[at + 2] = over(drawn.b(), pixels[at + 2]);
                        pixels[at + 3] = (alpha + u32::from(pixels[at + 3]) * inverse / 255) as u8;
                    }
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(image: &TextImage) -> usize {
        image
            .rgba
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 128)
            .count()
    }

    #[test]
    fn text_draws_inside_its_box() {
        let mut painter = TextPainter::new();
        let image = painter.paint_box("Hello", &TextStyle::default(), (300, 100), 1.0);
        assert_eq!((image.width, image.height), (300, 100));
        assert!(opaque(&image) > 100, "the words should be there");
    }

    #[test]
    fn a_background_fills_the_box() {
        let mut painter = TextPainter::new();
        let style = TextStyle {
            background: "#000000".to_string(),
            corner_radius: 0.0,
            ..TextStyle::default()
        };
        let image = painter.paint_box("", &style, (100, 50), 1.0);
        assert_eq!(opaque(&image), 5000);
    }

    #[test]
    fn rounded_corners_cut_the_box() {
        let mut painter = TextPainter::new();
        let style = TextStyle {
            background: "#000000".to_string(),
            corner_radius: 20.0,
            ..TextStyle::default()
        };
        let image = painter.paint_box("", &style, (100, 100), 1.0);
        assert!(opaque(&image) < 10_000);
        assert_eq!(image.rgba[3], 0, "the very corner is clear");
    }

    #[test]
    fn longer_text_wraps_to_more_lines_rather_than_overflowing() {
        let mut painter = TextPainter::new();
        let style = TextStyle::default();
        let one = painter.paint_box("word", &style, (200, 400), 1.0);
        let many = painter.paint_box(
            "word word word word word word word word word word",
            &style,
            (200, 400),
            1.0,
        );
        assert!(opaque(&many) > opaque(&one) * 3);
    }

    #[test]
    fn alignment_moves_the_text() {
        let mut painter = TextPainter::new();
        let at = |painter: &mut TextPainter, align| {
            let style = TextStyle {
                align,
                ..TextStyle::default()
            };
            let image = painter.paint_box("Hi", &style, (400, 100), 1.0);
            image
                .rgba
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, pixel)| pixel[3] > 128)
                .map(|(index, _)| index % 400)
                .min()
                .unwrap_or(0)
        };
        let (left, center, right) = (
            at(&mut painter, TextAlign::Left),
            at(&mut painter, TextAlign::Center),
            at(&mut painter, TextAlign::Right),
        );
        assert!(left < center && center < right, "{left} {center} {right}");
    }

    #[test]
    fn font_stacks_pick_the_first_named_font() {
        assert_eq!(
            family_of("\"SF Pro Display\", \"Helvetica Neue\", sans-serif"),
            Some("SF Pro Display")
        );
        assert_eq!(family_of("system-ui"), None);
        assert_eq!(family_of("sans-serif"), None);
    }

    #[test]
    fn a_caption_box_fits_its_text() {
        let mut painter = TextPainter::new();
        let style = TextStyle {
            bold: false,
            ..TextStyle::default()
        };
        let short =
            painter.paint_caption(&["Hi".to_string()], &style, 30.0, (33.0, 23.4), 17.5, 0.9);
        let long = painter.paint_caption(
            &["A much longer line".to_string()],
            &style,
            30.0,
            (33.0, 23.4),
            17.5,
            0.9,
        );
        let two = painter.paint_caption(
            &["Hi".to_string(), "there".to_string()],
            &style,
            30.0,
            (33.0, 23.4),
            17.5,
            0.9,
        );
        assert!(long.width > short.width);
        assert!(two.height > short.height);
    }

    #[test]
    fn measuring_grows_with_the_text() {
        let mut painter = TextPainter::new();
        let style = TextStyle::default();
        assert!(
            painter.measure("a long sentence", &style, 30.0) > painter.measure("a", &style, 30.0)
        );
    }
}
