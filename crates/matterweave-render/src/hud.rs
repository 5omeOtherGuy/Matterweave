use bytemuck::{Pod, Zeroable};
use font8x8::UnicodeFonts;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct HudVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}
/// Small immediate HUD. Positions are logical pixels supplied by the application.
pub struct Hud {
    pub(crate) vertices: Vec<HudVertex>,
    size: [f32; 2],
}
impl Hud {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            vertices: Vec::new(),
            size: [width.max(1.0), height.max(1.0)],
        }
    }
    pub fn rect(&mut self, rect: [f32; 4], color: [f32; 4]) {
        let [x, y, w, h] = rect;
        for [px, py] in [
            [x, y],
            [x, y + h],
            [x + w, y],
            [x + w, y],
            [x, y + h],
            [x + w, y + h],
        ] {
            self.vertices.push(HudVertex {
                position: [px / self.size[0] * 2.0 - 1.0, 1.0 - py / self.size[1] * 2.0],
                color,
            });
        }
    }
    pub fn text(&mut self, x: f32, y: f32, text: &str, scale: f32, color: [f32; 4]) {
        for (i, ch) in text.chars().enumerate() {
            if let Some(glyph) = font8x8::BASIC_FONTS.get(ch) {
                for (row, bits) in glyph.into_iter().enumerate() {
                    for col in 0..8 {
                        if bits & (1 << col) != 0 {
                            self.rect(
                                [
                                    x + (i * 8 + col) as f32 * scale,
                                    y + row as f32 * scale,
                                    scale,
                                    scale,
                                ],
                                color,
                            );
                        }
                    }
                }
            }
        }
    }
}
