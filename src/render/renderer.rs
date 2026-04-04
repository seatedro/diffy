use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

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

// ---------------------------------------------------------------------------
// TexturePool — reusable offscreen render targets
// ---------------------------------------------------------------------------

struct PooledTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    in_use: bool,
}

struct TexturePool {
    textures: Vec<PooledTexture>,
    format: wgpu::TextureFormat,
}

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

fn create_transient_buffer(
    device: &wgpu::Device,
    label: &'static str,
    size: u64,
) -> wgpu::Buffer {
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
        }
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
        self.textures.push(PooledTexture {
            texture,
            view,
            width: w,
            height: h,
            in_use: true,
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
    }

    /// Release all textures that haven't been used since the last trim.
    /// Call once per frame after rendering.
    fn trim(&mut self) {
        self.textures.retain(|t| t.in_use);
        for t in &mut self.textures {
            t.in_use = false;
        }
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

    pub fn render(&mut self, scene: &Scene, time_seconds: f32) -> Result<FrameStats, RenderError> {
        if self.surface_config.width == 0 || self.surface_config.height == 0 {
            return Ok(FrameStats::default());
        }

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
                        let sb =
                            buffer_pool.upload(device, queue, "diffy_shadow_instances", &si);
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

            let text_areas = prepare_text_areas(
                &mut self.font_system,
                &mut self.text_cache,
                &mut self.text_cache_frame,
                &zl.texts,
                &zl.rich_texts,
                self.scale_factor,
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
                let text_areas = prepare_text_areas(
                    &mut self.font_system,
                    &mut self.text_cache,
                    &mut self.text_cache_frame,
                    &zl.texts,
                    &zl.rich_texts,
                    self.scale_factor,
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
                let text_areas = prepare_text_areas(
                    &mut self.font_system,
                    &mut self.text_cache,
                    &mut self.text_cache_frame,
                    &all_texts,
                    &all_rich,
                    self.scale_factor,
                );

                // Full-screen blit of scene_tex.
                let scene_blit = BlitInstance {
                    bounds: [0.0, 0.0, sw as f32, sh as f32],
                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                    tint: [1.0, 1.0, 1.0, 1.0],
                };
                let scene_blit_buf = self
                    .instance_buffer_pool
                    .upload(
                        &self.device,
                        &self.queue,
                        "diffy_scene_blit",
                        &[scene_blit],
                    )
                    .expect("single blit upload");
                // Blur region blit.
                let blur_blit = BlitInstance {
                    bounds: [br.x, br.y, br.width, br.height],
                    uv_rect: [uv_min_x, uv_min_y, uv_max_x, uv_max_y],
                    tint: [1.0, 1.0, 1.0, 1.0],
                };
                let blur_blit_buf = self
                    .instance_buffer_pool
                    .upload(
                        &self.device,
                        &self.queue,
                        "diffy_blur_blit",
                        &[blur_blit],
                    )
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
) {
    for lb in layers {
        if let Some(ref shadow_buf) = lb.shadow_buffer {
            pass.set_pipeline(shadow_pipeline);
            pass.set_bind_group(0, viewport_bind_group, &[]);
            pass.set_vertex_buffer(0, shadow_buf.slice(..));
            for command in &lb.shadow_commands {
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

        if let Some(ref effect_buf) = lb.effect_buffer {
            pass.set_pipeline(effect_quad_pipeline);
            pass.set_bind_group(0, viewport_bind_group, &[]);
            pass.set_vertex_buffer(0, effect_buf.slice(..));
            for command in &lb.effect_commands {
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

        if let Some(ref quad_buf) = lb.quad_buffer {
            pass.set_pipeline(quad_pipeline);
            pass.set_bind_group(0, viewport_bind_group, &[]);
            pass.set_vertex_buffer(0, quad_buf.slice(..));
            for command in &lb.quad_commands {
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

        pass.set_pipeline(blit_pipeline);
        pass.set_bind_group(0, viewport_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.set_scissor_rect(0, 0, viewport_w, viewport_h);
        pass.draw(0..4, 0..1);
    }
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
struct ShadowInstance {
    /// Expanded quad bounds (x, y, w, h) — covers the full blur extent.
    draw_bounds: [f32; 4],
    /// Original shadow-casting rect (x, y, w, h) before expansion.
    shadow_bounds: [f32; 4],
    /// Shadow color (linear RGBA, premultiplied).
    color: [f32; 4],
    /// [blur_sigma, corner_radius, 0, 0]
    params: [f32; 4],
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
    corner_radius: f32,
    /// Index into the layers vec: this blur applies after all layers
    /// up to (but not including) this index have been rendered.
    layer_break: usize,
}

/// A z-layer groups all draw layers and text for one z-index value.
/// Rendered in z-index order: lower z-indices first, higher on top.
#[derive(Debug, Clone, Default)]
struct ZLayer {
    z_index: i32,
    draw_layers: Vec<DrawLayer>,
    texts: Vec<ClippedText>,
    rich_texts: Vec<ClippedRichText>,
    images: Vec<ClippedImage>,
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
struct CachedTextBuffer {
    buffer: Buffer,
    left: f32,
    top: f32,
    clip: Rect,
    default_color: GlyphonColor,
    last_used_frame: u64,
}

fn flatten_scene(scene: &Scene, viewport: Rect) -> FlattenedScene {
    use std::collections::BTreeMap;

    let mut clips = vec![viewport];
    let mut z_index_stack = vec![0i32];
    let mut z_map: BTreeMap<i32, ZLayer> = BTreeMap::new();

    // Ensure z=0 always exists.
    z_map.insert(
        0,
        ZLayer {
            z_index: 0,
            ..Default::default()
        },
    );

    let mut flattened = FlattenedScene {
        z_layers: Vec::new(),
        blur_regions: Vec::new(),
    };

    // Helper: get (or create) the current z-layer's draw layers.
    macro_rules! current_z {
        () => {{
            let z = *z_index_stack.last().unwrap();
            z_map.entry(z).or_insert_with(|| ZLayer {
                z_index: z,
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
                    if expanded.intersection(clip).is_some() {
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
                                },
                                clip,
                            });
                    }
                }
            }
            Primitive::TextRun(text) => {
                if let Some(clip) = clips.last().copied()
                    && let Some(intersection) = text.rect.intersection(clip)
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
                    && let Some(intersection) = text.rect.intersection(clip)
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
                    if blur.rect.intersection(clip).is_some() {
                        let zl = current_z!();
                        ensure_draw_layer!(zl);
                        zl.draw_layers.push(DrawLayer::default());
                        flattened.blur_regions.push(FlattenedBlurRegion {
                            rect: blur.rect,
                            blur_radius: blur.blur_radius,
                            corner_radius: blur.corner_radius,
                            layer_break: zl.draw_layers.len() - 1,
                        });
                    }
                }
            }
            Primitive::EffectQuad(effect) => {
                if let Some(clip) = clips.last().copied() {
                    if effect.rect.intersection(clip).is_some() {
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
                                },
                                clip,
                            });
                    }
                }
            }
            Primitive::Image(img) => {
                if let Some(clip) = clips.last().copied() {
                    if img.rect.intersection(clip).is_some() {
                        let zl = current_z!();
                        zl.images.push(ClippedImage {
                            primitive: img.clone(),
                            clip,
                        });
                    }
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

// ---------------------------------------------------------------------------
// Text preparation
// ---------------------------------------------------------------------------

fn prepare_text_areas<'a>(
    font_system: &mut FontSystem,
    text_cache: &'a mut HashMap<u64, CachedTextBuffer>,
    text_cache_frame: &mut u64,
    texts: &[ClippedText],
    rich_texts: &[ClippedRichText],
    scale_factor: f64,
) -> Vec<TextArea<'a>> {
        *text_cache_frame = text_cache_frame.wrapping_add(1);
        let frame = *text_cache_frame;
        let mut keys = Vec::with_capacity(texts.len() + rich_texts.len());

        for text in texts {
            let key = plain_text_cache_key(&text.primitive, text.clip, scale_factor);
            if !text_cache.contains_key(&key) {
                let prepared = build_plain_text_buffer(
                    font_system,
                    &text.primitive,
                    text.clip,
                    scale_factor,
                    frame,
                );
                text_cache.insert(key, prepared);
            }
            if let Some(entry) = text_cache.get_mut(&key) {
                entry.last_used_frame = frame;
            }
            keys.push(key);
        }

        for text in rich_texts {
            let key = rich_text_cache_key(&text.primitive, text.clip, scale_factor);
            if !text_cache.contains_key(&key) {
                let prepared = build_rich_text_buffer(
                    font_system,
                    &text.primitive,
                    text.clip,
                    scale_factor,
                    frame,
                );
                text_cache.insert(key, prepared);
            }
            if let Some(entry) = text_cache.get_mut(&key) {
                entry.last_used_frame = frame;
            }
            keys.push(key);
        }

        if frame % 240 == 0 {
            trim_text_cache(text_cache, frame);
        }

        keys.iter()
            .filter_map(|key| text_cache.get(key).map(text_area_from_cache))
            .collect()
}

fn text_area_from_cache(prepared: &CachedTextBuffer) -> TextArea<'_> {
    TextArea {
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
    }
}

fn build_plain_text_buffer(
    font_system: &mut FontSystem,
    primitive: &TextPrimitive,
    clip: Rect,
    scale_factor: f64,
    last_used_frame: u64,
) -> CachedTextBuffer {
    let metrics = Metrics::new(primitive.font_size, primitive.font_size * 1.35);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some((primitive.rect.width * scale_factor as f32).max(1.0)),
        Some((primitive.rect.height * scale_factor as f32).max(1.0)),
    );
    let attrs = attrs_for_font(primitive.font_kind, primitive.font_weight, primitive.color);
    buffer.set_text(
        font_system,
        primitive.text.as_ref(),
        &attrs,
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    CachedTextBuffer {
        buffer,
        left: primitive.rect.x,
        top: primitive.rect.y,
        clip,
        default_color: glyphon_color(primitive.color),
        last_used_frame,
    }
}

fn build_rich_text_buffer(
    font_system: &mut FontSystem,
    primitive: &RichTextPrimitive,
    clip: Rect,
    scale_factor: f64,
    last_used_frame: u64,
) -> CachedTextBuffer {
    let metrics = Metrics::new(primitive.font_size, primitive.font_size * 1.35);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(
        font_system,
        Some((primitive.rect.width * scale_factor as f32).max(1.0)),
        Some((primitive.rect.height * scale_factor as f32).max(1.0)),
    );
    let default_attrs = attrs_for_font(
        primitive.font_kind,
        primitive.font_weight,
        primitive.default_color,
    );
    let spans = primitive
        .spans
        .iter()
        .map(|span| {
            (
                span.text.as_ref(),
                attrs_for_font(primitive.font_kind, primitive.font_weight, span.color),
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
    CachedTextBuffer {
        buffer,
        left: primitive.rect.x,
        top: primitive.rect.y,
        clip,
        default_color: glyphon_color(primitive.default_color),
        last_used_frame,
    }
}

fn trim_text_cache(cache: &mut HashMap<u64, CachedTextBuffer>, frame: u64) {
    const KEEP_UNUSED_FRAMES: u64 = 240;
    cache.retain(|_, entry| frame.saturating_sub(entry.last_used_frame) <= KEEP_UNUSED_FRAMES);
}

fn plain_text_cache_key(primitive: &TextPrimitive, clip: Rect, scale_factor: f64) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write_u8(0);
    hash_rect(&mut hasher, primitive.rect);
    hash_rect(&mut hasher, clip);
    hasher.write_u32(primitive.font_size.to_bits());
    hasher.write_u64(scale_factor.to_bits());
    hasher.write_u8(font_kind_tag(primitive.font_kind));
    hasher.write_u8(font_weight_tag(primitive.font_weight));
    hash_color(&mut hasher, primitive.color);
    primitive.text.hash(&mut hasher);
    hasher.finish()
}

fn rich_text_cache_key(primitive: &RichTextPrimitive, clip: Rect, scale_factor: f64) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write_u8(1);
    hash_rect(&mut hasher, primitive.rect);
    hash_rect(&mut hasher, clip);
    hasher.write_u32(primitive.font_size.to_bits());
    hasher.write_u64(scale_factor.to_bits());
    hasher.write_u8(font_kind_tag(primitive.font_kind));
    hasher.write_u8(font_weight_tag(primitive.font_weight));
    hash_color(&mut hasher, primitive.default_color);
    hasher.write_usize(primitive.spans.len());
    for span in primitive.spans.iter() {
        span.text.hash(&mut hasher);
        hash_color(&mut hasher, span.color);
    }
    hasher.finish()
}

fn hash_rect(hasher: &mut DefaultHasher, rect: Rect) {
    hasher.write_u32(rect.x.to_bits());
    hasher.write_u32(rect.y.to_bits());
    hasher.write_u32(rect.width.to_bits());
    hasher.write_u32(rect.height.to_bits());
}

fn hash_color(hasher: &mut DefaultHasher, color: Color) {
    hasher.write_u8(color.r);
    hasher.write_u8(color.g);
    hasher.write_u8(color.b);
    hasher.write_u8(color.a);
}

fn font_kind_tag(kind: FontKind) -> u8 {
    match kind {
        FontKind::Ui => 0,
        FontKind::Mono => 1,
    }
}

fn font_weight_tag(weight: crate::render::scene::FontWeight) -> u8 {
    match weight {
        crate::render::scene::FontWeight::Normal => 0,
        crate::render::scene::FontWeight::Medium => 1,
        crate::render::scene::FontWeight::Semibold => 2,
        crate::render::scene::FontWeight::Bold => 3,
    }
}

fn attrs_for_font(
    font_kind: FontKind,
    font_weight: crate::render::scene::FontWeight,
    color: Color,
) -> Attrs<'static> {
    let family = match font_kind {
        FontKind::Ui => Family::SansSerif,
        FontKind::Mono => Family::Monospace,
    };
    let weight = match font_weight {
        crate::render::scene::FontWeight::Normal => glyphon::Weight::NORMAL,
        crate::render::scene::FontWeight::Medium => glyphon::Weight(500),
        crate::render::scene::FontWeight::Semibold => glyphon::Weight(600),
        crate::render::scene::FontWeight::Bold => glyphon::Weight::BOLD,
    };
    Attrs::new()
        .family(family)
        .weight(weight)
        .color(glyphon_text_color(color))
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
// Gaussian shadow shader
// ---------------------------------------------------------------------------

const SHADOW_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) draw_bounds: vec4<f32>,
    @location(1) shadow_bounds: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) shadow_bounds: vec4<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs_shadow(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.draw_bounds.xy + unit * input.draw_bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.shadow_bounds = input.shadow_bounds;
    out.color = input.color;
    out.params = input.params;
    return out;
}

// Attempt to approximate erf using a polynomial fit.
// Abramowitz & Stegun 7.1.26 — max error < 1.5e-7, which is more than
// enough for visual blur.
fn erf_approx(x: f32) -> f32 {
    let sign = select(-1.0, 1.0, x >= 0.0);
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;
    let t5 = t4 * t;
    let poly = 0.254829592 * t - 0.284496736 * t2 + 1.421413741 * t3
             - 1.453152027 * t4 + 1.061405429 * t5;
    return sign * (1.0 - poly * exp(-a * a));
}

// Integral of 1D Gaussian from -inf to x with given sigma.
fn gauss_integral(x: f32, sigma: f32) -> f32 {
    return 0.5 + 0.5 * erf_approx(x / (sigma * 1.4142135));
}

// Rounded-rect SDF (distance from point p to the rounded rect centered at
// origin with given half_size and corner_radius).
fn rounded_rect_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

@fragment
fn fs_shadow(input: VertexOutput) -> @location(0) vec4<f32> {
    let sigma = input.params.x;
    let corner_radius = input.params.y;
    let half_size = input.shadow_bounds.zw * 0.5;
    let center = input.shadow_bounds.xy + half_size;
    let p = input.position.xy - center;

    // For the blurred shadow, we compute the convolution of the rounded-rect
    // indicator function with a 2D Gaussian. For a box (no rounding), this
    // factors into the product of two 1D Gaussian integrals. For rounded
    // corners we use a hybrid: compute the box integral and multiply by a
    // smooth SDF-based corner correction.

    // Separable box blur integral.
    let ax = gauss_integral(p.x + half_size.x, sigma)
           - gauss_integral(p.x - half_size.x, sigma);
    let ay = gauss_integral(p.y + half_size.y, sigma)
           - gauss_integral(p.y - half_size.y, sigma);
    var alpha = ax * ay;

    // Corner correction: fade out the corners that the box integral
    // over-estimates. We sample the SDF and use the sigma to smooth it.
    if (corner_radius > 0.0) {
        let sdf = rounded_rect_sdf(p, half_size, corner_radius);
        // Outside the rounded rect, attenuate based on how far outside.
        // The smoothstep range is proportional to sigma for a soft edge.
        let corner_fade = 1.0 - smoothstep(-sigma * 0.5, sigma * 1.5, sdf);
        alpha = alpha * corner_fade;
    }

    let final_alpha = input.color.a * alpha;
    if (final_alpha < 0.001) {
        discard;
    }
    return vec4<f32>(input.color.rgb * final_alpha, final_alpha);
}
"#;

// ---------------------------------------------------------------------------
// SDF quad shader
// ---------------------------------------------------------------------------

const QUAD_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
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

// ---------------------------------------------------------------------------
// Procedural effect shader — noise gradient + linear gradient
// ---------------------------------------------------------------------------

const EFFECT_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,
    @location(1) color_a: vec4<f32>,
    @location(2) color_b: vec4<f32>,
    @location(3) params: vec4<f32>,   // [effect_type, param1, param2, corner_radius]
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    @location(1) @interpolate(flat) color_a: vec4<f32>,
    @location(2) @interpolate(flat) color_b: vec4<f32>,
    @location(3) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs_effect(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.bounds = input.bounds;
    out.color_a = input.color_a;
    out.color_b = input.color_b;
    out.params = input.params;
    return out;
}

// ---- Simplex noise (2D) ----

fn mod289_v3(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn mod289_v2(x: vec2<f32>) -> vec2<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn permute(x: vec3<f32>) -> vec3<f32> {
    return mod289_v3(((x * 34.0) + vec3<f32>(10.0)) * x);
}

fn simplex_noise(v: vec2<f32>) -> f32 {
    let C = vec4<f32>(
        0.211324865405187,   // (3.0 - sqrt(3.0)) / 6.0
        0.366025403784439,   // 0.5 * (sqrt(3.0) - 1.0)
        -0.577350269189626,  // -1.0 + 2.0 * C.x
        0.024390243902439    // 1.0 / 41.0
    );

    // First corner.
    var i = floor(v + dot(v, C.yy));
    let x0 = v - i + dot(i, C.xx);

    // Other corners.
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    var x12 = x0.xyxy + C.xxzz;
    x12 = vec4<f32>(x12.xy - i1, x12.zw);

    // Permutations.
    i = mod289_v2(i);
    let p = permute(permute(
        i.y + vec3<f32>(0.0, i1.y, 1.0))
      + i.x + vec3<f32>(0.0, i1.x, 1.0));

    var m = max(vec3<f32>(0.5) - vec3<f32>(
        dot(x0, x0),
        dot(x12.xy, x12.xy),
        dot(x12.zw, x12.zw)
    ), vec3<f32>(0.0));
    m = m * m;
    m = m * m;

    // Gradients.
    let x_ = 2.0 * fract(p * C.www) - vec3<f32>(1.0);
    let h = abs(x_) - vec3<f32>(0.5);
    let ox = floor(x_ + vec3<f32>(0.5));
    let a0 = x_ - ox;

    // Approximate normalisation.
    m = m * (vec3<f32>(1.79284291400159) - vec3<f32>(0.85373472095314) * (a0 * a0 + h * h));

    // Compute final noise value at P.
    let g = vec3<f32>(
        a0.x * x0.x + h.x * x0.y,
        a0.y * x12.x + h.y * x12.y,
        a0.z * x12.z + h.z * x12.w
    );

    return 130.0 * dot(m, g);
}

// ---- Rounded-rect SDF for masking ----

fn effect_sdf(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// ---- Fragment shader ----

@fragment
fn fs_effect(input: VertexOutput) -> @location(0) vec4<f32> {
    let half_size = input.bounds.zw * 0.5;
    let center = input.bounds.xy + half_size;
    let p = input.position.xy - center;
    let corner_radius = input.params.w;

    // Rounded-rect mask.
    let sdf = effect_sdf(p, half_size, corner_radius);
    let mask = saturate(0.5 - sdf);
    if (mask <= 0.0) {
        discard;
    }

    // Normalised UV within the element bounds.
    let uv = (input.position.xy - input.bounds.xy) / input.bounds.zw;

    let effect_type = u32(input.params.x);
    var color: vec4<f32>;

    switch (effect_type) {
        // Type 0: Noise gradient — simplex noise blended between two colors.
        case 0u: {
            let scale = input.params.y;
            let noise_coord = input.position.xy * scale + vec2<f32>(viewport.time * 3.0);
            let n = simplex_noise(noise_coord) * 0.5 + 0.5;
            // Layer a second octave for richer texture.
            let n2 = simplex_noise(noise_coord * 2.0 + vec2<f32>(17.3, 31.7)) * 0.5 + 0.5;
            let combined = n * 0.7 + n2 * 0.3;
            // Blend from color_a (top) to color_b (bottom) modulated by noise.
            let gradient = uv.y;
            let t = saturate(gradient + (combined - 0.5) * 0.4);
            color = mix(input.color_a, input.color_b, t);
        }
        // Type 1: Linear gradient with angle.
        case 1u: {
            let angle = input.params.y;
            let dir = vec2<f32>(cos(angle), sin(angle));
            let t = saturate(dot(uv - vec2<f32>(0.5), dir) + 0.5);
            color = mix(input.color_a, input.color_b, t);
        }
        // Type 2: Radial gradient — color_a at center, color_b at edge.
        case 2u: {
            let center = vec2<f32>(0.5, 0.5);
            let d = length((uv - center) * 2.0);
            let t = saturate(d);
            color = mix(input.color_a, input.color_b, t);
        }
        // Type 3: Animated shimmer — diagonal highlight sweep.
        case 3u: {
            let speed = input.params.y;
            // Diagonal position: combine x and y into a single sweep axis.
            let diag = (uv.x + uv.y) * 0.5;
            // Animate the highlight band across the diagonal.
            let phase = fract(viewport.time * speed * 0.3);
            let band_center = phase * 1.6 - 0.3; // sweep from left to right with overshoot
            let band = 1.0 - smoothstep(0.0, 0.15, abs(diag - band_center));
            color = mix(input.color_a, input.color_b, band);
        }
        // Type 4: Vignette — darken/tint edges.
        case 4u: {
            let intensity = input.params.y;
            let center = vec2<f32>(0.5, 0.5);
            let d = length((uv - center) * 2.0);
            let vignette_factor = smoothstep(0.2, 1.2, d) * intensity;
            // Start from transparent, blend toward color_a at edges.
            color = vec4<f32>(input.color_a.rgb, input.color_a.a * vignette_factor);
        }
        // Type 5: Color tint — flat semi-transparent overlay.
        case 5u: {
            color = input.color_a;
        }
        // Fallback: solid color_a.
        default: {
            color = input.color_a;
        }
    }

    let final_alpha = color.a * mask;
    if (final_alpha < 0.001) {
        discard;
    }
    return vec4<f32>(color.rgb * final_alpha, final_alpha);
}
"#;

// ---------------------------------------------------------------------------
// Blit shader — composite an offscreen texture to screen
// ---------------------------------------------------------------------------

const BLIT_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

@group(1) @binding(0)
var t_source: texture_2d<f32>;
@group(1) @binding(1)
var s_source: sampler;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,    // screen-space destination [x, y, w, h]
    @location(1) uv_rect: vec4<f32>,   // source UV [u_min, v_min, u_max, v_max]
    @location(2) tint: vec4<f32>,      // tint/opacity
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) tint: vec4<f32>,
};

@vertex
fn vs_blit(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    // Interpolate UV from uv_rect min→max.
    let uv = mix(input.uv_rect.xy, input.uv_rect.zw, unit);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.tint = input.tint;
    return out;
}

@fragment
fn fs_blit(input: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_source, s_source, input.uv);
    return tex_color * input.tint;
}
"#;

// ---------------------------------------------------------------------------
// Separable Gaussian blur shader — 13-tap kernel
// ---------------------------------------------------------------------------

const BLUR_SHADER: &str = r#"
struct ViewportUniform {
    resolution: vec2<f32>,
    time: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> viewport: ViewportUniform;

@group(1) @binding(0)
var t_source: texture_2d<f32>;
@group(1) @binding(1)
var s_source: sampler;

struct VertexInput {
    @builtin(vertex_index) vertex_id: u32,
    @location(0) bounds: vec4<f32>,       // [x, y, w, h] in pixel coords
    @location(1) uv_rect: vec4<f32>,      // [u_min, v_min, u_max, v_max]
    @location(2) blur_params: vec4<f32>,  // [dir_x, dir_y, sigma, 0]
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) blur_params: vec4<f32>,
};

@vertex
fn vs_blur(input: VertexInput) -> VertexOutput {
    let unit = vec2<f32>(
        f32(input.vertex_id & 1u),
        f32((input.vertex_id >> 1u) & 1u)
    );
    let pixel_pos = input.bounds.xy + unit * input.bounds.zw;
    let ndc = pixel_pos / viewport.resolution * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    let uv = mix(input.uv_rect.xy, input.uv_rect.zw, unit);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.blur_params = input.blur_params;
    return out;
}

@fragment
fn fs_blur(input: VertexOutput) -> @location(0) vec4<f32> {
    let sigma = input.blur_params.z;
    let dir = vec2<f32>(input.blur_params.x, input.blur_params.y);
    let tex_size = vec2<f32>(textureDimensions(t_source));
    // Step size in UV space, scaled up for large blur radii.
    let step_scale = max(1.0, sigma / 6.0);
    let texel = dir / tex_size * step_scale;

    // 13-tap Gaussian kernel (offsets -6..+6).
    var color = vec4<f32>(0.0);
    var total_weight = 0.0;

    let w0 = exp(0.0);
    let w1 = exp(-1.0 / (2.0 * sigma * sigma));
    let w2 = exp(-4.0 / (2.0 * sigma * sigma));
    let w3 = exp(-9.0 / (2.0 * sigma * sigma));
    let w4 = exp(-16.0 / (2.0 * sigma * sigma));
    let w5 = exp(-25.0 / (2.0 * sigma * sigma));
    let w6 = exp(-36.0 / (2.0 * sigma * sigma));

    color += textureSample(t_source, s_source, input.uv + texel * -6.0) * w6;
    color += textureSample(t_source, s_source, input.uv + texel * -5.0) * w5;
    color += textureSample(t_source, s_source, input.uv + texel * -4.0) * w4;
    color += textureSample(t_source, s_source, input.uv + texel * -3.0) * w3;
    color += textureSample(t_source, s_source, input.uv + texel * -2.0) * w2;
    color += textureSample(t_source, s_source, input.uv + texel * -1.0) * w1;
    color += textureSample(t_source, s_source, input.uv)                * w0;
    color += textureSample(t_source, s_source, input.uv + texel *  1.0) * w1;
    color += textureSample(t_source, s_source, input.uv + texel *  2.0) * w2;
    color += textureSample(t_source, s_source, input.uv + texel *  3.0) * w3;
    color += textureSample(t_source, s_source, input.uv + texel *  4.0) * w4;
    color += textureSample(t_source, s_source, input.uv + texel *  5.0) * w5;
    color += textureSample(t_source, s_source, input.uv + texel *  6.0) * w6;

    total_weight = w6 + w5 + w4 + w3 + w2 + w1 + w0 + w1 + w2 + w3 + w4 + w5 + w6;

    return color / total_weight;
}
"#;
