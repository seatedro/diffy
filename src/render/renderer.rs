use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use thiserror::Error;
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

use crate::render::scene::{
    BorderPrimitive, ClipPrimitive, FontKind, Primitive, Rect, RoundedRectPrimitive, Scene,
    ShadowPrimitive, TextPrimitive,
};
use crate::ui::theme::Color;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub primitive_count: usize,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("no compatible GPU adapter found")]
    NoAdapter,
    #[error("failed to create surface: {0}")]
    CreateSurface(#[from] wgpu::CreateSurfaceError),
    #[error("failed to request device: {0}")]
    RequestDevice(#[from] wgpu::RequestDeviceError),
    #[error("failed to prepare text: {0}")]
    PrepareText(#[from] glyphon::PrepareError),
    #[error("failed to render text: {0}")]
    RenderText(#[from] glyphon::RenderError),
    #[error("surface acquisition failed")]
    SurfaceAcquire,
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    scale_factor: f64,
    rect_pipeline: wgpu::RenderPipeline,
    viewport_buffer: wgpu::Buffer,
    viewport_bind_group: wgpu::BindGroup,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Result<Self, RenderError> {
        pollster::block_on(Self::new_async(window))
    }

    async fn new_async(window: Arc<Window>) -> Result<Self, RenderError> {
        let size = window.inner_size();
        let scale_factor = window.scale_factor();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone())?;
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..wgpu::RequestAdapterOptions::default()
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(_) => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..wgpu::RequestAdapterOptions::default()
                })
                .await
                .map_err(|_| RenderError::NoAdapter)?,
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;

        let surface_capabilities = surface.get_capabilities(&adapter);
        let surface_format = surface_capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_capabilities.formats[0]);
        let surface_config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .unwrap_or(wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: size.width.max(1),
                height: size.height.max(1),
                desired_maximum_frame_latency: 2,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
            });
        surface.configure(&device, &surface_config);

        let viewport_uniform = ViewportUniform::new(surface_config.width, surface_config.height);
        let viewport_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("diffy_viewport_uniform"),
            contents: bytemuck::bytes_of(&viewport_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let viewport_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("diffy_viewport_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let viewport_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("diffy_viewport_bind_group"),
            layout: &viewport_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diffy_rect_shader"),
            source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
        });
        let rect_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("diffy_rect_pipeline_layout"),
            bind_group_layouts: &[&viewport_bind_group_layout],
            immediate_size: 0,
        });
        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("diffy_rect_pipeline"),
            layout: Some(&rect_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[RectVertex::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let glyph_cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &glyph_cache);
        let mut atlas = TextAtlas::new(&device, &queue, &glyph_cache, surface_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        font_system.db_mut().set_monospace_family("Consolas");

        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
            size,
            scale_factor,
            rect_pipeline,
            viewport_buffer,
            viewport_bind_group,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32, scale_factor: f64) {
        if width == 0 || height == 0 {
            self.size = PhysicalSize::new(width, height);
            self.scale_factor = scale_factor;
            return;
        }

        self.size = PhysicalSize::new(width, height);
        self.scale_factor = scale_factor;
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        self.queue.write_buffer(
            &self.viewport_buffer,
            0,
            bytemuck::bytes_of(&ViewportUniform::new(width, height)),
        );
    }

    pub fn render(&mut self, scene: &Scene) -> Result<FrameStats, RenderError> {
        if self.surface_config.width == 0 || self.surface_config.height == 0 {
            return Ok(FrameStats::default());
        }

        let flattened = flatten_scene(
            scene,
            Rect {
                x: 0.0,
                y: 0.0,
                width: self.surface_config.width as f32,
                height: self.surface_config.height as f32,
            },
        );

        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.surface_config);
                return Err(RenderError::SurfaceAcquire);
            }
            Err(wgpu::SurfaceError::Timeout) => return Err(RenderError::SurfaceAcquire),
            Err(_) => return Err(RenderError::SurfaceAcquire),
        };

        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.surface_config.width,
                height: self.surface_config.height,
            },
        );

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("diffy_frame_encoder"),
            });

        let (rect_vertices, draw_commands) = build_rect_vertices(&flattened.rects);
        let rect_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("diffy_rect_vertices"),
                contents: bytemuck::cast_slice(&rect_vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let mut text_buffers = Vec::with_capacity(flattened.texts.len());
        for text in &flattened.texts {
            let metrics = Metrics::new(text.primitive.font_size, text.primitive.font_size * 1.35);
            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            buffer.set_size(
                &mut self.font_system,
                Some((text.primitive.rect.width * self.scale_factor as f32).max(1.0)),
                Some((text.primitive.rect.height * self.scale_factor as f32).max(1.0)),
            );
            let attrs = match text.primitive.font_kind {
                FontKind::Ui => Attrs::new().family(Family::SansSerif),
                FontKind::Mono => Attrs::new().family(Family::Monospace),
            };
            buffer.set_text(
                &mut self.font_system,
                &text.primitive.text,
                &attrs,
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            text_buffers.push(buffer);
        }

        let text_areas = flattened
            .texts
            .iter()
            .zip(text_buffers.iter())
            .map(|(text, buffer)| TextArea {
                buffer,
                left: text.primitive.rect.x,
                top: text.primitive.rect.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: text.clip.x.round() as i32,
                    top: text.clip.y.round() as i32,
                    right: text.clip.right().round() as i32,
                    bottom: text.clip.bottom().round() as i32,
                },
                default_color: GlyphonColor::rgba(
                    text.primitive.color.r,
                    text.primitive.color.g,
                    text.primitive.color.b,
                    text.primitive.color.a,
                ),
                custom_glyphs: &[],
            })
            .collect::<Vec<_>>();

        self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            text_areas,
            &mut self.swash_cache,
        )?;

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("diffy_frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            if !draw_commands.is_empty() {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                pass.set_vertex_buffer(0, rect_buffer.slice(..));
                for command in &draw_commands {
                    if command.clip.width <= 0.0 || command.clip.height <= 0.0 {
                        continue;
                    }
                    pass.set_scissor_rect(
                        command.clip.x.max(0.0).round() as u32,
                        command.clip.y.max(0.0).round() as u32,
                        command.clip.width.max(1.0).round() as u32,
                        command.clip.height.max(1.0).round() as u32,
                    );
                    pass.draw(command.vertex_range.clone(), 0..1);
                }
            }

            // Reset the scissor after rectangle emission so glyphon text is not
            // accidentally clipped to the final border or shadow slice.
            pass.set_scissor_rect(0, 0, self.surface_config.width, self.surface_config.height);
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)?;
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        self.atlas.trim();

        Ok(FrameStats {
            primitive_count: scene.len(),
            viewport_width: self.surface_config.width,
            viewport_height: self.surface_config.height,
        })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RectVertex {
    position: [f32; 2],
    color: [f32; 4],
}

