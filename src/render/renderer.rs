use crate::render::scene::Scene;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub primitive_count: usize,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[derive(Debug, Default)]
pub struct Renderer {
    width: u32,
    height: u32,
}

impl Renderer {
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn render(&mut self, scene: &Scene) -> FrameStats {
        FrameStats {
            primitive_count: scene.len(),
            viewport_width: self.width,
            viewport_height: self.height,
        }
    }
}
