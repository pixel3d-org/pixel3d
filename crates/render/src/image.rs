//! RGBA images on the CPU: text, lines, rectangles, tiling into contact
//! sheets, part labels, and PNG encoding.

use crate::font::{GLYPH_H, GLYPH_W, glyph, ink};
use crate::scene::PartLabel;

/// An 8-bit sRGB RGBA color.
pub type Rgba = [u8; 4];

pub const INK: Rgba = [32, 36, 44, 255];
pub const PAPER: Rgba = [250, 250, 248, 255];

/// Labels drawn per image at most (largest parts first).
pub const MAX_LABELS: usize = 24;

/// An RGBA image, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32, color: Rgba) -> Image {
        let pixels = color.iter().copied().cycle().take(width as usize * height as usize * 4).collect();
        Image { width, height, pixels }
    }

    pub fn get(&self, x: u32, y: u32) -> Rgba {
        let k = (y as usize * self.width as usize + x as usize) * 4;
        [self.pixels[k], self.pixels[k + 1], self.pixels[k + 2], self.pixels[k + 3]]
    }

    #[inline]
    pub fn put(&mut self, x: i64, y: i64, c: Rgba) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let k = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels[k..k + 4].copy_from_slice(&c);
    }

    /// Fills a rectangle (clipped).
    ///
    /// Complexity: O(w x h).
    pub fn fill_rect(&mut self, x: i64, y: i64, w: i64, h: i64, c: Rgba) {
        for yy in y.max(0)..(y + h).min(i64::from(self.height)) {
            for xx in x.max(0)..(x + w).min(i64::from(self.width)) {
                self.put(xx, yy, c);
            }
        }
    }

    /// A one-pixel line (Bresenham).
    ///
    /// Complexity: O(length).
    pub fn line(&mut self, (x0, y0): (i64, i64), (x1, y1): (i64, i64), c: Rgba) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        // Lines longer than the image are clipped by `put`; cap the walk anyway.
        for _ in 0..=(dx - dy).clamp(0, 1 << 16) {
            self.put(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Size of a text in pixels at `scale` font pixels per glyph pixel.
    pub fn text_size(text: &str, scale: u32) -> (i64, i64) {
        let n = text.chars().count() as i64;
        (n * (GLYPH_W as i64 + 1) * i64::from(scale) - i64::from(scale), GLYPH_H as i64 * i64::from(scale))
    }

    /// Draws text with its top-left corner at (x, y).
    ///
    /// Complexity: O(characters x scale^2).
    pub fn text(&mut self, x: i64, y: i64, text: &str, scale: u32, c: Rgba) {
        let s = i64::from(scale);
        for (i, ch) in text.chars().enumerate() {
            let g = glyph(ch);
            let ox = x + i as i64 * (GLYPH_W as i64 + 1) * s;
            for gy in 0..GLYPH_H {
                for gx in 0..GLYPH_W {
                    if ink(&g, gx, gy) {
                        self.fill_rect(ox + gx as i64 * s, y + gy as i64 * s, s, s, c);
                    }
                }
            }
        }
    }

    /// Copies `other` with its top-left corner at (x, y).
    ///
    /// Complexity: O(pixels of `other`).
    pub fn blit(&mut self, other: &Image, x: u32, y: u32) {
        for row in 0..other.height.min(self.height.saturating_sub(y)) {
            let w = other.width.min(self.width.saturating_sub(x)) as usize * 4;
            let src = row as usize * other.width as usize * 4;
            let dst = ((y + row) as usize * self.width as usize + x as usize) * 4;
            self.pixels[dst..dst + w].copy_from_slice(&other.pixels[src..src + w]);
        }
    }

    /// PNG bytes (8-bit sRGB RGBA).
    ///
    /// Complexity: O(pixels).
    pub fn to_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            let mut w = enc.write_header()?;
            w.write_image_data(&self.pixels)?;
        }
        Ok(out)
    }
}

type Rect = (i64, i64, i64, i64);

