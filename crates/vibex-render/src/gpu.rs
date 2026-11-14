//! GPU backend (wgpu): every rect and glyph in a `Scene` becomes one
//! instance of a quad; glyph bitmaps live in a shelf-packed RGBA atlas.
//! Inline images get a texture each (cached by image uid) and break the
//! draw into a few more calls.

use crate::font::{Bitmap, FontSet, GlyphKey};
use crate::scene::{Clip, Item, Scene};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

const ATLAS_SIZE: u32 = 2048;
/// The atlas grows (once) when one frame's glyphs don't fit.
const MAX_ATLAS_SIZE: u32 = 4096;
/// The instance buffer starts here and shrinks back to it after a big frame.
const MIN_INSTANCES: usize = 16 * 1024;
/// Image textures not drawn for this many frames are dropped.
const IMAGE_TTL_FRAMES: u64 = 600;
/// Keep at most this much image texture memory.
const IMAGE_BUDGET: usize = 512 * 1024 * 1024;

/// Offscreen targets for a blurred backdrop, sized to the surface.
struct BlurTargets {
    size: (u32, u32),
    _scene: wgpu::Texture,
    scene_view: wgpu::TextureView,
    _a: wgpu::Texture,
    a_view: wgpu::TextureView,
    _b: wgpu::Texture,
    b_view: wgpu::TextureView,
    /// Blur passes: scene→a (h), a→b (v), b→a (h), a→b (v).
    passes: [wgpu::BindGroup; 4],
    /// The sharp scene and the blurred one, for the quad pipeline.
    scene_quad: wgpu::BindGroup,
    blur_quad: wgpu::BindGroup,
}

/// Where the backdrop blur happens in a frame.
#[derive(Clone, Copy)]
struct Backdrop {
    /// Instances before this draw into the offscreen scene.
    split: u32,
    rect: Clip,
    radius: f32,
}

struct GpuImage {
    bind_group: wgpu::BindGroup,
    _texture: wgpu::Texture,
    /// Texture size (may be smaller than the image if it was too big).
    last_used: u64,
    bytes: usize,
}

/// A run of instances drawn with one scissor + bind group.
#[derive(Clone, Copy)]
struct Segment {
    start: u32,
    clip: Option<Clip>,
    /// Image texture to bind (None = glyph atlas).
    image: Option<u64>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    uv0: [f32; 2],
    uv1: [f32; 2],
    color: [f32; 4],
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    screen: [f32; 2],
    _pad: [f32; 2],
}

#[derive(Clone, Copy)]
struct AtlasEntry {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    xmin: i32,
    ymin: i32,
    color: bool,
    fake_bold: bool,
}

struct Atlas {
    map: HashMap<GlyphKey, AtlasEntry>,
    size: u32,
    x: u32,
    y: u32,
    row_h: u32,
}

impl Atlas {
    fn new(size: u32) -> Self {
        Self { map: HashMap::new(), size, x: 0, y: 0, row_h: 0 }
    }
    fn clear(&mut self) {
        *self = Self::new(self.size);
    }
    /// Reserve space (1px gutter). None if full.
    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let size = self.size;
        if w.saturating_add(1) > size || h.saturating_add(1) > size {
            return None;
        }
        if self.x + w + 1 > size {
            self.x = 0;
            self.y += self.row_h;
            self.row_h = 0;
        }
        if self.y + h + 1 > size {
            return None;
        }
        let pos = (self.x, self.y);
        self.x += w + 1;
        self.row_h = self.row_h.max(h + 1);
        Some(pos)
    }
}

pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// None while a lost surface couldn't be made again (retried each frame).
    surface: Option<wgpu::Surface<'static>>,
    /// Makes a new surface for the window (after the old one is lost).
    make_surface: Box<dyn Fn() -> Option<wgpu::Surface<'static>> + Send + Sync>,
    config: wgpu::SurfaceConfiguration,
    /// Configure again after presenting (the last frame was suboptimal).
    reconfigure: bool,
    /// Set by wgpu when the device is gone (driver reset, GPU switch).
    lost: Arc<AtomicBool>,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    image_sampler: wgpu::Sampler,
    images: HashMap<u64, GpuImage>,
    frame_no: u64,
    max_texture: u32,
    uniforms: wgpu::Buffer,
    instances: wgpu::Buffer,
    instance_cap: usize,
    texture: wgpu::Texture,
    atlas: Atlas,
    font_generation: (u32, u32, usize),
    blur_pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    blur_params: [wgpu::Buffer; 4],
    blur: Option<BlurTargets>,
    /// The surface composites with what's behind the window.
    pub transparent: bool,
    pub adapter_info: String,
}

fn rgb_f32(c: u32, a: f32) -> [f32; 4] {
    [((c >> 16) & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, (c & 0xff) as f32 / 255.0, a]
}

impl GpuRenderer {
    /// Create a renderer for `target` (an `Arc<Window>`). With
    /// `want_transparency`, DX12 presents through DirectComposition so the
    /// window can be see-through.
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>> + Clone + Send + Sync + 'static,
        width: u32,
        height: u32,
        want_transparency: bool,
    ) -> Result<Self, String> {
        let mut backend_options = wgpu::BackendOptions::default();
        if want_transparency {
            backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }
        // Transparency needs DX12's DirectComposition swapchain, so ask for
        // DX12 only in that case; otherwise let wgpu pick DX12 or Vulkan.
        let backends = if want_transparency && cfg!(windows) { wgpu::Backends::DX12 } else { wgpu::Backends::DX12 | wgpu::Backends::VULKAN };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, backend_options, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let make_surface = {
            let (instance, target) = (instance.clone(), target.clone());
            move || instance.create_surface(target.clone()).ok()
        };
        let surface = instance.create_surface(target).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|e| e.to_string())?;
        let info = adapter.get_info();
        let adapter_info = format!("{} ({:?})", info.name, info.backend);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vibex"),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        // wgpu's default is to panic on any error; a lost device (sleep,
        // driver reset, GPU switch) or a bad frame should not kill the app
        let lost = Arc::new(AtomicBool::new(false));
        {
            let lost = lost.clone();
            device.set_device_lost_callback(move |reason, msg| {
                if reason != wgpu::DeviceLostReason::Destroyed {
                    eprintln!("vibex: GPU device lost: {msg}");
                }
                lost.store(true, Ordering::Relaxed);
            });
        }
        let errors = AtomicU32::new(0);
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            // a broken frame repeats every frame: say it a few times only
            if errors.fetch_add(1, Ordering::Relaxed) < 8 {
                eprintln!("vibex: GPU error: {e}");
            }
        }));

        let caps = surface.get_capabilities(&adapter);
        // Non-sRGB format: blend in sRGB space like the CPU backend does.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .or_else(|| caps.formats.first().copied())
            .ok_or("surface has no formats")?;
        let transparent = want_transparency && caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied);
        let mut config = surface.get_default_config(&adapter, width.max(1), height.max(1)).ok_or("surface not supported by adapter")?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.alpha_mode = if transparent { wgpu::CompositeAlphaMode::PreMultiplied } else { wgpu::CompositeAlphaMode::Opaque };
        config.view_formats = vec![];
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibex shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibex bind layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let (texture, bind_group) = atlas_texture(&device, &layout, &uniforms, &sampler, ATLAS_SIZE);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vibex pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4, 5 => Uint32
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibex pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // backdrop blur
        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vibex blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blur.wgsl").into()),
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vibex blur layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vibex blur pipeline layout"),
            bind_group_layouts: &[Some(&blur_layout)],
            immediate_size: 0,
        });
        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vibex blur pipeline"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState { module: &blur_shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blur_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let blur_params = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("blur params"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let max_texture = device.limits().max_texture_dimension_2d;
        let instance_cap = MIN_INSTANCES;
        let instances = Self::instance_buffer(&device, instance_cap);
        Ok(Self {
            device,
            queue,
            surface: Some(surface),
            make_surface: Box::new(make_surface),
            config,
            reconfigure: false,
            lost,
            pipeline,
            bind_group,
            layout,
            sampler,
            image_sampler,
            images: HashMap::new(),
            frame_no: 0,
            max_texture,
            uniforms,
            instances,
            instance_cap,
            texture,
            atlas: Atlas::new(ATLAS_SIZE),
            font_generation: (0, 0, 0),
            blur_pipeline,
            blur_layout,
            blur_params,
            blur: None,
            transparent,
            adapter_info,
        })
    }

    fn instance_buffer(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (cap * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        // keep inside the device's limit (a window spanning huge monitors)
        let (width, height) = (width.min(self.max_texture), height.min(self.max_texture));
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            if let Some(s) = &self.surface {
                s.configure(&self.device, &self.config);
            }
        }
    }

    /// The GPU device is gone (driver reset, GPU unplugged or switched).
    /// Frames are skipped from then on: make a new renderer or fall back
    /// to the CPU one.
    pub fn device_lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }

    /// Replace a lost surface (drop the old one first: the window can only
    /// have one swapchain).
    fn recreate_surface(&mut self) {
        self.surface = None;
        self.surface = (self.make_surface)();
        if let Some(s) = &self.surface {
            s.configure(&self.device, &self.config);
        }
    }

    /// A bigger glyph atlas, when one frame's glyphs don't fit. False if it
    /// can't grow.
    fn grow_atlas(&mut self) -> bool {
        let size = (self.atlas.size * 2).min(MAX_ATLAS_SIZE).min(self.max_texture);
        if size <= self.atlas.size {
            return false;
        }
        let (texture, bind_group) = atlas_texture(&self.device, &self.layout, &self.uniforms, &self.sampler, size);
        self.texture = texture;
        self.bind_group = bind_group;
        self.atlas = Atlas::new(size);
        true
    }

    /// Atlas entry for `key`, uploading the glyph on first use.
    fn entry(&mut self, fonts: &mut FontSet, key: GlyphKey) -> Option<Option<AtlasEntry>> {
        if let Some(e) = self.atlas.map.get(&key) {
            return Some(Some(*e));
        }
        let g = fonts.glyph(key);
        if g.width == 0 || g.height == 0 {
            return Some(None);
        }
        let (w, h) = (g.width as u32, g.height as u32);
        let (x, y) = self.atlas.alloc(w, h)?;
        let (data, color): (Vec<u8>, bool) = match &g.bitmap {
            Bitmap::Mono(cov) => (cov.iter().flat_map(|&a| [255, 255, 255, a]).collect(), false),
            Bitmap::Color(rgba) => (rgba.iter().flatten().copied().collect(), true),
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let e = AtlasEntry { x, y, w, h, xmin: g.xmin, ymin: g.ymin, color, fake_bold: g.fake_bold };
        self.atlas.map.insert(key, e);
        Some(Some(e))
    }

    /// Upload an image as a premultiplied texture (downscaled if it's larger
    /// than the device allows); returns the scale applied to its size, or
    /// None for an image with no pixels.
    fn image_entry(&mut self, img: &vibex_term::image::ImageData) -> Option<f32> {
        if !crate::scene::image_ok(img) {
            return None;
        }
        let frame = self.frame_no;
        let max = self.max_texture.max(1);
        let scale = (max as f32 / img.width.max(img.height) as f32).min(1.0);
        if let Some(e) = self.images.get_mut(&img.uid) {
            e.last_used = frame;
            return Some(scale);
        }
        let (w, h) = (((img.width as f32 * scale) as u32).clamp(1, max), ((img.height as f32 * scale) as u32).clamp(1, max));
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            let sy = ((y as f32 + 0.5) / scale) as u32;
            for x in 0..w {
                let sx = ((x as f32 + 0.5) / scale) as u32;
                let o = (sy.min(img.height - 1) * img.width + sx.min(img.width - 1)) as usize * 4;
                let a = img.rgba[o + 3] as u32;
                let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
                data.extend_from_slice(&[pm(img.rgba[o]), pm(img.rgba[o + 1]), pm(img.rgba[o + 2]), a as u8]);
            }
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("inline image"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("inline image"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.image_sampler) },
            ],
        });
        let bytes = data.len();
        self.images.insert(img.uid, GpuImage { bind_group, _texture: texture, last_used: frame, bytes });
        Some(scale)
    }

    /// Drop textures of images that haven't been drawn for a while, and
    /// the oldest ones when over budget.
    fn evict_images(&mut self) {
        let frame = self.frame_no;
        self.images.retain(|_, e| frame - e.last_used < IMAGE_TTL_FRAMES);
        let mut total: usize = self.images.values().map(|e| e.bytes).sum();
        while total > IMAGE_BUDGET {
            let Some((&oldest, _)) = self.images.iter().filter(|(_, e)| e.last_used != frame).min_by_key(|(_, e)| e.last_used) else {
                break;
            };
            total -= self.images.remove(&oldest).map_or(0, |e| e.bytes);
        }
    }

    /// Offscreen targets for the backdrop blur at the surface's size.
    fn blur_targets(&mut self) -> &BlurTargets {
        let (w, h) = (self.config.width, self.config.height);
        if self.blur.as_ref().is_none_or(|b| b.size != (w, h)) {
            let target = |label, w: u32, h: u32| {
                let t = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.config.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let v = t.create_view(&Default::default());
                (t, v)
            };
            let (scene, scene_view) = target("blur scene", w, h);
            let (a, a_view) = target("blur a", w.div_ceil(4), h.div_ceil(4));
            let (b, b_view) = target("blur b", w.div_ceil(4), h.div_ceil(4));
            let group = |layout: &wgpu::BindGroupLayout, buf: &wgpu::Buffer, view: &wgpu::TextureView| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("blur"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.image_sampler) },
                    ],
                })
            };
            let bl = &self.blur_layout;
            let passes = [
                group(bl, &self.blur_params[0], &scene_view),
                group(bl, &self.blur_params[1], &a_view),
                group(bl, &self.blur_params[2], &b_view),
                group(bl, &self.blur_params[3], &a_view),
            ];
            let scene_quad = group(&self.layout, &self.uniforms, &scene_view);
            let blur_quad = group(&self.layout, &self.uniforms, &b_view);
            self.blur = Some(BlurTargets { size: (w, h), _scene: scene, scene_view, _a: a, a_view, _b: b, b_view, passes, scene_quad, blur_quad });
        }
        self.blur.as_ref().expect("just made")
    }

    /// Draw the instances of `segments` that fall in `lo..hi`.
    fn draw_segments(&self, pass: &mut wgpu::RenderPass, segments: &[Segment], total: u32, lo: u32, hi: u32) {
        let (sw, sh) = (self.config.width, self.config.height);
        for (i, seg) in segments.iter().enumerate() {
            let start = seg.start.max(lo);
            let end = segments.get(i + 1).map_or(total, |s| s.start).min(hi);
            if end <= start {
                continue;
            }
            let bind = match seg.image.and_then(|uid| self.images.get(&uid)) {
                Some(img) => &img.bind_group,
                None if seg.image.is_some() => continue,
                None => &self.bind_group,
            };
            pass.set_bind_group(0, bind, &[]);
            let Some((x, y, w, h)) = scissor(seg.clip, sw, sh) else { continue };
            pass.set_scissor_rect(x, y, w, h);
            pass.draw(0..6, start..end);
        }
    }

    /// Build instances plus draw segments. If the atlas fills up mid-way:
    /// None when `strict`, else the glyphs that don't fit are left out.
    fn build(&mut self, scene: &Scene, fonts: &mut FontSet, strict: bool) -> Option<(Vec<Instance>, Vec<Segment>, Option<Backdrop>)> {
        let mut out = Vec::with_capacity(scene.items.len() + 16);
        let mut segments: Vec<Segment> = vec![Segment { start: 0, clip: None, image: None }];
        let mut clip_now: Option<Clip> = None;
        let mut backdrop = None;
        let s = self.atlas.size as f32;
        for item in &scene.items {
            match *item {
                Item::Blur(rect, radius) => {
                    // only the first one counts
                    if backdrop.is_none() {
                        let split = out.len() as u32;
                        backdrop = Some(Backdrop { split, rect, radius });
                        if segments.last().is_none_or(|l| l.start != split) {
                            segments.push(Segment { start: split, clip: clip_now, image: None });
                        }
                    }
                }
                Item::Clip(c) => {
                    clip_now = c;
                    let start = out.len() as u32;
                    match segments.last_mut() {
                        Some(last) if last.start == start => {
                            last.clip = c;
                            last.image = None;
                        }
                        _ => segments.push(Segment { start, clip: c, image: None }),
                    }
                }
                Item::Image(im) => {
                    let Some(img) = scene.images.get(im.index as usize) else { continue };
                    let Some(scale) = self.image_entry(img) else { continue };
                    let (tw, th) = ((img.width as f32 * scale).max(1.0), (img.height as f32 * scale).max(1.0));
                    let start = out.len() as u32;
                    match segments.last_mut() {
                        Some(last) if last.start == start => last.image = Some(img.uid),
                        _ => segments.push(Segment { start, clip: clip_now, image: Some(img.uid) }),
                    }
                    let [sx, sy, sw, sh] = im.src;
                    out.push(Instance {
                        pos: [im.x as f32, im.y as f32],
                        size: [im.w as f32, im.h as f32],
                        uv0: [sx * scale / tw, sy * scale / th],
                        uv1: [(sx + sw) * scale / tw, (sy + sh) * scale / th],
                        color: [1.0; 4],
                        kind: 2,
                    });
                    segments.push(Segment { start: start + 1, clip: clip_now, image: None });
                }
                Item::Rect(r) => out.push(Instance {
                    pos: [r.x as f32, r.y as f32],
                    size: [r.w as f32, r.h as f32],
                    uv0: [0.0; 2],
                    uv1: [0.0; 2],
                    color: rgb_f32(r.color, r.alpha as f32 / 255.0),
                    kind: 0,
                }),
                Item::Glyph(g) => {
                    let e = match self.entry(fonts, g.key) {
                        Some(Some(e)) => e,
                        Some(None) => continue,
                        None if strict => return None,
                        None => continue,
                    };
                    let x = (g.x + e.xmin) as f32;
                    let y = (g.y - e.ymin - e.h as i32) as f32;
                    let inst = Instance {
                        pos: [x, y],
                        size: [e.w as f32, e.h as f32],
                        uv0: [e.x as f32 / s, e.y as f32 / s],
                        uv1: [(e.x + e.w) as f32 / s, (e.y + e.h) as f32 / s],
                        color: rgb_f32(g.color, 1.0),
                        kind: if e.color { 2 } else { 1 },
                    };
                    out.push(inst);
                    if e.fake_bold {
                        out.push(Instance { pos: [x + 1.0, y], ..inst });
                    }
                }
            }
        }
        Some((out, segments, backdrop))
    }

    /// Draw a frame. `opacity` applies to the default background only.
    pub fn render(&mut self, scene: &Scene, fonts: &mut FontSet, opacity: f32) {
        if self.device_lost() {
            return;
        }
        self.frame_no += 1;
        if fonts.generation() != self.font_generation {
            self.font_generation = fonts.generation();
            self.atlas.clear();
        }
        let built = match self.build(scene, fonts, true) {
            Some(b) => Some(b),
            None => {
                // atlas full: start over with only this frame's glyphs
                self.atlas.clear();
                self.build(scene, fonts, true)
            }
        };
        let (mut instances, segments, backdrop) = match built {
            Some(b) => b,
            None => {
                // more glyphs than the atlas holds: a bigger one, and if
                // even that is full, the frame without the rest
                self.grow_atlas();
                self.atlas.clear();
                self.build(scene, fonts, false).unwrap_or_default()
            }
        };

        let Some(surface) = &self.surface else {
            self.recreate_surface();
            return;
        };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.reconfigure = true;
                t
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Validation => return,
            wgpu::CurrentSurfaceTexture::Outdated => {
                surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface();
                return;
            }
        };

        let scene_total = instances.len() as u32;
        let (sw, sh) = (self.config.width as f32, self.config.height as f32);
        if let Some(b) = backdrop {
            let full = Instance { pos: [0.0; 2], size: [sw, sh], uv0: [0.0; 2], uv1: [1.0; 2], color: [1.0; 4], kind: 2 };
            let (x, y, w, h) = (b.rect.x as f32, b.rect.y as f32, b.rect.w as f32, b.rect.h as f32);
            instances.push(full);
            instances.push(Instance { pos: [x, y], size: [w, h], uv0: [x / sw, y / sh], uv1: [(x + w) / sw, (y + h) / sh], ..full });
            // tap spacing in full-size pixels; two h+v rounds
            let spacing = (b.radius / 2.8).max(1.0);
            let h_step = [spacing / sw, 0.0, 0.0, 0.0];
            let v_step = [0.0, spacing / sh, 0.0, 0.0];
            for (i, st) in [h_step, v_step, h_step, v_step].iter().enumerate() {
                self.queue.write_buffer(&self.blur_params[i], 0, bytemuck::cast_slice(st));
            }
            self.blur_targets();
        }
        if let Some(cap) = instance_cap(self.instance_cap, instances.len()) {
            self.instance_cap = cap;
            self.instances = Self::instance_buffer(&self.device, cap);
        }
        if !instances.is_empty() {
            self.queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        }
        let u = Uniforms { screen: [self.config.width as f32, self.config.height as f32], _pad: [0.0; 2] };
        self.queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));

        let a = if self.transparent { opacity.clamp(0.0, 1.0) as f64 } else { 1.0 };
        let [r, g, b, _] = rgb_f32(scene.background, 1.0);
        let clear = wgpu::Color { r: r as f64 * a, g: g as f64 * a, b: b as f64 * a, a };

        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let begin = |encoder: &'_ mut wgpu::CommandEncoder, target: &'_ wgpu::TextureView, clear: wgpu::Color| {
            encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vibex frame"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime()
        };
        match (backdrop, self.blur.as_ref()) {
            (Some(bd), Some(bt)) => {
                // 1. the scene so far, offscreen
                {
                    let mut pass = begin(&mut encoder, &bt.scene_view, clear);
                    pass.set_pipeline(&self.pipeline);
                    pass.set_vertex_buffer(0, self.instances.slice(..));
                    self.draw_segments(&mut pass, &segments, scene_total, 0, bd.split);
                }
                // 2. blur it at quarter size
                let targets = [&bt.a_view, &bt.b_view, &bt.a_view, &bt.b_view];
                for (group, target) in bt.passes.iter().zip(targets) {
                    let mut pass = begin(&mut encoder, target, wgpu::Color::TRANSPARENT);
                    pass.set_pipeline(&self.blur_pipeline);
                    pass.set_bind_group(0, group, &[]);
                    pass.draw(0..3, 0..1);
                }
                // 3. sharp outside the rectangle, blurred inside, then the rest
                let mut pass = begin(&mut encoder, &view, wgpu::Color::TRANSPARENT);
                pass.set_pipeline(&self.pipeline);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                let (w, h) = (self.config.width, self.config.height);
                let r = bd.rect;
                let (rx0, ry0) = (r.x.clamp(0, w as i32) as u32, r.y.clamp(0, h as i32) as u32);
                let (rx1, ry1) = (r.x.saturating_add_unsigned(r.w).clamp(0, w as i32) as u32, r.y.saturating_add_unsigned(r.h).clamp(0, h as i32) as u32);
                pass.set_bind_group(0, &bt.scene_quad, &[]);
                for (x0, y0, x1, y1) in [(0, 0, w, ry0), (0, ry1, w, h), (0, ry0, rx0, ry1), (rx1, ry0, w, ry1)] {
                    if x1 > x0 && y1 > y0 {
                        pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                        pass.draw(0..6, scene_total..scene_total + 1);
                    }
                }
                if rx1 > rx0 && ry1 > ry0 {
                    pass.set_bind_group(0, &bt.blur_quad, &[]);
                    pass.set_scissor_rect(rx0, ry0, rx1 - rx0, ry1 - ry0);
                    pass.draw(0..6, scene_total + 1..scene_total + 2);
                }
                self.draw_segments(&mut pass, &segments, scene_total, bd.split, scene_total);
            }
            _ => {
                let mut pass = begin(&mut encoder, &view, clear);
                pass.set_pipeline(&self.pipeline);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                self.draw_segments(&mut pass, &segments, scene_total, 0, scene_total);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        if std::mem::take(&mut self.reconfigure) {
            if let Some(s) = &self.surface {
                s.configure(&self.device, &self.config);
            }
        }
        self.evict_images();
    }
}

