use std::{
    collections::HashMap,
    error::Error,
    hash::{DefaultHasher, Hash, Hasher},
    ptr::NonNull,
};

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

use crate::overlay::OverlayRenderer;

#[derive(Clone, Debug)]
pub struct RenderText {
    pub id: String,
    pub text: String,
    pub left: f32,
    pub top: f32,
    pub font_size: f32,
    pub bold: bool,
    pub color: [u8; 4],
    pub shadow: Option<[u8; 4]>,
}

struct CachedBuffer {
    signature: u64,
    buffer: Buffer,
}

pub struct WgpuTextRenderer {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    buffers: HashMap<String, CachedBuffer>,
    items: Vec<RenderText>,
    logical_width: u32,
    logical_height: u32,
    scale: i32,
}

impl WgpuTextRenderer {
    pub fn new(conn: &Connection, wl_surface: &WlSurface) -> Result<Self, Box<dyn Error>> {
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::GL;
        let instance = wgpu::Instance::new(instance_descriptor);
        let display = NonNull::new(conn.backend().display_ptr() as *mut _)
            .ok_or("Wayland returned a null display pointer")?;
        let window = NonNull::new(wl_surface.id().as_ptr() as *mut _)
            .ok_or("Wayland returned a null surface pointer")?;
        let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display));
        let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(window));

        // SAFETY: `App` owns the Wayland connection and layer surface for longer
        // than this renderer, and explicitly drops the renderer first.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(raw_display_handle),
                raw_window_handle,
            })?
        };
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or("wgpu surface exposes no texture formats")?;
        let alpha_mode = [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
        ]
        .into_iter()
        .find(|candidate| capabilities.alpha_modes.contains(candidate))
        .ok_or("wgpu surface exposes no transparent alpha mode")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: 1,
            height: 1,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![format],
            color_space: wgpu::SurfaceColorSpace::Srgb,
        };
        surface.configure(&device, &config);

        let font_system = bundled_font_system();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);

        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            buffers: HashMap::new(),
            items: Vec::new(),
            logical_width: 1,
            logical_height: 1,
            scale: 1,
        })
    }

    pub fn set_items(&mut self, items: Vec<RenderText>) {
        self.items = items;
    }

    fn configure(&mut self) {
        self.config.width = self.logical_width.saturating_mul(self.scale as u32).max(1);
        self.config.height = self.logical_height.saturating_mul(self.scale as u32).max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn update_buffers(&mut self) {
        self.buffers
            .retain(|id, _| self.items.iter().any(|item| &item.id == id));
        for item in &self.items {
            let mut hasher = DefaultHasher::new();
            item.text.hash(&mut hasher);
            item.font_size.to_bits().hash(&mut hasher);
            item.bold.hash(&mut hasher);
            self.scale.hash(&mut hasher);
            let signature = hasher.finish();
            let cached = self
                .buffers
                .entry(item.id.clone())
                .or_insert_with(|| CachedBuffer {
                    signature: 0,
                    buffer: Buffer::new(&mut self.font_system, Metrics::new(16.0, 19.0)),
                });
            if cached.signature != signature {
                let size = item.font_size * self.scale as f32;
                cached.buffer.set_metrics(Metrics::new(size, size * 1.2));
                cached.buffer.set_size(
                    Some(self.config.width as f32),
                    Some(self.config.height as f32),
                );
                let weight = if item.bold {
                    Weight::BOLD
                } else {
                    Weight::NORMAL
                };
                cached.buffer.set_text(
                    &item.text,
                    &Attrs::new()
                        .family(Family::Name("Noto Sans"))
                        .weight(weight),
                    Shaping::Advanced,
                    None,
                );
                cached
                    .buffer
                    .shape_until_scroll(&mut self.font_system, false);
                cached.signature = signature;
            }
        }
    }

    fn render_frame(&mut self) -> Result<(), Box<dyn Error>> {
        self.update_buffers();
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        let scale = self.scale as f32;
        let bounds = TextBounds {
            left: 0,
            top: 0,
            right: self.config.width as i32,
            bottom: self.config.height as i32,
        };
        let mut areas = Vec::with_capacity(self.items.len() * 9);
        for item in &self.items {
            let buffer = &self.buffers[&item.id].buffer;
            if let Some(shadow) = item.shadow {
                // The script-level `shadow` color is rendered on every side of
                // the glyphs. Keeping the existing property avoids breaking
                // HUD scripts while making it a legible one-pixel outline.
                for (offset_x, offset_y) in OUTLINE_OFFSETS {
                    areas.push(TextArea {
                        buffer,
                        left: (item.left + offset_x) * scale,
                        top: (item.top + offset_y) * scale,
                        scale: 1.0,
                        bounds,
                        default_color: rgba(shadow),
                        custom_glyphs: &[],
                    });
                }
            }
            areas.push(TextArea {
                buffer,
                left: item.left * scale,
                top: item.top * scale,
                scale: 1.0,
                bounds,
                default_color: rgba(item.color),
                custom_glyphs: &[],
            });
        }
        self.text_renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash_cache,
        )?;

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.configure();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("wgpu surface validation error".into());
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("HUD text encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("HUD transparent text pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)?;
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        self.atlas.trim();
        Ok(())
    }
}

impl OverlayRenderer for WgpuTextRenderer {
    fn resize(&mut self, width: u32, height: u32, scale: i32) {
        self.logical_width = width.max(1);
        self.logical_height = height.max(1);
        self.scale = scale.max(1);
        self.configure();
    }

    fn render(&mut self) -> Result<(), Box<dyn Error>> {
        self.render_frame()
    }
}

const OUTLINE_OFFSETS: [(f32, f32); 8] = [
    (-1.0, -1.0),
    (0.0, -1.0),
    (1.0, -1.0),
    (-1.0, 0.0),
    (1.0, 0.0),
    (-1.0, 1.0),
    (0.0, 1.0),
    (1.0, 1.0),
];

fn rgba(value: [u8; 4]) -> Color {
    Color::rgba(value[0], value[1], value[2], value[3])
}

pub fn bundled_font_system() -> FontSystem {
    let mut fonts = FontSystem::new();
    fonts
        .db_mut()
        .load_font_data(include_bytes!("../../assets/fonts/NotoSans-Regular.ttf").to_vec());
    fonts
        .db_mut()
        .load_font_data(include_bytes!("../../assets/fonts/NotoSans-Bold.ttf").to_vec());
    fonts
}
