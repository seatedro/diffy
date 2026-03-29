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
    ClipPrimitive, FontKind, Primitive, Rect, RichTextPrimitive, Scene, TextPrimitive,
};
use crate::ui::theme::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    pub ui_font_size_px: f32,
    pub ui_line_height_px: f32,
    pub mono_font_size_px: f32,
    pub mono_line_height_px: f32,
    pub mono_char_width_px: f32,
}

impl Default for TextMetrics {
    fn default() -> Self {
        Self {
            ui_font_size_px: 14.0,
            ui_line_height_px: 18.0,
            mono_font_size_px: 13.0,
            mono_line_height_px: 20.0,
            mono_char_width_px: 8.0,
        }
    }
}

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
    #[error("device request failed: {0}")]
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
    quad_pipeline: wgpu::RenderPipeline,
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
            label: Some("diffy_quad_shader"),
            source: wgpu::ShaderSource::Wgsl(QUAD_SHADER.into()),
        });
        let quad_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("diffy_quad_pipeline_layout"),
                bind_group_layouts: &[&viewport_bind_group_layout],
                immediate_size: 0,
            });
        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("diffy_quad_pipeline"),
            layout: Some(&quad_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_quad"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[QuadInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_quad"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                ..wgpu::PrimitiveState::default()
            },
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
            quad_pipeline,
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

    pub fn text_metrics(&self) -> TextMetrics {
        let scale = self.scale_factor as f32;
        TextMetrics {
            ui_font_size_px: 14.0 * scale,
            ui_line_height_px: 18.0 * scale,
            mono_font_size_px: 13.0 * scale,
            mono_line_height_px: 20.0 * scale,
            mono_char_width_px: 8.0 * scale,
        }
    }

    pub fn render(&mut self, scene: &Scene) -> Result<FrameStats, RenderError> {
        if self.surface_config.width == 0 || self.surface_config.height == 0 {
            return Ok(FrameStats::default());
        }

        let viewport_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: self.surface_config.width as f32,
            height: self.surface_config.height as f32,
        };

        let flattened = flatten_scene(scene, viewport_rect);

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

        let (quad_instances, draw_commands) = build_quad_instances(&flattened.quads);
        let quad_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("diffy_quad_instances"),
                contents: bytemuck::cast_slice(&quad_instances),
                usage: wgpu::BufferUsages::VERTEX,
            });

        let mut prepared_texts = Vec::with_capacity(
            flattened
                .texts
                .len()
                .saturating_add(flattened.rich_texts.len()),
        );
        for text in &flattened.texts {
            prepared_texts.push(prepare_plain_text(
                &mut self.font_system,
                &text.primitive,
                text.clip,
                self.scale_factor,
            ));
        }
        for text in &flattened.rich_texts {
            prepared_texts.push(prepare_rich_text(
                &mut self.font_system,
                &text.primitive,
                text.clip,
                self.scale_factor,
            ));
        }

        let text_areas = prepared_texts
            .iter()
            .map(|prepared| TextArea {
                buffer: &prepared.buffer,
                left: prepared.left,
                top: prepared.top,
                scale: 1.0,
                bounds: TextBounds {
                    left: prepared.clip.x.round() as i32,
                    top: prepared.clip.y.round() as i32,
                    right: prepared.clip.right().round() as i32,
                    bottom: prepared.clip.bottom().round() as i32,
                },
                default_color: prepared.default_color,
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
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                pass.set_vertex_buffer(0, quad_buffer.slice(..));
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
                    pass.draw(0..4, command.instance_range.clone());
                }
            }

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

// ---------------------------------------------------------------------------
// GPU types
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct QuadInstance {
    bounds: [f32; 4],
    background: [f32; 4],
    border_color: [f32; 4],
    corner_radii: [f32; 4],
    border_widths: [f32; 4],
}

impl QuadInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 4,
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

// ---------------------------------------------------------------------------
// Scene flattening
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct FlattenedScene {
    quads: Vec<ClippedQuad>,
    texts: Vec<ClippedText>,
    rich_texts: Vec<ClippedRichText>,
}

#[derive(Debug, Clone, Copy)]
struct ClippedQuad {
    instance: QuadInstance,
    clip: Rect,
}

#[derive(Debug, Clone)]
struct ClippedText {
    primitive: TextPrimitive,
    clip: Rect,
}

#[derive(Debug, Clone)]
struct ClippedRichText {
    primitive: RichTextPrimitive,
    clip: Rect,
}

struct QuadDrawCommand {
    instance_range: std::ops::Range<u32>,
    clip: Rect,
}

