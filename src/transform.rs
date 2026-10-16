use crate::camera::Frame;

/// Rotation is clockwise. Mirroring is applied after rotation, in image coordinates.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ImageTransform {
    pub quarter_turns: u8,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
}

impl ImageTransform {
    pub fn is_identity(self) -> bool {
        self.quarter_turns == 0 && !self.flip_horizontal && !self.flip_vertical
    }

    pub fn dimensions(self, width: u32, height: u32) -> (u32, u32) {
        if self.quarter_turns.is_multiple_of(2) {
            (width, height)
        } else {
            (height, width)
        }
    }

    pub fn source(self, x: u32, y: u32, width: u32, height: u32) -> (u32, u32) {
        let (output_width, output_height) = self.dimensions(width, height);
        let x = if self.flip_horizontal {
            output_width - 1 - x
        } else {
            x
        };
        let y = if self.flip_vertical {
            output_height - 1 - y
        } else {
            y
        };
        match self.quarter_turns {
            0 => (x, y),
            1 => (y, height - 1 - x),
            2 => (width - 1 - x, height - 1 - y),
            3 => (width - 1 - y, x),
            _ => unreachable!("validated rotation"),
        }
    }

    pub fn apply(self, frame: &Frame) -> Frame {
        let (width, height) = self.dimensions(frame.width, frame.height);
        let mut rgb = vec![0; frame.rgb.len()];
        for (index, pixel) in rgb.as_chunks_mut::<3>().0.iter_mut().enumerate() {
            let x = index as u32 % width;
            let y = index as u32 / width;
            let (source_x, source_y) = self.source(x, y, frame.width, frame.height);
            let source = (source_y as usize * frame.width as usize + source_x as usize) * 3;
            pixel.copy_from_slice(&frame.rgb[source..source + 3]);
        }
        Frame { width, height, rgb }
    }
}
