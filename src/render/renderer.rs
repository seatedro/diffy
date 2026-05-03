use std::{collections::HashMap, sync::Arc};

use bytemuck::{Pod, Zeroable};
use glyphon::{
    Buffer, Cache, Color as GlyphonColor, FontSystem, Resolution, SwashCache, TextAtlas,
    TextRenderer, Viewport,
};
use thiserror::Error;
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

use crate::render::scene::{
    ClipPrimitive, EditorTextSlot, Primitive, Rect, RichTextPrimitive, Scene, TextPrimitive,
};

use super::shaders::{BLIT_SHADER, BLUR_SHADER, EFFECT_SHADER, QUAD_SHADER, SHADOW_SHADER};
use super::text::{color_to_linear, measure_mono_char_width, prepare_text_areas};

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
            mono_line_height_px: 24.0,
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

// ---------------------------------------------------------------------------
// TexturePool — reusable offscreen render targets
// ---------------------------------------------------------------------------

struct PooledTexture {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    in_use: bool,
    last_used_frame: u64,
}

struct TexturePool {
    textures: Vec<PooledTexture>,
    format: wgpu::TextureFormat,
    frame: u64,
}

const KEEP_UNUSED_OFFSCREEN_FRAMES: u64 = 2;

#[derive(Debug)]
struct ReusableBuffer {
    buffer: wgpu::Buffer,
    capacity_bytes: usize,
}

#[derive(Debug, Default)]
struct TransientBufferPool {
    buffers: Vec<ReusableBuffer>,
    next_buffer: usize,
}

impl TransientBufferPool {
    fn begin_frame(&mut self) {
        self.next_buffer = 0;
    }

    fn upload<T: Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &'static str,
        data: &[T],
    ) -> Option<wgpu::Buffer> {
        if data.is_empty() {
            return None;
        }

        let bytes = bytemuck::cast_slice(data);
        let required_bytes = bytes.len().max(std::mem::size_of::<T>());
        let capacity_bytes = required_bytes.next_power_of_two().max(256);
        let buffer = if let Some(entry) = self.buffers.get_mut(self.next_buffer) {
            if entry.capacity_bytes < required_bytes {
                entry.buffer = create_transient_buffer(device, label, capacity_bytes as u64);
                entry.capacity_bytes = capacity_bytes;
            }
            entry.buffer.clone()
        } else {
            let buffer = create_transient_buffer(device, label, capacity_bytes as u64);
            self.buffers.push(ReusableBuffer {
                buffer: buffer.clone(),
                capacity_bytes,
            });
            buffer
        };
        self.next_buffer += 1;
        queue.write_buffer(&buffer, 0, bytes);
        Some(buffer)
    }
}