#[derive(Debug)]
struct PreparedTextBuffer {
    buffer: Buffer,
    left: f32,
    top: f32,
    clip: Rect,
    default_color: GlyphonColor,
}

fn flatten_scene(scene: &Scene, viewport: Rect) -> FlattenedScene {
    let mut clips = vec![viewport];
    let mut flattened = FlattenedScene {
        quads: Vec::new(),
        texts: Vec::new(),
        rich_texts: Vec::new(),
    };

    for primitive in &scene.primitives {
        match primitive {
            Primitive::Rect(rect) => {
                push_quad(
                    rect.rect,
                    color_to_linear(rect.color),
                    [0.0; 4],
                    [0.0; 4],
                    [0.0; 4],
                    &clips,
                    &mut flattened.quads,
                );
            }
            Primitive::RoundedRect(rect) => {
                push_quad(
                    rect.rect,
                    color_to_linear(rect.color),
                    [0.0; 4],
                    [rect.radius; 4],
                    [0.0; 4],
                    &clips,
                    &mut flattened.quads,
                );
            }
            Primitive::Border(border) => {
                let r = border.radius;
                let w = border.width;
                push_quad(
                    border.rect,
                    [0.0; 4],
                    color_to_linear(border.color),
                    [r, r, r, r],
                    [w, w, w, w],
                    &clips,
                    &mut flattened.quads,
                );
            }
            Primitive::Shadow(shadow) => {
                let expansion = shadow.blur_radius.max(1.0);
                let expanded = Rect {
                    x: shadow.rect.x - expansion,
                    y: shadow.rect.y - expansion,
                    width: shadow.rect.width + expansion * 2.0,
                    height: shadow.rect.height + expansion * 2.0,
                };
                push_quad(
                    expanded,
                    color_to_linear(shadow.color),
                    [0.0; 4],
                    [shadow.corner_radius + expansion; 4],
                    [0.0; 4],
                    &clips,
                    &mut flattened.quads,
                );
            }
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
            Primitive::RichTextRun(text) => {
                if let Some(clip) = clips.last().copied()
                    && let Some(intersection) = text.rect.intersection(clip)
                {
                    flattened.rich_texts.push(ClippedRichText {
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

fn push_quad(
    rect: Rect,
    background: [f32; 4],
    border_color: [f32; 4],
    corner_radii: [f32; 4],
    border_widths: [f32; 4],
    clips: &[Rect],
    out: &mut Vec<ClippedQuad>,
) {
    if let Some(clip) = clips.last().copied() {
        if rect.intersection(clip).is_some() {
            out.push(ClippedQuad {
                instance: QuadInstance {
                    bounds: [rect.x, rect.y, rect.width, rect.height],
                    background,
                    border_color,
                    corner_radii,
                    border_widths,
                },
                clip,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Quad instance batching
// ---------------------------------------------------------------------------

fn build_quad_instances(quads: &[ClippedQuad]) -> (Vec<QuadInstance>, Vec<QuadDrawCommand>) {
    let mut instances = Vec::with_capacity(quads.len());
    let mut commands = Vec::with_capacity(quads.len());

    let mut i = 0;
    while i < quads.len() {
        let start = i as u32;
        let clip = quads[i].clip;
        instances.push(quads[i].instance);
        i += 1;

        while i < quads.len() && rects_equal(quads[i].clip, clip) {
            instances.push(quads[i].instance);
            i += 1;
        }

        commands.push(QuadDrawCommand {
            instance_range: start..i as u32,
            clip,
        });
    }

    (instances, commands)
}

fn rects_equal(a: Rect, b: Rect) -> bool {
    a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
}

// ---------------------------------------------------------------------------
// Text preparation (unchanged)
// ---------------------------------------------------------------------------

fn prepare_plain_text(
    font_system: &mut FontSystem,
    primitive: &TextPrimitive,
    clip: Rect,
    scale_factor: f64,
) -> PreparedTextBuffer {
    let metrics = Metrics::new(primitive.font_size, primitive.font_size * 1.35);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some((primitive.rect.width * scale_factor as f32).max(1.0)),
        Some((primitive.rect.height * scale_factor as f32).max(1.0)),
    );
    let attrs = attrs_for_font(primitive.font_kind, primitive.color);
    buffer.set_text(
        font_system,
        &primitive.text,
        &attrs,
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    PreparedTextBuffer {
        buffer,
        left: primitive.rect.x,
        top: primitive.rect.y,
        clip,
        default_color: glyphon_color(primitive.color),
    }
}

fn prepare_rich_text(
    font_system: &mut FontSystem,
    primitive: &RichTextPrimitive,
    clip: Rect,
    scale_factor: f64,
) -> PreparedTextBuffer {
    let metrics = Metrics::new(primitive.font_size, primitive.font_size * 1.35);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some((primitive.rect.width * scale_factor as f32).max(1.0)),
        Some((primitive.rect.height * scale_factor as f32).max(1.0)),
    );
    let default_attrs = attrs_for_font(primitive.font_kind, primitive.default_color);
    let spans = primitive
        .spans
        .iter()
        .map(|span| {
            (
                span.text.as_str(),
                attrs_for_font(primitive.font_kind, span.color),
            )
        })
        .collect::<Vec<_>>();
    if spans.is_empty() {
        buffer.set_text(font_system, "", &default_attrs, Shaping::Advanced, None);
    } else {
        buffer.set_rich_text(
            font_system,
            spans.iter().map(|(text, attrs)| (*text, attrs.clone())),
            &default_attrs,
            Shaping::Advanced,
            None,
        );
    }
    buffer.shape_until_scroll(font_system, false);
    PreparedTextBuffer {
        buffer,
        left: primitive.rect.x,
        top: primitive.rect.y,
        clip,
        default_color: glyphon_color(primitive.default_color),
    }
}

fn attrs_for_font(font_kind: FontKind, color: Color) -> Attrs<'static> {
    let family = match font_kind {
        FontKind::Ui => Family::SansSerif,
        FontKind::Mono => Family::Monospace,
    };
    Attrs::new().family(family).color(glyphon_text_color(color))
}

fn glyphon_color(color: Color) -> GlyphonColor {
    GlyphonColor::rgba(color.r, color.g, color.b, color.a)
}

fn glyphon_text_color(color: Color) -> glyphon::Color {
    glyphon::Color::rgba(color.r, color.g, color.b, color.a)
}

// ---------------------------------------------------------------------------
// Color conversion
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// SDF quad shader
// ---------------------------------------------------------------------------

const QUAD_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) background: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) corner_radii: vec4<f32>,
    @location(4) border_widths: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    @location(1) @interpolate(flat) background: vec4<f32>,
    @location(2) @interpolate(flat) border_color: vec4<f32>,
    @location(3) @interpolate(flat) corner_radii: vec4<f32>,
    @location(4) @interpolate(flat) border_widths: vec4<f32>,
};

@vertex
fn vs_quad(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.bounds = input.bounds;
    out.background = input.background;
    out.border_color = input.border_color;
    out.corner_radii = input.corner_radii;
    out.border_widths = input.border_widths;
    return out;
}

fn pick_corner_radius(p: vec2<f32>, radii: vec4<f32>) -> f32 {
    // radii: tl, tr, br, bl
    if (p.x < 0.0) {
        return select(radii.w, radii.x, p.y < 0.0);
    } else {
        return select(radii.z, radii.y, p.y < 0.0);
    }
}

fn quad_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(p) - half_size + vec2<f32>(radius);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

fn over(below: vec4<f32>, above: vec4<f32>) -> vec4<f32> {
    let a = above.a + below.a * (1.0 - above.a);
    if (a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let c = (above.rgb * above.a + below.rgb * below.a * (1.0 - above.a)) / a;
    return vec4<f32>(c, a);
}

@fragment
fn fs_quad(input: VertexOutput) -> @location(0) vec4<f32> {
    let half_size = input.bounds.zw * 0.5;
    let center = input.bounds.xy + half_size;
    let p = input.position.xy - center;

    let corner_radius = pick_corner_radius(p, input.corner_radii);
    let outer_sdf = quad_sdf(p, half_size, corner_radius);

    let aa = 0.5;
    let outer_alpha = saturate(aa - outer_sdf);
    if (outer_alpha <= 0.0) {
        discard;
    }

    let max_border = max(
        max(input.border_widths.x, input.border_widths.y),
        max(input.border_widths.z, input.border_widths.w)
    );

    var color: vec4<f32>;
    if (max_border > 0.0) {
        let bw = max_border;
        let inner_half = half_size - vec2<f32>(bw);
        let inner_radius = max(0.0, corner_radius - bw);
        let inner_sdf = quad_sdf(p, inner_half, inner_radius);
        let fill_blend = saturate(aa - inner_sdf);
        let blended = over(input.background, input.border_color);
        color = mix(blended, input.background, fill_blend);
    } else {
        color = input.background;
    }

    let final_alpha = color.a * outer_alpha;
    return vec4<f32>(color.rgb * final_alpha, final_alpha);
}
"#;