/// The glyph atlas texture and the bind group that samples it.
fn atlas_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    size: u32,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glyph atlas"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("vibex bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    (texture, bind_group)
}

/// A new instance buffer size, if `cap` doesn't suit `n` instances: grow to
/// fit, or shrink after a frame much smaller than the buffer (one huge frame
/// shouldn't keep its buffer forever).
fn instance_cap(cap: usize, n: usize) -> Option<usize> {
    if n > cap {
        return Some(n.next_power_of_two());
    }
    let want = n.next_power_of_two().max(MIN_INSTANCES);
    (cap > MIN_INSTANCES && want * 4 <= cap).then_some(want)
}

/// A clip as a scissor rectangle inside the target; None when empty.
fn scissor(clip: Option<Clip>, sw: u32, sh: u32) -> Option<(u32, u32, u32, u32)> {
    let (x, y, w, h) = match clip {
        None => (0, 0, sw, sh),
        Some(c) => {
            let x0 = c.x.clamp(0, sw as i32) as u32;
            let y0 = c.y.clamp(0, sh as i32) as u32;
            let x1 = c.x.saturating_add_unsigned(c.w).clamp(0, sw as i32) as u32;
            let y1 = c.y.saturating_add_unsigned(c.h).clamp(0, sh as i32) as u32;
            (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
        }
    };
    (w > 0 && h > 0).then_some((x, y, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_fills_up_then_clears() {
        let mut a = Atlas::new(64);
        // 31x31 + gutter: four fit in 64x64, the fifth doesn't
        for _ in 0..4 {
            assert!(a.alloc(31, 31).is_some());
        }
        assert_eq!(a.alloc(31, 31), None);
        assert_eq!(a.alloc(u32::MAX, 1), None, "no overflow on a huge glyph");
        a.clear();
        assert_eq!(a.size, 64);
        assert_eq!(a.alloc(31, 31), Some((0, 0)));
    }

    #[test]
    fn instance_buffer_grows_and_shrinks_back() {
        assert_eq!(instance_cap(MIN_INSTANCES, 100), None);
        assert_eq!(instance_cap(MIN_INSTANCES, MIN_INSTANCES + 1), Some(MIN_INSTANCES * 2));
        // after one huge frame, an ordinary one gives the memory back
        let big = 1 << 22;
        assert_eq!(instance_cap(big, 5000), Some(MIN_INSTANCES));
        // but not for a frame that still needs a good part of it
        assert_eq!(instance_cap(big, big / 2), None);
    }

    #[test]
    fn scissor_survives_huge_and_negative_clips() {
        assert_eq!(scissor(None, 0, 10), None);
        let c = Clip { x: i32::MAX - 5, y: 0, w: u32::MAX, h: 10 };
        assert_eq!(scissor(Some(c), 100, 100), None);
        let c = Clip { x: -50, y: -50, w: u32::MAX, h: u32::MAX };
        assert_eq!(scissor(Some(c), 100, 80), Some((0, 0, 100, 80)));
        let c = Clip { x: 10, y: 10, w: 0, h: 5 };
        assert_eq!(scissor(Some(c), 100, 80), None);
    }
}
