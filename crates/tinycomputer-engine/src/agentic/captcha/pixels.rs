//! Cutting widgets out of a viewport screenshot and scaling them into the
//! sizes Jev reads, while remembering how to map an answer back to a point
//! in the viewport.

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage, imageops};

use super::scan::Rect;

/// Smallest side Jev reads, in pixels.
const MIN_SIDE: u32 = 280;
/// Largest side Jev reads, in pixels.
const MAX_SIDE: u32 = 1568;

/// A part of the viewport screenshot, and where it sits in the viewport.
#[derive(Debug, Clone)]
pub(super) struct Region {
    image: RgbaImage,
    /// The viewport point, in CSS pixels, of the region's top-left pixel.
    left: f64,
    top: f64,
    /// CSS pixels per screenshot pixel: one over the device pixel ratio.
    css_per_px: f64,
}

/// A region scaled (and padded, if need be) for Jev, as PNG.
#[derive(Debug, Clone)]
pub(super) struct Fitted {
    pub(super) png: Vec<u8>,
    /// Fitted pixels per region pixel.
    scale: f64,
    width: u32,
    height: u32,
}

impl Region {
    /// The whole screenshot `png` of a viewport `viewport_width` CSS pixels
    /// wide; `None` when it is not a readable PNG.
    pub(super) fn screenshot(png: &[u8], viewport_width: f64) -> Option<Self> {
        let image = image::load_from_memory_with_format(png, ImageFormat::Png)
            .ok()?
            .to_rgba8();
        if image.width() == 0 || viewport_width <= 0.0 {
            return None;
        }
        Some(Self {
            css_per_px: viewport_width / f64::from(image.width()),
            image,
            left: 0.0,
            top: 0.0,
        })
    }

    /// The part of this region under `rect`, given in viewport CSS pixels;
    /// `None` when they do not overlap.
    pub(super) fn cut(&self, rect: &Rect) -> Option<Self> {
        let to_px = |css: f64, origin: f64| (css - origin) / self.css_per_px;
        let left = to_px(rect.x, self.left).max(0.0);
        let top = to_px(rect.y, self.top).max(0.0);
        let right = to_px(rect.x + rect.width, self.left).min(f64::from(self.image.width()));
        let bottom = to_px(rect.y + rect.height, self.top).min(f64::from(self.image.height()));
        self.crop_px(left, top, right, bottom)
    }

    /// The part between the fractions `left`..`right` of its width and
    /// `top`..`bottom` of its height.
    pub(super) fn part(&self, left: f64, top: f64, right: f64, bottom: f64) -> Option<Self> {
        let (width, height) = (
            f64::from(self.image.width()),
            f64::from(self.image.height()),
        );
        self.crop_px(left * width, top * height, right * width, bottom * height)
    }

    /// The region cut into `rows` by `columns` equal cells, row by row.
    pub(super) fn cells(&self, rows: u32, columns: u32) -> Vec<Self> {
        let (rows_f, columns_f) = (f64::from(rows), f64::from(columns));
        (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (row, column)))
            .filter_map(|(row, column)| {
                let (row, column) = (f64::from(row), f64::from(column));
                self.part(
                    column / columns_f,
                    row / rows_f,
                    (column + 1.0) / columns_f,
                    (row + 1.0) / rows_f,
                )
            })
            .collect()
    }

    /// The viewport point, in CSS pixels, at the fractions (`x`, `y`) of this
    /// region's width and height.
    pub(super) fn point(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.left + x.clamp(0.0, 1.0) * f64::from(self.image.width()) * self.css_per_px,
            self.top + y.clamp(0.0, 1.0) * f64::from(self.image.height()) * self.css_per_px,
        )
    }

    /// The viewport point at the fractions (`x`, `y`) of `fitted`, the image
    /// [`Region::fit`] made of this region.
    pub(super) fn point_in(&self, fitted: &Fitted, x: f64, y: f64) -> (f64, f64) {
        let to_region = |fraction: f64, fitted_side: u32, side: u32| {
            fraction * f64::from(fitted_side) / fitted.scale / f64::from(side.max(1))
        };
        self.point(
            to_region(x, fitted.width, self.image.width()),
            to_region(y, fitted.height, self.image.height()),
        )
    }

    /// The region as Jev reads it: scaled so its shorter side is at least
    /// 280 pixels and its longer at most 1568, then padded with white on the
    /// right or bottom if a side is still short. `None` when it cannot be
    /// encoded.
    pub(super) fn fit(&self) -> Option<Fitted> {
        let (width, height) = (self.image.width(), self.image.height());
        if width == 0 || height == 0 {
            return None;
        }
        let shorter = f64::from(width.min(height));
        let longer = f64::from(width.max(height));
        let mut scale = 1.0_f64;
        if shorter < f64::from(MIN_SIDE) {
            scale = f64::from(MIN_SIDE) / shorter;
        }
        if longer * scale > f64::from(MAX_SIDE) {
            scale = f64::from(MAX_SIDE) / longer;
        }
        let scaled = |side: u32| side_px((f64::from(side) * scale).round());
        let (scaled_width, scaled_height) = (scaled(width), scaled(height));
        let resized = if (scaled_width, scaled_height) == (width, height) {
            self.image.clone()
        } else {
            imageops::resize(
                &self.image,
                scaled_width,
                scaled_height,
                imageops::FilterType::Triangle,
            )
        };
        let (out_width, out_height) = (scaled_width.max(MIN_SIDE), scaled_height.max(MIN_SIDE));
        let mut canvas = RgbaImage::from_pixel(out_width, out_height, Rgba([255, 255, 255, 255]));
        imageops::replace(&mut canvas, &resized, 0, 0);
        let mut png = Vec::new();
        canvas
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .ok()?;
        Some(Fitted {
            png,
            scale: f64::from(scaled_width) / f64::from(width),
            width: out_width,
            height: out_height,
        })
    }

    fn crop_px(&self, left: f64, top: f64, right: f64, bottom: f64) -> Option<Self> {
        let (x, y) = (side_px(left.floor()), side_px(top.floor()));
        let (end_x, end_y) = (side_px(right.ceil()), side_px(bottom.ceil()));
        let end_x = end_x.min(self.image.width());
        let end_y = end_y.min(self.image.height());
        if end_x <= x || end_y <= y {
            return None;
        }
        let image = imageops::crop_imm(&self.image, x, y, end_x - x, end_y - y).to_image();
        Some(Self {
            image,
            left: self.left + f64::from(x) * self.css_per_px,
            top: self.top + f64::from(y) * self.css_per_px,
            css_per_px: self.css_per_px,
        })
    }
}

/// A non-negative whole number of pixels, saturating.
fn side_px(value: f64) -> u32 {
    if value.is_nan() || value <= 0.0 {
        0
    } else if value >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        // In range and whole: the conversion is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pixels = value as u32;
        pixels
    }
}