fn create_transient_buffer(device: &wgpu::Device, label: &'static str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Handle to an offscreen render target allocated from the pool.
pub struct OffscreenTarget {
    pool_index: usize,
    pub width: u32,
    pub height: u32,
}

impl TexturePool {
    fn new(format: wgpu::TextureFormat) -> Self {
        Self {
            textures: Vec::new(),
            format,
            frame: 0,
        }
    }

    fn begin_frame(&mut self) {
        self.frame = self.frame.saturating_add(1);
    }

    /// Acquire a texture of at least the given dimensions.
    /// Returns a pool index. The caller must call `release()` when done.
    fn acquire(&mut self, device: &wgpu::Device, width: u32, height: u32) -> OffscreenTarget {
        let w = width.max(1);
        let h = height.max(1);

        // Look for an existing unused texture that's big enough.
        for (i, entry) in self.textures.iter_mut().enumerate() {
            if !entry.in_use && entry.width >= w && entry.height >= h {
                entry.in_use = true;
                entry.last_used_frame = self.frame;
                return OffscreenTarget {
                    pool_index: i,
                    width: entry.width,
                    height: entry.height,
                };
            }
        }

        // Allocate a new texture.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("diffy_offscreen"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let index = self.textures.len();
        let _ = texture;
        self.textures.push(PooledTexture {
            view,
            width: w,
            height: h,
            in_use: true,
            last_used_frame: self.frame,
        });
        OffscreenTarget {
            pool_index: index,
            width: w,
            height: h,
        }
    }

    fn view(&self, target: &OffscreenTarget) -> &wgpu::TextureView {
        &self.textures[target.pool_index].view
    }

    fn release(&mut self, target: OffscreenTarget) {
        self.textures[target.pool_index].in_use = false;
        self.textures[target.pool_index].last_used_frame = self.frame;
    }

    fn trim_unused(&mut self) {
        if self.textures.iter().any(|entry| entry.in_use) {
            return;
        }
        let frame = self.frame;
        self.textures.retain(|entry| {
            frame.saturating_sub(entry.last_used_frame) <= KEEP_UNUSED_OFFSCREEN_FRAMES
        });
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    scale_factor: f64,
    quad_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    effect_quad_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    texture_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    texture_pool: TexturePool,
    instance_buffer_pool: TransientBufferPool,
    image_cache: HashMap<u64, (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    viewport_buffer: wgpu::Buffer,
    viewport_bind_group: wgpu::BindGroup,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text_cache: HashMap<u64, CachedTextBuffer>,
    text_cache_frame: u64,
    cached_mono_char_width: Option<(f32, f32)>,
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
        let quad_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diffy_shadow_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADOW_SHADER.into()),
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("diffy_shadow_pipeline_layout"),
                bind_group_layouts: &[&viewport_bind_group_layout],
                immediate_size: 0,
            });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("diffy_shadow_pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_shadow"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[ShadowInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shadow_shader,
                entry_point: Some("fs_shadow"),
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

        let effect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diffy_effect_shader"),
            source: wgpu::ShaderSource::Wgsl(EFFECT_SHADER.into()),
        });
        let effect_quad_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("diffy_effect_quad_pipeline_layout"),
                bind_group_layouts: &[&viewport_bind_group_layout],
                immediate_size: 0,
            });
        let effect_quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("diffy_effect_quad_pipeline"),
            layout: Some(&effect_quad_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &effect_shader,
                entry_point: Some("vs_effect"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[EffectQuadInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &effect_shader,
                entry_point: Some("fs_effect"),
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

        // --- Blit pipeline (composites offscreen textures back to screen) ---

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("diffy_texture_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("diffy_blit_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diffy_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("diffy_blit_pipeline_layout"),
            bind_group_layouts: &[&viewport_bind_group_layout, &texture_bind_group_layout],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("diffy_blit_pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_blit"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[BlitInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_blit"),
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

        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diffy_blur_shader"),
            source: wgpu::ShaderSource::Wgsl(BLUR_SHADER.into()),
        });
        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("diffy_blur_pipeline_layout"),
            bind_group_layouts: &[&viewport_bind_group_layout, &texture_bind_group_layout],
            immediate_size: 0,
        });
        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("diffy_blur_pipeline"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blur_shader,
                entry_point: Some("vs_blur"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[BlurInstance::layout()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blur_shader,
                entry_point: Some("fs_blur"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None, // blur fully overwrites
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

        let texture_pool = TexturePool::new(surface_format);

        let font_system = crate::fonts::new_font_system();
        let swash_cache = SwashCache::new();
        let glyph_cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &glyph_cache);
        let mut atlas = TextAtlas::new(&device, &queue, &glyph_cache, surface_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);

        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
            size,
            scale_factor,
            quad_pipeline,
            shadow_pipeline,
            effect_quad_pipeline,
            blit_pipeline,
            blur_pipeline,
            texture_bind_group_layout,
            sampler,
            texture_pool,
            instance_buffer_pool: TransientBufferPool::default(),
            image_cache: HashMap::new(),
            viewport_buffer,
            viewport_bind_group,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text_cache: HashMap::new(),
            text_cache_frame: 0,
            cached_mono_char_width: None,
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

    pub fn font_system(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    pub fn font_system_mut(&mut self) -> &mut FontSystem {
        &mut self.font_system
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    pub fn text_metrics(&mut self) -> TextMetrics {
        let scale = self.scale_factor as f32;
        let mono_font_size = 13.0 * scale;
        let char_w = match self.cached_mono_char_width {
            Some((cached_size, cached_w)) if (cached_size - mono_font_size).abs() < 0.001 => {
                cached_w
            }
            _ => {
                let w = measure_mono_char_width(&mut self.font_system, mono_font_size);
                self.cached_mono_char_width = Some((mono_font_size, w));
                w
            }
        };
        TextMetrics {
            ui_font_size_px: 14.0 * scale,
            ui_line_height_px: 18.0 * scale,
            mono_font_size_px: mono_font_size,
            mono_line_height_px: 24.0 * scale,
            mono_char_width_px: char_w,
        }
    }

    // -- Offscreen render target management --

    /// Acquire an offscreen texture from the pool. The returned target can be
    /// used as a render attachment and later sampled via `create_texture_bind_group`.
    pub fn acquire_offscreen(&mut self, width: u32, height: u32) -> OffscreenTarget {
        self.texture_pool.acquire(&self.device, width, height)
    }

    /// Get the texture view for an offscreen target (for use as a render attachment).
    pub fn offscreen_view(&self, target: &OffscreenTarget) -> &wgpu::TextureView {
        self.texture_pool.view(target)
    }

    /// Create a bind group for sampling an offscreen target in a shader.
    pub fn create_texture_bind_group(&self, target: &OffscreenTarget) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("diffy_offscreen_bind_group"),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(self.texture_pool.view(target)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Return an offscreen target to the pool for reuse.
    pub fn release_offscreen(&mut self, target: OffscreenTarget) {
        self.texture_pool.release(target);
    }

    pub fn render(
        &mut self,
        scene: &Scene,
        time_seconds: f32,
        editors: &[Option<&crate::editor::Editor>],
    ) -> Result<FrameStats, RenderError> {
        if self.surface_config.width == 0 || self.surface_config.height == 0 {
            return Ok(FrameStats::default());
        }
        self.texture_pool.begin_frame();

        let viewport_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: self.surface_config.width as f32,
            height: self.surface_config.height as f32,
        };

        // Update time in the viewport uniform buffer.
        let viewport_uniform = ViewportUniform {
            resolution: [
                self.surface_config.width as f32,
                self.surface_config.height as f32,
            ],
            time: time_seconds,
            _padding: 0.0,
        };
        self.queue.write_buffer(
            &self.viewport_buffer,
            0,
            bytemuck::bytes_of(&viewport_uniform),
        );

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

        self.instance_buffer_pool.begin_frame();

        // Build GPU buffers for each z-layer's draw layers.
        struct ZLayerBuffers {
            layer_buffers: Vec<LayerBuffers>,
        }

        let device = &self.device;
        let queue = &self.queue;
        let buffer_pool = &mut self.instance_buffer_pool;
        let z_layer_buffers: Vec<ZLayerBuffers> = flattened
            .z_layers
            .iter()
            .map(|zl| {
                let layer_buffers = zl
                    .draw_layers
                    .iter()
                    .map(|layer| {
                        let (si, sc) = build_shadow_instances(&layer.shadows);
                        let sb = buffer_pool.upload(device, queue, "diffy_shadow_instances", &si);
                        let (qi, qc) = build_quad_instances(&layer.quads);
                        let qb = buffer_pool.upload(device, queue, "diffy_quad_instances", &qi);
                        let (ei, ec) = build_effect_quad_instances(&layer.effect_quads);
                        let eb =
                            buffer_pool.upload(device, queue, "diffy_effect_quad_instances", &ei);
                        LayerBuffers {
                            shadow_buffer: sb,
                            shadow_commands: sc,
                            quad_buffer: qb,
                            quad_commands: qc,
                            effect_buffer: eb,
                            effect_commands: ec,
                        }
                    })
                    .collect();
                ZLayerBuffers { layer_buffers }
            })
            .collect();

        let single_z = flattened.z_layers.len() <= 1;

        for zl in &flattened.z_layers {
            for img in &zl.images {
                let key = img.primitive.cache_key;
                if key != 0 && !self.image_cache.contains_key(&key) {
                    if !img.primitive.rgba.is_empty()
                        && img.primitive.width > 0
                        && img.primitive.height > 0
                    {
                        let texture = self.device.create_texture_with_data(
                            &self.queue,
                            &wgpu::TextureDescriptor {
                                label: Some("diffy_cached_image"),
                                size: wgpu::Extent3d {
                                    width: img.primitive.width,
                                    height: img.primitive.height,
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                                view_formats: &[],
                            },
                            wgpu::util::TextureDataOrder::LayerMajor,
                            &img.primitive.rgba,
                        );
                        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let bind_group =
                            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("diffy_cached_image_bind"),
                                layout: &self.texture_bind_group_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::TextureView(&view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                                    },
                                ],
                            });
                        self.image_cache.insert(key, (texture, view, bind_group));
                    }
                }
            }
        }

        if single_z && flattened.blur_regions.is_empty() {
            // ---- Fast path: single z-layer, no blur ----
            let zl = &flattened.z_layers[0];
            let zlb = &z_layer_buffers[0];

            let mut text_areas = prepare_text_areas(
                &mut self.font_system,
                &mut self.text_cache,
                &mut self.text_cache_frame,
                &zl.texts,
                &zl.rich_texts,
                self.scale_factor,
            );
            append_editor_text_areas(&mut text_areas, &zl.editor_slots, editors);

            self.text_renderer.prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )?;

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

            draw_layers(
                &mut pass,
                &zlb.layer_buffers,
                &self.shadow_pipeline,
                &self.effect_quad_pipeline,
                &self.quad_pipeline,
                &self.viewport_bind_group,
                self.surface_config.width,
                self.surface_config.height,
            );

            draw_images(
                &mut pass,
                &zl.images,
                &mut self.instance_buffer_pool,
                &self.device,
                &self.queue,
                &self.blit_pipeline,
                &self.viewport_bind_group,
                &self.image_cache,
                self.surface_config.width,
                self.surface_config.height,
            );

            pass.set_scissor_rect(0, 0, self.surface_config.width, self.surface_config.height);
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)?;
        } else if flattened.blur_regions.is_empty() {
            // ---- Multi z-layer path: separate text render per z-layer ----
            // Each z-layer gets its own encoder+submit so that
            // text_renderer.prepare() for layer N cannot destroy the vertex
            // buffer that layer N-1's render pass still references.
            let sw = self.surface_config.width;
            let sh = self.surface_config.height;
            let mut first = true;

            for (zl, zlb) in flattened.z_layers.iter().zip(z_layer_buffers.iter()) {
                let mut text_areas = prepare_text_areas(
                    &mut self.font_system,
                    &mut self.text_cache,
                    &mut self.text_cache_frame,
                    &zl.texts,
                    &zl.rich_texts,
                    self.scale_factor,
                );
                append_editor_text_areas(&mut text_areas, &zl.editor_slots, editors);

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
                    let load = if first {
                        first = false;
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    };

                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("diffy_z_layer_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });

                    draw_layers(
                        &mut pass,
                        &zlb.layer_buffers,
                        &self.shadow_pipeline,
                        &self.effect_quad_pipeline,
                        &self.quad_pipeline,
                        &self.viewport_bind_group,
                        sw,
                        sh,
                    );

                    draw_images(
                        &mut pass,
                        &zl.images,
                        &mut self.instance_buffer_pool,
                        &self.device,
                        &self.queue,
                        &self.blit_pipeline,
                        &self.viewport_bind_group,
                        &self.image_cache,
                        sw,
                        sh,
                    );

                    pass.set_scissor_rect(0, 0, sw, sh);
                    self.text_renderer
                        .render(&self.atlas, &self.viewport, &mut pass)?;
                }

                self.queue.submit(Some(encoder.finish()));
                encoder = self
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("diffy_frame_encoder"),
                    });
            }
        } else {
            // ---- Blur path: render via offscreen intermediates ----
            // Flatten all z-layers into a single draw layer list for the blur.
            let all_layer_bufs: Vec<LayerBuffers> = z_layer_buffers
                .into_iter()
                .flat_map(|z| z.layer_buffers.into_iter())
                .collect();
            let all_texts: Vec<ClippedText> = flattened
                .z_layers
                .iter()
                .flat_map(|z| z.texts.iter().cloned())
                .collect();
            let all_rich: Vec<ClippedRichText> = flattened
                .z_layers
                .iter()
                .flat_map(|z| z.rich_texts.iter().cloned())
                .collect();
            let all_editor_slots: Vec<ClippedEditorSlot> = flattened
                .z_layers
                .iter()
                .flat_map(|z| z.editor_slots.iter().copied())
                .collect();

            let blur = flattened.blur_regions[0];
            let sw = self.surface_config.width;
            let sh = self.surface_config.height;

            let scene_target = self.texture_pool.acquire(&self.device, sw, sh);
            let h_target = self.texture_pool.acquire(&self.device, sw, sh);
            let v_target = self.texture_pool.acquire(&self.device, sw, sh);

            let scene_bind = create_texture_bind_group(
                &self.device,
                &self.texture_bind_group_layout,
                self.texture_pool.view(&scene_target),
                &self.sampler,
            );
            let h_bind = create_texture_bind_group(
                &self.device,
                &self.texture_bind_group_layout,
                self.texture_pool.view(&h_target),
                &self.sampler,
            );
            let v_bind = create_texture_bind_group(
                &self.device,
                &self.texture_bind_group_layout,
                self.texture_pool.view(&v_target),
                &self.sampler,
            );

            let sigma = (blur.blur_radius * 0.5).max(0.5);
            let br = blur.rect;
            let uv_min_x = br.x / sw as f32;
            let uv_min_y = br.y / sh as f32;
            let uv_max_x = br.right() / sw as f32;
            let uv_max_y = br.bottom() / sh as f32;

            // Step 1: Render pre-blur layers → scene_tex.
            {
                let scene_view = self.texture_pool.view(&scene_target);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("diffy_blur_scene_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: scene_view,
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

                let end = blur.layer_break.min(all_layer_bufs.len());
                draw_layers(
                    &mut pass,
                    &all_layer_bufs[..end],
                    &self.shadow_pipeline,
                    &self.effect_quad_pipeline,
                    &self.quad_pipeline,
                    &self.viewport_bind_group,
                    sw,
                    sh,
                );
            }

            // Step 2: Horizontal blur → h_target.
            {
                let h_view = self.texture_pool.view(&h_target);
                let blur_inst = BlurInstance {
                    bounds: [br.x, br.y, br.width, br.height],
                    uv_rect: [uv_min_x, uv_min_y, uv_max_x, uv_max_y],
                    blur_params: [1.0, 0.0, sigma, 0.0],
                };
                let buf = self
                    .instance_buffer_pool
                    .upload(
                        &self.device,
                        &self.queue,
                        "diffy_blur_h_instance",
                        &[blur_inst],
                    )
                    .expect("single blur instance upload");

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("diffy_blur_h_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: h_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                pass.set_pipeline(&self.blur_pipeline);
                pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                pass.set_bind_group(1, &scene_bind, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..4, 0..1);
            }

            // Step 3: Vertical blur → v_target.
            {
                let v_view = self.texture_pool.view(&v_target);
                let blur_inst = BlurInstance {
                    bounds: [br.x, br.y, br.width, br.height],
                    uv_rect: [uv_min_x, uv_min_y, uv_max_x, uv_max_y],
                    blur_params: [0.0, 1.0, sigma, 0.0],
                };
                let buf = self
                    .instance_buffer_pool
                    .upload(
                        &self.device,
                        &self.queue,
                        "diffy_blur_v_instance",
                        &[blur_inst],
                    )
                    .expect("single blur instance upload");

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("diffy_blur_v_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: v_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });

                pass.set_pipeline(&self.blur_pipeline);
                pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                pass.set_bind_group(1, &h_bind, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..4, 0..1);
            }

            // Step 4: Composite to surface.
            {
                let mut text_areas = prepare_text_areas(
                    &mut self.font_system,
                    &mut self.text_cache,
                    &mut self.text_cache_frame,
                    &all_texts,
                    &all_rich,
                    self.scale_factor,
                );
                append_editor_text_areas(&mut text_areas, &all_editor_slots, editors);

                // Full-screen blit of scene_tex.
                let scene_blit = BlitInstance {
                    bounds: [0.0, 0.0, sw as f32, sh as f32],
                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                    tint: [1.0, 1.0, 1.0, 1.0],
                };
                let scene_blit_buf = self
                    .instance_buffer_pool
                    .upload(&self.device, &self.queue, "diffy_scene_blit", &[scene_blit])
                    .expect("single blit upload");
                // Blur region blit.
                let blur_blit = BlitInstance {
                    bounds: [br.x, br.y, br.width, br.height],
                    uv_rect: [uv_min_x, uv_min_y, uv_max_x, uv_max_y],
                    tint: [1.0, 1.0, 1.0, 1.0],
                };
                let blur_blit_buf = self
                    .instance_buffer_pool
                    .upload(&self.device, &self.queue, "diffy_blur_blit", &[blur_blit])
                    .expect("single blit upload");

                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("diffy_composite_pass"),
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

                // Blit scene background.
                pass.set_pipeline(&self.blit_pipeline);
                pass.set_bind_group(0, &self.viewport_bind_group, &[]);
                pass.set_bind_group(1, &scene_bind, &[]);
                pass.set_vertex_buffer(0, scene_blit_buf.slice(..));
                pass.draw(0..4, 0..1);

                // Blit blurred region on top.
                pass.set_bind_group(1, &v_bind, &[]);
                pass.set_vertex_buffer(0, blur_blit_buf.slice(..));
                pass.draw(0..4, 0..1);

                // Render remaining layers (modal content, etc.) on top.
                let start = blur.layer_break.min(all_layer_bufs.len());
                pass.set_scissor_rect(0, 0, sw, sh);
                draw_layers(
                    &mut pass,
                    &all_layer_bufs[start..],
                    &self.shadow_pipeline,
                    &self.effect_quad_pipeline,
                    &self.quad_pipeline,
                    &self.viewport_bind_group,
                    sw,
                    sh,
                );

                self.text_renderer.prepare(
                    &self.device,
                    &self.queue,
                    &mut self.font_system,
                    &mut self.atlas,
                    &self.viewport,
                    text_areas,
                    &mut self.swash_cache,
                )?;
                pass.set_scissor_rect(0, 0, sw, sh);
                self.text_renderer
                    .render(&self.atlas, &self.viewport, &mut pass)?;
            }

            self.texture_pool.release(scene_target);
            self.texture_pool.release(h_target);
            self.texture_pool.release(v_target);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        self.atlas.trim();
        self.texture_pool.trim_unused();

        Ok(FrameStats {
            primitive_count: scene.len(),
            viewport_width: self.surface_config.width,
            viewport_height: self.surface_config.height,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct LayerBuffers {
    shadow_buffer: Option<wgpu::Buffer>,
    shadow_commands: Vec<QuadDrawCommand>,
    quad_buffer: Option<wgpu::Buffer>,
    quad_commands: Vec<QuadDrawCommand>,
    effect_buffer: Option<wgpu::Buffer>,
    effect_commands: Vec<QuadDrawCommand>,
}

fn draw_layers<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    layers: &'pass [LayerBuffers],
    shadow_pipeline: &'pass wgpu::RenderPipeline,
    effect_quad_pipeline: &'pass wgpu::RenderPipeline,
    quad_pipeline: &'pass wgpu::RenderPipeline,
    viewport_bind_group: &'pass wgpu::BindGroup,
    viewport_w: u32,
    viewport_h: u32,
) {
    for lb in layers {
        if let Some(ref shadow_buf) = lb.shadow_buffer {
            pass.set_pipeline(shadow_pipeline);
            pass.set_bind_group(0, viewport_bind_group, &[]);
            pass.set_vertex_buffer(0, shadow_buf.slice(..));
            for command in &lb.shadow_commands {
                let Some((sx, sy, sw, sh)) = scissor_rect(command.clip, viewport_w, viewport_h)
                else {
                    continue;
                };
                pass.set_scissor_rect(sx, sy, sw, sh);
                pass.draw(0..4, command.instance_range.clone());
            }
        }

        if let Some(ref effect_buf) = lb.effect_buffer {
            pass.set_pipeline(effect_quad_pipeline);
            pass.set_bind_group(0, viewport_bind_group, &[]);
            pass.set_vertex_buffer(0, effect_buf.slice(..));
            for command in &lb.effect_commands {
                let Some((sx, sy, sw, sh)) = scissor_rect(command.clip, viewport_w, viewport_h)
                else {
                    continue;
                };
                pass.set_scissor_rect(sx, sy, sw, sh);
                pass.draw(0..4, command.instance_range.clone());
            }
        }

        if let Some(ref quad_buf) = lb.quad_buffer {
            pass.set_pipeline(quad_pipeline);
            pass.set_bind_group(0, viewport_bind_group, &[]);
            pass.set_vertex_buffer(0, quad_buf.slice(..));
            for command in &lb.quad_commands {
                let Some((sx, sy, sw, sh)) = scissor_rect(command.clip, viewport_w, viewport_h)
                else {
                    continue;
                };
                pass.set_scissor_rect(sx, sy, sw, sh);
                pass.draw(0..4, command.instance_range.clone());
            }
        }
    }
}

fn draw_images<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    images: &[ClippedImage],
    buffer_pool: &mut TransientBufferPool,
    device: &'pass wgpu::Device,
    queue: &wgpu::Queue,
    blit_pipeline: &'pass wgpu::RenderPipeline,
    viewport_bind_group: &'pass wgpu::BindGroup,
    image_cache: &'pass HashMap<u64, (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup)>,
    viewport_w: u32,
    viewport_h: u32,
) {
    for img in images {
        if img.primitive.rgba.is_empty() || img.primitive.width == 0 || img.primitive.height == 0 {
            continue;
        }

        let bind_group = match image_cache.get(&img.primitive.cache_key) {
            Some((_, _, bg)) => bg,
            None => continue,
        };

        let r = img.primitive.rect;
        let blit_inst = BlitInstance {
            bounds: [r.x, r.y, r.width, r.height],
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
        };
        let Some(buf) = buffer_pool.upload(device, queue, "diffy_image_blit", &[blit_inst]) else {
            continue;
        };

        let Some((sx, sy, sw, sh)) = scissor_rect(img.clip, viewport_w, viewport_h) else {
            continue;
        };

        pass.set_pipeline(blit_pipeline);
        pass.set_bind_group(0, viewport_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.set_scissor_rect(sx, sy, sw, sh);
        pass.draw(0..4, 0..1);
    }
}

fn scissor_rect(clip: Rect, viewport_w: u32, viewport_h: u32) -> Option<(u32, u32, u32, u32)> {
    if viewport_w == 0
        || viewport_h == 0
        || clip.width <= 0.0
        || clip.height <= 0.0
        || !clip.x.is_finite()
        || !clip.y.is_finite()
        || !clip.width.is_finite()
        || !clip.height.is_finite()
    {
        return None;
    }

    let viewport_w = viewport_w as f32;
    let viewport_h = viewport_h as f32;
    let left = clip.x.max(0.0).floor().min(viewport_w);
    let top = clip.y.max(0.0).floor().min(viewport_h);
    let right = (clip.x + clip.width).ceil().clamp(0.0, viewport_w);
    let bottom = (clip.y + clip.height).ceil().clamp(0.0, viewport_h);

    if right <= left || bottom <= top {
        return None;
    }

    let sx = left as u32;
    let sy = top as u32;
    let sw = (right as u32).saturating_sub(sx);
    let sh = (bottom as u32).saturating_sub(sy);

    (sw > 0 && sh > 0).then_some((sx, sy, sw, sh))
}

fn create_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("diffy_texture_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
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
    /// Rounded-clip rect [x, y, w, h] — the rect whose radii apply to the SDF clip test.
    clip_bounds: [f32; 4],
    /// Rounded-clip corner radii [tl, tr, br, bl]. All zero = no rounded clip.
    clip_radii: [f32; 4],
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
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 96,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ShadowInstance {
    /// Expanded quad bounds (x, y, w, h) — covers the full blur extent.
    draw_bounds: [f32; 4],
    /// Original shadow-casting rect (x, y, w, h) before expansion.
    shadow_bounds: [f32; 4],
    /// Shadow color (linear RGBA, premultiplied).
    color: [f32; 4],
    /// [blur_sigma, corner_radius, 0, 0]
    params: [f32; 4],
    /// Rounded-clip rect [x, y, w, h] — the rect whose radii apply for the SDF clip test.
    clip_bounds: [f32; 4],
    /// Rounded-clip corner radii [tl, tr, br, bl]. All zero = no rounded clip.
    clip_radii: [f32; 4],
}

impl ShadowInstance {
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
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct EffectQuadInstance {
    /// Element bounds: [x, y, width, height].
    bounds: [f32; 4],
    /// First color (linear RGBA, premultiplied).
    color_a: [f32; 4],
    /// Second color (linear RGBA, premultiplied).
    color_b: [f32; 4],
    /// [effect_type, param1, param2, corner_radius].
    params: [f32; 4],
    /// Rounded-clip rect [x, y, w, h] — the rect whose radii apply for the SDF clip test.
    clip_bounds: [f32; 4],
    /// Rounded-clip corner radii [tl, tr, br, bl]. All zero = no rounded clip.
    clip_radii: [f32; 4],
}

impl EffectQuadInstance {
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
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct BlitInstance {
    /// Screen-space destination bounds: [x, y, width, height].
    bounds: [f32; 4],
    /// Source texture UV rect: [u_min, v_min, u_max, v_max].
    uv_rect: [f32; 4],
    /// Tint/opacity multiplier (usually [1, 1, 1, alpha]).
    tint: [f32; 4],
}

impl BlitInstance {
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
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct BlurInstance {
    /// Destination bounds in the target texture: [x, y, width, height].
    bounds: [f32; 4],
    /// Source UV rect: [u_min, v_min, u_max, v_max].
    uv_rect: [f32; 4],
    /// [direction_x, direction_y, blur_sigma, 0.0]
    /// direction = (1,0) for horizontal, (0,1) for vertical.
    blur_params: [f32; 4],
}

impl BlurInstance {
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
            ],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ViewportUniform {
    resolution: [f32; 2],
    time: f32,
    _padding: f32,
}

impl ViewportUniform {
    fn new(width: u32, height: u32) -> Self {
        Self {
            resolution: [width as f32, height as f32],
            time: 0.0,
            _padding: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Scene flattening
// ---------------------------------------------------------------------------

/// A draw layer groups shadows that must render before their corresponding
/// quads. Layers are rendered in order: for each layer we draw all its shadows,
/// then all its quads. A new layer starts when a shadow primitive appears after
/// quads have already been added to the current layer, ensuring correct
/// depth ordering for elevated surfaces like modals.
#[derive(Debug, Clone, Default)]
struct DrawLayer {
    shadows: Vec<ClippedShadow>,
    quads: Vec<ClippedQuad>,
    effect_quads: Vec<ClippedEffectQuad>,
}

#[derive(Debug, Clone, Copy)]
struct FlattenedBlurRegion {
    /// Screen-space bounds of the blur region.
    rect: Rect,
    blur_radius: f32,
    /// Index into the layers vec: this blur applies after all layers
    /// up to (but not including) this index have been rendered.
    layer_break: usize,
}

/// A z-layer groups all draw layers and text for one z-index value.
/// Rendered in z-index order: lower z-indices first, higher on top.
#[derive(Debug, Clone, Default)]
struct ZLayer {
    draw_layers: Vec<DrawLayer>,
    texts: Vec<ClippedText>,
    rich_texts: Vec<ClippedRichText>,
    images: Vec<ClippedImage>,
    editor_slots: Vec<ClippedEditorSlot>,
}

#[derive(Debug, Clone, Copy)]
struct ClippedEditorSlot {
    slot: EditorTextSlot,
    clip: Rect,
}

#[derive(Debug, Clone)]
struct FlattenedScene {
    z_layers: Vec<ZLayer>,
    blur_regions: Vec<FlattenedBlurRegion>,
}

#[derive(Debug, Clone, Copy)]
struct ClippedShadow {
    instance: ShadowInstance,
    clip: Rect,
}

#[derive(Debug, Clone, Copy)]
struct ClippedQuad {
    instance: QuadInstance,
    clip: Rect,
}

#[derive(Debug, Clone, Copy)]
struct ClippedEffectQuad {
    instance: EffectQuadInstance,
    clip: Rect,
}

#[derive(Debug, Clone)]
struct ClippedImage {
    primitive: crate::render::scene::ImagePrimitive,
    clip: Rect,
}

#[derive(Debug, Clone)]
pub(super) struct ClippedText {
    pub(super) primitive: TextPrimitive,
    pub(super) clip: Rect,
}

#[derive(Debug, Clone)]
pub(super) struct ClippedRichText {
    pub(super) primitive: RichTextPrimitive,
    pub(super) clip: Rect,
}

struct QuadDrawCommand {
    instance_range: std::ops::Range<u32>,
    clip: Rect,
}

#[derive(Debug)]
pub(super) struct CachedTextBuffer {
    pub(super) buffer: Buffer,
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) clip: Rect,
    pub(super) default_color: GlyphonColor,
    pub(super) last_used_frame: u64,
}

/// Active clip state carried on the flatten_scene stack.
///
/// `scissor` is the intersection of every rect clip active at this point — it
/// maps directly to `set_scissor_rect` for rectangular culling. The rounded
/// clip is tracked separately: `rounded_rect` and `corner_radii` describe the
/// innermost rounded clip ancestor (if any). When a non-rounded clip is
/// pushed, we inherit the parent's rounded clip so that descendants of a
/// rounded container are still clipped to its rounded boundary. This drops
/// information for nested rounded clips (inner wins) — good enough in
/// practice since nested rounded clips are rare.
#[derive(Debug, Clone, Copy)]
struct ActiveClip {
    scissor: Rect,
    rounded_rect: Rect,
    corner_radii: [f32; 4],
}

impl ActiveClip {
    fn root(viewport: Rect) -> Self {
        Self {
            scissor: viewport,
            rounded_rect: viewport,
            corner_radii: [0.0; 4],
        }
    }

    fn has_rounded(&self) -> bool {
        self.corner_radii[0] > 0.0
            || self.corner_radii[1] > 0.0
            || self.corner_radii[2] > 0.0
            || self.corner_radii[3] > 0.0
    }

    fn push(&self, rect: Rect, corner_radii: [f32; 4]) -> Option<Self> {
        let scissor = self.scissor.intersection(rect)?;
        let is_rounded = corner_radii[0] > 0.0
            || corner_radii[1] > 0.0
            || corner_radii[2] > 0.0
            || corner_radii[3] > 0.0;
        let (rounded_rect, radii) = if is_rounded {
            (rect, corner_radii)
        } else {
            (self.rounded_rect, self.corner_radii)
        };
        Some(Self {
            scissor,
            rounded_rect,
            corner_radii: radii,
        })
    }

    fn clip_bounds_attr(&self) -> [f32; 4] {
        if self.has_rounded() {
            [
                self.rounded_rect.x,
                self.rounded_rect.y,
                self.rounded_rect.width,
                self.rounded_rect.height,
            ]
        } else {
            [0.0; 4]
        }
    }

    fn clip_radii_attr(&self) -> [f32; 4] {
        self.corner_radii
    }
}

fn flatten_scene(scene: &Scene, viewport: Rect) -> FlattenedScene {
    use std::collections::BTreeMap;

    let mut clips = vec![ActiveClip::root(viewport)];
    let mut z_index_stack = vec![0i32];
    let mut z_map: BTreeMap<i32, ZLayer> = BTreeMap::new();

    // Ensure z=0 always exists.
    z_map.insert(0, ZLayer::default());

    let mut flattened = FlattenedScene {
        z_layers: Vec::new(),
        blur_regions: Vec::new(),
    };

    // Helper: get (or create) the current z-layer's draw layers.
    macro_rules! current_z {
        () => {{
            let z = *z_index_stack.last().unwrap();
            z_map.entry(z).or_insert_with(|| ZLayer {
                draw_layers: vec![DrawLayer::default()],
                ..Default::default()
            })
        }};
    }

    // Ensure the current z-layer has at least one draw layer.
    macro_rules! ensure_draw_layer {
        ($zl:expr) => {
            if $zl.draw_layers.is_empty() {
                $zl.draw_layers.push(DrawLayer::default());
            }
        };
    }

    for primitive in &scene.primitives {
        match primitive {
            Primitive::Rect(rect) => {
                let zl = current_z!();
                ensure_draw_layer!(zl);
                push_quad(
                    rect.rect,
                    color_to_linear(rect.color),
                    [0.0; 4],
                    [0.0; 4],
                    [0.0; 4],
                    &clips,
                    &mut zl.draw_layers.last_mut().unwrap().quads,
                );
            }
            Primitive::RoundedRect(rect) => {
                let zl = current_z!();
                ensure_draw_layer!(zl);
                push_quad(
                    rect.rect,
                    color_to_linear(rect.color),
                    [0.0; 4],
                    rect.corner_radii,
                    [0.0; 4],
                    &clips,
                    &mut zl.draw_layers.last_mut().unwrap().quads,
                );
            }
            Primitive::Border(border) => {
                let zl = current_z!();
                ensure_draw_layer!(zl);
                push_quad(
                    border.rect,
                    [0.0; 4],
                    color_to_linear(border.color),
                    border.corner_radii,
                    border.widths,
                    &clips,
                    &mut zl.draw_layers.last_mut().unwrap().quads,
                );
            }
            Primitive::Shadow(shadow) => {
                let zl = current_z!();
                ensure_draw_layer!(zl);
                if !zl.draw_layers.last().unwrap().quads.is_empty() {
                    zl.draw_layers.push(DrawLayer::default());
                }

                let sigma = (shadow.blur_radius * 0.5).max(0.5);
                let expansion = sigma * 3.0;
                let offset_x = shadow.offset[0];
                let offset_y = shadow.offset[1];
                let expanded = Rect {
                    x: shadow.rect.x + offset_x - expansion,
                    y: shadow.rect.y + offset_y - expansion,
                    width: shadow.rect.width + expansion * 2.0,
                    height: shadow.rect.height + expansion * 2.0,
                };
                if let Some(clip) = clips.last().copied() {
                    if expanded.intersection(clip.scissor).is_some() {
                        let color = color_to_linear(shadow.color);
                        zl.draw_layers
                            .last_mut()
                            .unwrap()
                            .shadows
                            .push(ClippedShadow {
                                instance: ShadowInstance {
                                    draw_bounds: [
                                        expanded.x,
                                        expanded.y,
                                        expanded.width,
                                        expanded.height,
                                    ],
                                    shadow_bounds: [
                                        shadow.rect.x + offset_x,
                                        shadow.rect.y + offset_y,
                                        shadow.rect.width,
                                        shadow.rect.height,
                                    ],
                                    color,
                                    params: [sigma, shadow.corner_radius, 0.0, 0.0],
                                    clip_bounds: clip.clip_bounds_attr(),
                                    clip_radii: clip.clip_radii_attr(),
                                },
                                clip: clip.scissor,
                            });
                    }
                }
            }
            Primitive::TextRun(text) => {
                if let Some(clip) = clips.last().copied()
                    && let Some(intersection) = text.rect.intersection(clip.scissor)
                {
                    let zl = current_z!();
                    zl.texts.push(ClippedText {
                        primitive: text.clone(),
                        clip: intersection,
                    });
                }
            }
            Primitive::RichTextRun(text) => {
                if let Some(clip) = clips.last().copied()
                    && let Some(intersection) = text.rect.intersection(clip.scissor)
                {
                    let zl = current_z!();
                    zl.rich_texts.push(ClippedRichText {
                        primitive: text.clone(),
                        clip: intersection,
                    });
                }
            }
            Primitive::BlurRegion(blur) => {
                if let Some(clip) = clips.last().copied() {
                    if blur.rect.intersection(clip.scissor).is_some() {
                        let zl = current_z!();
                        ensure_draw_layer!(zl);
                        zl.draw_layers.push(DrawLayer::default());
                        flattened.blur_regions.push(FlattenedBlurRegion {
                            rect: blur.rect,
                            blur_radius: blur.blur_radius,
                            layer_break: zl.draw_layers.len() - 1,
                        });
                    }
                }
            }
            Primitive::EffectQuad(effect) => {
                if let Some(clip) = clips.last().copied() {
                    if effect.rect.intersection(clip.scissor).is_some() {
                        let color_a = color_to_linear(effect.color_a);
                        let color_b = color_to_linear(effect.color_b);
                        let zl = current_z!();
                        ensure_draw_layer!(zl);
                        zl.draw_layers
                            .last_mut()
                            .unwrap()
                            .effect_quads
                            .push(ClippedEffectQuad {
                                instance: EffectQuadInstance {
                                    bounds: [
                                        effect.rect.x,
                                        effect.rect.y,
                                        effect.rect.width,
                                        effect.rect.height,
                                    ],
                                    color_a,
                                    color_b,
                                    params: [
                                        effect.effect_type as u32 as f32,
                                        effect.params[0],
                                        effect.params[1],
                                        effect.corner_radius,
                                    ],
                                    clip_bounds: clip.clip_bounds_attr(),
                                    clip_radii: clip.clip_radii_attr(),
                                },
                                clip: clip.scissor,
                            });
                    }
                }
            }
            Primitive::Image(img) => {
                if let Some(clip) = clips.last().copied() {
                    if img.rect.intersection(clip.scissor).is_some() {
                        let zl = current_z!();
                        zl.images.push(ClippedImage {
                            primitive: img.clone(),
                            clip: clip.scissor,
                        });
                    }
                }
            }
            Primitive::EditorText(slot) => {
                if let Some(clip) = clips.last().copied()
                    && let Some(intersection) = slot.rect.intersection(clip.scissor)
                {
                    let zl = current_z!();
                    zl.editor_slots.push(ClippedEditorSlot {
                        slot: *slot,
                        clip: intersection,
                    });
                }
            }
            Primitive::Icon(icon) => {
                if let Some(clip) = clips.last().copied() {
                    if icon.rect.intersection(clip.scissor).is_some() {
                        let px_size = icon.rect.width.max(icon.rect.height).ceil() as u32;
                        let cache_key =
                            crate::ui::icons::cache_key(&icon.name, px_size, icon.color);
                        let (rgba, w, h) =
                            crate::ui::icons::rasterize_svg(&icon.name, px_size, icon.color);
                        let zl = current_z!();
                        zl.images.push(ClippedImage {
                            primitive: crate::render::scene::ImagePrimitive {
                                rect: crate::render::Rect {
                                    x: icon.rect.x.round(),
                                    y: icon.rect.y.round(),
                                    width: icon.rect.width.round(),
                                    height: icon.rect.height.round(),
                                },
                                width: w,
                                height: h,
                                rgba,
                                cache_key,
                            },
                            clip: clip.scissor,
                        });
                    }
                }
            }
            Primitive::ClipStart(ClipPrimitive { rect, corner_radii }) => {
                let next = clips
                    .last()
                    .and_then(|clip| clip.push(*rect, *corner_radii))
                    .unwrap_or_else(|| ActiveClip {
                        scissor: Rect::default(),
                        rounded_rect: Rect::default(),
                        corner_radii: [0.0; 4],
                    });
                clips.push(next);
            }
            Primitive::ClipEnd => {
                if clips.len() > 1 {
                    clips.pop();
                }
            }
            Primitive::ZIndexPush(z) => {
                z_index_stack.push(*z);
            }
            Primitive::ZIndexPop => {
                if z_index_stack.len() > 1 {
                    z_index_stack.pop();
                }
            }
            Primitive::LayerBoundary => {}
        }
    }

    // Collect z-layers sorted by z-index (BTreeMap is already sorted).
    flattened.z_layers = z_map.into_values().collect();
    flattened
}

fn append_editor_text_areas<'a>(
    text_areas: &mut Vec<glyphon::TextArea<'a>>,
    slots: &[ClippedEditorSlot],
    editors: &[Option<&'a crate::editor::Editor>],
) {
    for es in slots {
        let Some(Some(editor)) = editors.get(es.slot.editor_id as usize) else {
            continue;
        };
        let Some(buffer) = editor.buffer() else {
            continue;
        };
        let color = es.slot.color;
        text_areas.push(glyphon::TextArea {
            buffer,
            left: es.slot.rect.x,
            top: es.slot.rect.y - es.slot.scroll_y,
            scale: 1.0,
            bounds: glyphon::TextBounds {
                left: es.clip.x.round() as i32,
                top: es.clip.y.round() as i32,
                right: es.clip.right().round() as i32,
                bottom: es.clip.bottom().round() as i32,
            },
            default_color: GlyphonColor::rgba(color.r, color.g, color.b, color.a),
            custom_glyphs: &[],
        });
    }
}

fn push_quad(
    rect: Rect,
    background: [f32; 4],
    border_color: [f32; 4],
    corner_radii: [f32; 4],
    border_widths: [f32; 4],
    clips: &[ActiveClip],
    out: &mut Vec<ClippedQuad>,
) {
    if let Some(clip) = clips.last().copied() {
        if rect.intersection(clip.scissor).is_some() {
            out.push(ClippedQuad {
                instance: QuadInstance {
                    bounds: [rect.x, rect.y, rect.width, rect.height],
                    background,
                    border_color,
                    corner_radii,
                    border_widths,
                    clip_bounds: clip.clip_bounds_attr(),
                    clip_radii: clip.clip_radii_attr(),
                },
                clip: clip.scissor,
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

fn build_shadow_instances(
    shadows: &[ClippedShadow],
) -> (Vec<ShadowInstance>, Vec<QuadDrawCommand>) {
    let mut instances = Vec::with_capacity(shadows.len());
    let mut commands = Vec::with_capacity(shadows.len());

    let mut i = 0;
    while i < shadows.len() {
        let start = i as u32;
        let clip = shadows[i].clip;
        instances.push(shadows[i].instance);
        i += 1;

        while i < shadows.len() && rects_equal(shadows[i].clip, clip) {
            instances.push(shadows[i].instance);
            i += 1;
        }

        commands.push(QuadDrawCommand {
            instance_range: start..i as u32,
            clip,
        });
    }

    (instances, commands)
}

fn build_effect_quad_instances(
    effects: &[ClippedEffectQuad],
) -> (Vec<EffectQuadInstance>, Vec<QuadDrawCommand>) {
    let mut instances = Vec::with_capacity(effects.len());
    let mut commands = Vec::with_capacity(effects.len());

    let mut i = 0;
    while i < effects.len() {
        let start = i as u32;
        let clip = effects[i].clip;
        instances.push(effects[i].instance);
        i += 1;

        while i < effects.len() && rects_equal(effects[i].clip, clip) {
            instances.push(effects[i].instance);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scissor_rect_clamps_to_render_target() {
        let clip = Rect {
            x: 807.0,
            y: 118.0,
            width: 844.0,
            height: 865.0,
        };

        assert_eq!(scissor_rect(clip, 1650, 1050), Some((807, 118, 843, 865)));
    }

    #[test]
    fn scissor_rect_skips_clips_outside_render_target() {
        let clip = Rect {
            x: 1650.0,
            y: 118.0,
            width: 10.0,
            height: 865.0,
        };

        assert_eq!(scissor_rect(clip, 1650, 1050), None);
    }

    #[test]
    fn child_quad_inherits_rounded_clip_from_parent() {
        use crate::render::scene::{ClipPrimitive, Primitive, RoundedRectPrimitive, Scene};
        use crate::ui::theme::Color;

        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 36.0,
        };
        let cluster = Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 36.0,
        };
        let child = Rect {
            x: 300.0,
            y: 0.0,
            width: 100.0,
            height: 36.0,
        };

        let mut scene = Scene::default();
        // Parent bg (self-rounded, not clipped by anything)
        scene.push(Primitive::RoundedRect(RoundedRectPrimitive {
            rect: cluster,
            corner_radii: [6.0; 4],
            color: Color::rgba(100, 100, 100, 255),
        }));
        // Parent's rounded clip
        scene.push(Primitive::ClipStart(ClipPrimitive {
            rect: cluster,
            corner_radii: [6.0; 4],
        }));
        // Child bg (square corners, should inherit rounded clip)
        scene.push(Primitive::RoundedRect(RoundedRectPrimitive {
            rect: child,
            corner_radii: [0.0; 4],
            color: Color::rgba(200, 200, 0, 255),
        }));
        scene.push(Primitive::ClipEnd);

        let flat = flatten_scene(&scene, viewport);
        let quads: Vec<&QuadInstance> = flat
            .z_layers
            .iter()
            .flat_map(|z| z.draw_layers.iter())
            .flat_map(|dl| dl.quads.iter().map(|q| &q.instance))
            .collect();

        // Expect 2 quads: the parent bg and the child bg.
        assert_eq!(quads.len(), 2, "expected 2 quads, got {}", quads.len());

        // Parent's own rect is drawn before the clip is pushed — should have
        // no rounded-clip attribution (clip_radii all zero).
        let parent_quad = quads[0];
        assert_eq!(
            parent_quad.clip_radii, [0.0; 4],
            "parent bg (before ClipStart) should have no rounded clip: {:?}",
            parent_quad.clip_radii,
        );

        // Child is inside the rounded clip — should inherit clip_bounds =
        // cluster rect and clip_radii = [6; 4].
        let child_quad = quads[1];
        assert_eq!(
            child_quad.clip_radii,
            [6.0, 6.0, 6.0, 6.0],
            "child bg should inherit rounded clip radii from parent",
        );
        assert_eq!(
            child_quad.clip_bounds,
            [cluster.x, cluster.y, cluster.width, cluster.height],
            "child bg should inherit rounded clip bounds from parent",
        );
    }
}

// Text preparation, caching, and color helpers are in text.rs
// Shader source constants are in shaders.rs
