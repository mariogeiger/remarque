#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BgraImage {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelRectangle {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl PixelRectangle {
    pub const fn full(width: usize, height: usize) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    pub fn include(self, other: Self) -> Self {
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }
}

impl BgraImage {
    pub fn try_from_bgra(
        width: usize,
        height: usize,
        pixels: Vec<u8>,
    ) -> Result<Self, &'static str> {
        if pixels.len() != width * height * 4 {
            return Err("BGRA byte count does not match the image dimensions");
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn filled(width: usize, height: usize, rgb: [u8; 3]) -> Self {
        let mut pixels = vec![0; width * height * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[rgb[2], rgb[1], rgb[0], 0xff]);
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * self.width + x) * 4;
        self.pixels[offset..offset + 4].try_into().unwrap()
    }

    pub fn fill_rectangle(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        rgb: [u8; 3],
    ) {
        let right = (x + width).min(self.width);
        let bottom = (y + height).min(self.height);
        for row in y.min(self.height)..bottom {
            for column in x.min(self.width)..right {
                let offset = (row * self.width + column) * 4;
                self.pixels[offset..offset + 4].copy_from_slice(&[rgb[2], rgb[1], rgb[0], 0xff]);
            }
        }
    }

    pub fn fill_rounded_rectangle(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        radius: f32,
        rgb: [u8; 3],
    ) {
        if radius <= 0.0 {
            self.fill_rectangle(x, y, width, height, rgb);
            return;
        }
        let right = (x + width).min(self.width);
        let bottom = (y + height).min(self.height);
        let radius = radius
            .max(0.0)
            .min((right.saturating_sub(x) as f32) * 0.5)
            .min((bottom.saturating_sub(y) as f32) * 0.5);
        for row in y.min(bottom)..bottom {
            for column in x.min(right)..right {
                let center_x = column as f32 + 0.5;
                let center_y = row as f32 + 0.5;
                let nearest_x = center_x.clamp(x as f32 + radius, right as f32 - radius);
                let nearest_y = center_y.clamp(y as f32 + radius, bottom as f32 - radius);
                let distance = (center_x - nearest_x).hypot(center_y - nearest_y);
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.blend_rgb_opacity(column, row, rgb, coverage);
                }
            }
        }
    }

    pub fn draw_circle_outline(
        &mut self,
        center_x: f32,
        center_y: f32,
        diameter: f32,
        rgb: [u8; 3],
    ) {
        let radius = diameter * 0.5;
        let outer_radius = radius + 1.0;
        let left = (center_x - outer_radius).floor().max(0.0) as usize;
        let right = (center_x + outer_radius).ceil().min(self.width as f32) as usize;
        let top = (center_y - outer_radius).floor().max(0.0) as usize;
        let bottom = (center_y + outer_radius).ceil().min(self.height as f32) as usize;
        for y in top..bottom {
            for x in left..right {
                let distance = (x as f32 + 0.5 - center_x).hypot(y as f32 + 0.5 - center_y);
                let coverage = (1.5 - (distance - radius).abs()).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.blend_rgb_opacity(x, y, rgb, coverage);
                }
            }
        }
    }

    pub fn copy_rectangle(&self, x: usize, y: usize, width: usize, height: usize) -> Vec<u8> {
        let right = (x + width).min(self.width);
        let bottom = (y + height).min(self.height);
        let mut copy = Vec::with_capacity((right - x.min(right)) * (bottom - y.min(bottom)) * 4);
        for row in y.min(bottom)..bottom {
            let start = (row * self.width + x.min(right)) * 4;
            let end = (row * self.width + right) * 4;
            copy.extend_from_slice(&self.pixels[start..end]);
        }
        copy
    }

    pub fn copy_bgra_rectangle(
        &mut self,
        destination_x: usize,
        destination_y: usize,
        width: usize,
        height: usize,
        source_stride: usize,
        source: &[u8],
    ) -> Result<(), &'static str> {
        let row_bytes = width
            .checked_mul(4)
            .ok_or("BGRA rectangle dimensions overflow")?;
        let source_bytes = source_stride
            .checked_mul(height)
            .ok_or("BGRA source dimensions overflow")?;
        if source_stride < row_bytes || source.len() < source_bytes {
            return Err("BGRA source is smaller than the rectangle");
        }
        if destination_x
            .checked_add(width)
            .is_none_or(|right| right > self.width)
            || destination_y
                .checked_add(height)
                .is_none_or(|bottom| bottom > self.height)
        {
            return Err("BGRA rectangle exceeds the destination image");
        }
        for row in 0..height {
            let source_start = row * source_stride;
            let destination_start = ((destination_y + row) * self.width + destination_x) * 4;
            self.pixels[destination_start..destination_start + row_bytes]
                .copy_from_slice(&source[source_start..source_start + row_bytes]);
        }
        Ok(())
    }

    pub fn restore_rectangle(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        copy: &[u8],
    ) {
        let right = (x + width).min(self.width);
        let bottom = (y + height).min(self.height);
        let row_bytes = (right - x.min(right)) * 4;
        assert_eq!(copy.len(), row_bytes * (bottom - y.min(bottom)));
        for (source_row, row) in (y.min(bottom)..bottom).enumerate() {
            let start = (row * self.width + x.min(right)) * 4;
            self.pixels[start..start + row_bytes]
                .copy_from_slice(&copy[source_row * row_bytes..(source_row + 1) * row_bytes]);
        }
    }

    pub fn blend_rgb_coverage(&mut self, x: usize, y: usize, rgb: [u8; 3], coverage: u8) {
        self.blend_bgra_coverage(x, y, [rgb[2], rgb[1], rgb[0], u8::MAX], coverage);
    }

    fn blend_rgb_opacity(&mut self, x: usize, y: usize, rgb: [u8; 3], opacity: f32) {
        let offset = (y * self.width + x) * 4;
        let opacity = opacity.clamp(0.0, 1.0);
        for (channel, source) in [rgb[2], rgb[1], rgb[0]].into_iter().enumerate() {
            let destination = f32::from(self.pixels[offset + channel]);
            self.pixels[offset + channel] =
                (f32::from(source) * opacity + destination * (1.0 - opacity)).round() as u8;
        }
    }

    pub(crate) fn blend_bgra_coverage(&mut self, x: usize, y: usize, bgra: [u8; 4], coverage: u8) {
        if coverage == 0 {
            return;
        }
        let offset = (y * self.width + x) * 4;
        if coverage == u8::MAX && bgra[3] == u8::MAX {
            self.pixels[offset..offset + 4].copy_from_slice(&bgra);
            return;
        }
        let source_alpha = u16::from(bgra[3]) * u16::from(coverage) / 255;
        let destination_weight = 255 - source_alpha;
        for (channel, source) in bgra.into_iter().enumerate() {
            let source = u16::from(source) * u16::from(coverage) / 255;
            let product = u16::from(self.pixels[offset + channel]) * destination_weight;
            let destination = (product + ((product + 0x101) >> 8)) >> 8;
            self.pixels[offset + channel] = (source + destination) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_outline_leaves_its_center_untouched() {
        let mut image = BgraImage::filled(20, 20, [255, 255, 255]);
        image.draw_circle_outline(10.0, 10.0, 10.0, [0, 0, 0]);
        assert_eq!(image.pixel(10, 10), [255, 255, 255, 255]);
        assert!(image.pixel(10, 4)[0] <= 8);
    }

    #[test]
    fn rounded_rectangle_fills_its_center_and_leaves_outer_corners_untouched() {
        let mut image = BgraImage::filled(12, 12, [255, 255, 255]);
        image.fill_rounded_rectangle(1, 1, 10, 10, 4.0, [0, 0, 0]);
        assert_eq!(image.pixel(6, 6), [0, 0, 0, 255]);
        assert_eq!(image.pixel(1, 1), [255, 255, 255, 255]);
    }

    #[test]
    fn coverage_blends_rgb_into_the_destination() {
        let mut image = BgraImage::filled(1, 1, [255, 255, 255]);
        image.blend_rgb_coverage(0, 0, [0, 0, 0], 128);
        assert_eq!(image.pixel(0, 0), [127, 127, 127, 255]);
    }

    #[test]
    fn rejects_bgra_with_the_wrong_byte_count() {
        assert!(BgraImage::try_from_bgra(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn copies_strided_bgra_into_a_rectangle() {
        let mut image = BgraImage::filled(3, 2, [255, 255, 255]);
        image
            .copy_bgra_rectangle(
                1,
                0,
                2,
                2,
                12,
                &[
                    1, 2, 3, 4, 5, 6, 7, 8, 99, 99, 99, 99, 9, 10, 11, 12, 13, 14, 15, 16, 99, 99,
                    99, 99,
                ],
            )
            .unwrap();
        assert_eq!(image.pixel(1, 0), [1, 2, 3, 4]);
        assert_eq!(image.pixel(2, 1), [13, 14, 15, 16]);
    }
}