fn overlaps(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Draws each visible part's name in a box with a leader line to its
/// anchor (a visible pixel of the part: `(x, y, visible pixels)` per part,
/// `None` when hidden), most visible parts first, placing boxes where they
/// do not cover earlier ones.
///
/// Complexity: O(labels^2) for at most [`MAX_LABELS`] labels, plus the
/// pixels drawn.
pub fn draw_labels(img: &mut Image, anchors: &[Option<(i64, i64, usize)>], parts: &[PartLabel], scale: u32) {
    let mut order: Vec<usize> = (0..parts.len().min(anchors.len())).filter(|&i| anchors[i].is_some()).collect();
    let seen = |i: usize| anchors[i].map_or(0, |a| a.2);
    order.sort_by(|&a, &b| seen(b).cmp(&seen(a)).then(a.cmp(&b)));
    const { assert!(MAX_LABELS <= 64, "label placement is quadratic in labels") };
    let mut placed: Vec<Rect> = Vec::new();
    let (w, h) = (i64::from(img.width), i64::from(img.height));
    let pad = i64::from(scale) * 2;
    for &i in order.iter().take(MAX_LABELS) {
        let p = &parts[i];
        let Some((ax, ay, _)) = anchors[i] else { continue };
        let (tw, th) = Image::text_size(&p.name, scale);
        let (bw, bh) = (tw + 2 * pad, th + 2 * pad);
        let gap = 10 * i64::from(scale);
        let candidates: Vec<Rect> = [1, 2, 3]
            .iter()
            .flat_map(|&k| {
                let g = gap * k;
                [(ax + g, ay - g - bh), (ax - g - bw, ay - g - bh), (ax + g, ay + g), (ax - g - bw, ay + g)]
            })
            .map(|(x, y)| (x, y, bw, bh))
            .collect();
        let inside = |r: &Rect| r.0 >= 0 && r.1 >= 0 && r.0 + r.2 <= w && r.1 + r.3 <= h;
        let Some(r) = candidates
            .iter()
            .find(|r| inside(r) && !placed.iter().any(|q| overlaps(**r, *q)))
            .or_else(|| candidates.iter().find(|r| inside(r)))
            .copied()
        else {
            continue;
        };
        // Leader from the part's center to the nearest corner of the box.
        let corner = (if r.0 > ax { r.0 } else { r.0 + r.2 - 1 }, if r.1 > ay { r.1 } else { r.1 + r.3 - 1 });
        img.line((ax, ay), corner, INK);
        img.fill_rect(ax - 1, ay - 1, 3, 3, INK);
        img.fill_rect(r.0, r.1, r.2, r.3, INK);
        img.fill_rect(r.0 + 1, r.1 + 1, r.2 - 2, r.3 - 2, PAPER);
        img.text(r.0 + pad, r.1 + pad, &p.name, scale, INK);
        placed.push(r);
    }
}

/// Tiles images into a grid (columns chosen to keep it about square), each
/// tile captioned with its title.
///
/// Complexity: O(total pixels).
pub fn contact_sheet(tiles: &[(String, Image)], scale: u32) -> Image {
    let n = tiles.len().max(1);
    let cols = (n as f64).sqrt().ceil() as u32;
    let rows = (n as u32).div_ceil(cols);
    let (tw, th) = tiles.first().map_or((1, 1), |(_, i)| (i.width, i.height));
    let gap = 2;
    let mut sheet = Image::new(cols * tw + (cols - 1) * gap, rows * th + (rows - 1) * gap, INK);
    for (k, (title, img)) in tiles.iter().enumerate() {
        let (cx, cy) = ((k as u32 % cols) * (tw + gap), (k as u32 / cols) * (th + gap));
        sheet.blit(img, cx, cy);
        let (w, h) = Image::text_size(title, scale);
        let pad = i64::from(scale) * 2;
        sheet.fill_rect(i64::from(cx), i64::from(cy), w + 2 * pad, h + 2 * pad, INK);
        sheet.text(i64::from(cx) + pad, i64::from(cy) + pad, title, scale, PAPER);
    }
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_and_png() {
        let mut img = Image::new(40, 20, PAPER);
        img.text(1, 1, "Hi", 1, INK);
        assert_eq!(img.get(1, 1), INK, "H starts with ink at its top-left");
        assert_eq!(img.get(2, 1), PAPER);
        img.line((0, 19), (39, 19), INK);
        assert_eq!(img.get(39, 19), INK);
        img.put(-5, 100, INK);
        let png = img.to_png().unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(Image::text_size("ab", 2), (22, 14));
    }

    #[test]
    fn sheets_tile_in_a_grid() {
        let tiles: Vec<(String, Image)> = (0..3).map(|i| (format!("v{i}"), Image::new(10, 8, PAPER))).collect();
        let s = contact_sheet(&tiles, 1);
        assert_eq!((s.width, s.height), (22, 18));
        assert_eq!(s.get(11, 9), INK, "the gap between tiles");
    }
}