impl RectVertex {
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ViewportUniform {
    resolution: [f32; 2],
    _padding: [f32; 2],
}

impl ViewportUniform {
    fn new(width: u32, height: u32) -> Self {
        Self {
            resolution: [width as f32, height as f32],
            _padding: [0.0; 2],
        }
    }
}

#[derive(Debug, Clone)]
struct FlattenedScene {
    rects: Vec<ClippedRect>,
    texts: Vec<ClippedText>,
}

#[derive(Debug, Clone)]
struct ClippedRect {
    rect: Rect,
    color: Color,
    clip: Rect,
}

#[derive(Debug, Clone)]
struct ClippedText {
    primitive: TextPrimitive,
    clip: Rect,
}

#[derive(Debug, Clone)]
struct RectDrawCommand {
    vertex_range: std::ops::Range<u32>,
    clip: Rect,
}

fn flatten_scene(scene: &Scene, viewport: Rect) -> FlattenedScene {
    let mut clips = vec![viewport];
    let mut flattened = FlattenedScene {
        rects: Vec::new(),
        texts: Vec::new(),
    };

    for primitive in &scene.primitives {
        match primitive {
            Primitive::Rect(rect) => push_rect(rect.rect, rect.color, &clips, &mut flattened.rects),
            Primitive::RoundedRect(rect) => push_rounded_rect(rect, &clips, &mut flattened.rects),
            Primitive::Border(border) => push_border(border, &clips, &mut flattened.rects),
            Primitive::Shadow(shadow) => push_shadow(shadow, &clips, &mut flattened.rects),
            Primitive::TextRun(text) => {
                if let Some(clip) = clips.last().copied()
                    && let Some(intersection) = text.rect.intersection(clip)
                {
                    flattened.texts.push(ClippedText {
                        primitive: text.clone(),
                        clip: intersection,
                    });
                }
            }
            Primitive::Icon(_) => {}
            Primitive::ClipStart(ClipPrimitive { rect }) => {
                let next = clips
                    .last()
                    .and_then(|clip| clip.intersection(*rect))
                    .unwrap_or_default();
                clips.push(next);
            }
            Primitive::ClipEnd => {
                if clips.len() > 1 {
                    clips.pop();
                }
            }
            Primitive::LayerBoundary => {}
        }
    }

    flattened
}

fn push_rect(rect: Rect, color: Color, clips: &[Rect], out: &mut Vec<ClippedRect>) {
    if let Some(clip) = clips.last().copied()
        && let Some(intersection) = rect.intersection(clip)
    {
        out.push(ClippedRect {
            rect: intersection,
            color,
            clip: intersection,
        });
    }
}

fn push_rounded_rect(rect: &RoundedRectPrimitive, clips: &[Rect], out: &mut Vec<ClippedRect>) {
    push_rect(rect.rect, rect.color, clips, out);
}

fn push_border(border: &BorderPrimitive, clips: &[Rect], out: &mut Vec<ClippedRect>) {
    let top = Rect {
        x: border.rect.x,
        y: border.rect.y,
        width: border.rect.width,
        height: border.width,
    };
    let bottom = Rect {
        x: border.rect.x,
        y: border.rect.bottom() - border.width,
        width: border.rect.width,
        height: border.width,
    };
    let left = Rect {
        x: border.rect.x,
        y: border.rect.y,
        width: border.width,
        height: border.rect.height,
    };
    let right = Rect {
        x: border.rect.right() - border.width,
        y: border.rect.y,
        width: border.width,
        height: border.rect.height,
    };
    for rect in [top, bottom, left, right] {
        push_rect(rect, border.color, clips, out);
    }
}

fn push_shadow(shadow: &ShadowPrimitive, clips: &[Rect], out: &mut Vec<ClippedRect>) {
    let expansion = shadow.blur_radius.max(1.0);
    let rect = Rect {
        x: shadow.rect.x - expansion,
        y: shadow.rect.y - expansion,
        width: shadow.rect.width + expansion * 2.0,
        height: shadow.rect.height + expansion * 2.0,
    };
    push_rect(rect, shadow.color, clips, out);
}

fn build_rect_vertices(rects: &[ClippedRect]) -> (Vec<RectVertex>, Vec<RectDrawCommand>) {
    let mut vertices = Vec::with_capacity(rects.len() * 6);
    let mut commands = Vec::with_capacity(rects.len());
    for rect in rects {
        let start = vertices.len() as u32;
        let color = color_to_linear(rect.color);
        let x0 = rect.rect.x;
        let y0 = rect.rect.y;
        let x1 = rect.rect.right();
        let y1 = rect.rect.bottom();
        vertices.extend_from_slice(&[
            RectVertex {
                position: [x0, y0],
                color,
            },
            RectVertex {
                position: [x1, y0],
                color,
            },
            RectVertex {
                position: [x1, y1],
                color,
            },
            RectVertex {
                position: [x0, y0],
                color,
            },
            RectVertex {
                position: [x1, y1],
                color,
            },
            RectVertex {
                position: [x0, y1],
                color,
            },
        ]);
        commands.push(RectDrawCommand {
            vertex_range: start..start + 6,
            clip: rect.clip,
        });
    }
    (vertices, commands)
}

fn color_to_linear(color: Color) -> [f32; 4] {
    [
        srgb_to_linear(color.r),
        srgb_to_linear(color.g),
        srgb_to_linear(color.b),
        color.a as f32 / 255.0,
    ]
}

fn srgb_to_linear(channel: u8) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

const RECT_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let ndc_x = (input.position.x / viewport.resolution.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (input.position.y / viewport.resolution.y) * 2.0;
    var out: VertexOutput;
    out.position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#;
