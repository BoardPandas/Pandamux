use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};
use wgpu::{
    BackendOptions, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, DeviceDescriptor,
    Dx12BackendOptions, Extent3d, InstanceDescriptor, InstanceFlags, LoadOp, MapMode, MemoryHints,
    MultisampleState, Operations, Origin3d, PollType, RenderPassColorAttachment,
    RenderPassDescriptor, StoreOp, TexelCopyBufferInfo, TexelCopyBufferLayout,
    TexelCopyTextureInfo, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureViewDescriptor,
};
use wgpu_glyphon as wgpu;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 360;
const FRAMES: usize = 120;

#[derive(Debug)]
pub struct GpuRenderReport {
    pub adapter: String,
    pub backend: String,
    pub layout_lines: usize,
    pub glyphs: usize,
    pub nonblack_pixels: usize,
    pub first_frame_ms: u128,
    pub avg_frame_ms: f64,
    pub avg_dynamic_frame_ms: f64,
    pub max_frame_ms: u128,
    pub fps_estimate: f64,
    pub artifact_path: Option<PathBuf>,
    pub visual_checks: Vec<VisualRowCheck>,
}

#[derive(Debug, Clone)]
pub struct VisualRowCheck {
    pub label: &'static str,
    pub nonblack_pixels: usize,
    pub passed: bool,
}

impl GpuRenderReport {
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "PANDAMUX_GPU_RENDER_SMOKE_OK\nadapter={}\nbackend={}\nlayout_lines={}\nglyphs={}\nnonblack_pixels={}\nfirst_frame_ms={}\navg_frame_ms={:.3}\navg_dynamic_frame_ms={:.3}\nmax_frame_ms={}\nfps_estimate={:.1}",
            self.adapter,
            self.backend,
            self.layout_lines,
            self.glyphs,
            self.nonblack_pixels,
            self.first_frame_ms,
            self.avg_frame_ms,
            self.avg_dynamic_frame_ms,
            self.max_frame_ms,
            self.fps_estimate
        );

        if let Some(path) = &self.artifact_path {
            summary.push_str(&format!("\nvisual_artifact={}", path.display()));
        }

        for check in &self.visual_checks {
            summary.push_str(&format!(
                "\nvisual_check_{}={} nonblack_pixels={}",
                check.label,
                if check.passed { "pass" } else { "fail" },
                check.nonblack_pixels
            ));
        }

        summary
    }
}

pub fn run_gpu_render_smoke() -> Result<GpuRenderReport> {
    pollster::block_on(run_gpu_render_smoke_async(None))
}

pub fn run_visual_qa_smoke(output_path: impl Into<PathBuf>) -> Result<GpuRenderReport> {
    pollster::block_on(run_gpu_render_smoke_async(Some(output_path.into())))
}

async fn run_gpu_render_smoke_async(output_path: Option<PathBuf>) -> Result<GpuRenderReport> {
    let instance = wgpu::Instance::new(InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: InstanceFlags::empty(),
        memory_budget_thresholds: Default::default(),
        backend_options: BackendOptions {
            dx12: Dx12BackendOptions {
                shader_compiler: wgpu::Dx12Compiler::Fxc,
                ..Default::default()
            },
            ..Default::default()
        },
        display: None,
    });

    let adapter = wgpu::util::initialize_adapter_from_env_or_default(&instance, None)
        .await
        .map_err(|error| anyhow!("no usable wgpu adapter found: {error}"))?;
    let adapter_info = adapter.get_info();
    let (device, queue) = adapter
        .request_device(&DeviceDescriptor {
            label: Some("PandaMUX Phase 2 GPU Render Smoke Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: MemoryHints::Performance,
            ..Default::default()
        })
        .await?;

    let format = TextureFormat::Bgra8Unorm;
    let mut font_system = FontSystem::new();
    let mut swash_cache = SwashCache::new();
    let cache = Cache::new(&device);
    let mut viewport = Viewport::new(&device, &cache);
    let mut atlas = TextAtlas::with_color_mode(&device, &queue, &cache, format, ColorMode::Web);
    let mut text_renderer =
        TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
    let mut input_buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
    buffer.set_size(
        &mut font_system,
        Some((WIDTH - 32) as f32),
        Some((HEIGHT - 32) as f32),
    );
    buffer.set_text(
        &mut font_system,
        terminal_sample_text().as_str(),
        &Attrs::new()
            .family(Family::Monospace)
            .weight(Weight::NORMAL),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, false);
    input_buffer.set_size(&mut font_system, Some((WIDTH - 32) as f32), Some(32.0));
    input_buffer.set_text(
        &mut font_system,
        "input: ",
        &Attrs::new()
            .family(Family::Monospace)
            .weight(Weight::NORMAL),
        Shaping::Advanced,
        None,
    );
    input_buffer.shape_until_scroll(&mut font_system, false);
    let (layout_lines, glyphs) = layout_counts(&buffer);
    if layout_lines < 8 || glyphs < 80 {
        bail!("Unicode sample shaped too little text: lines={layout_lines}, glyphs={glyphs}");
    }

    viewport.update(
        &queue,
        Resolution {
            width: WIDTH,
            height: HEIGHT,
        },
    );

    let texture = device.create_texture(&TextureDescriptor {
        label: Some("PandaMUX Phase 2 offscreen render target"),
        size: Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());

    let mut frame_times = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let started = Instant::now();
        render_frame(
            &device,
            &queue,
            &mut font_system,
            &mut swash_cache,
            &mut viewport,
            &mut atlas,
            &mut text_renderer,
            &buffer,
            None,
            &view,
        )?;
        device.poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })?;
        frame_times.push(started.elapsed());
    }

    let mut dynamic_frame_times = Vec::with_capacity(60);
    for frame in 0..60 {
        let started = Instant::now();
        input_buffer.set_text(
            &mut font_system,
            &format!("input: cargo test --workspace --frame={frame}"),
            &Attrs::new()
                .family(Family::Monospace)
                .weight(Weight::NORMAL),
            Shaping::Advanced,
            None,
        );
        input_buffer.shape_until_scroll(&mut font_system, false);
        render_frame(
            &device,
            &queue,
            &mut font_system,
            &mut swash_cache,
            &mut viewport,
            &mut atlas,
            &mut text_renderer,
            &buffer,
            Some(&input_buffer),
            &view,
        )?;
        device.poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })?;
        dynamic_frame_times.push(started.elapsed());
    }

    let pixels = read_texture_pixels(&device, &queue, &texture)?;
    let nonblack_pixels = count_nonblack_pixels(&pixels);
    if nonblack_pixels < 100 {
        bail!("glyphon/wgpu render produced too few nonblack pixels: {nonblack_pixels}");
    }

    let visual_checks = visual_row_checks(&pixels);
    if let Some(failed) = visual_checks.iter().find(|check| !check.passed) {
        bail!(
            "visual QA row rendered too few pixels: {}={}",
            failed.label,
            failed.nonblack_pixels
        );
    }

    if let Some(path) = &output_path {
        write_bmp(path, WIDTH, HEIGHT, &pixels)?;
    }

    let total_ms: f64 = frame_times
        .iter()
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .sum();
    let avg_frame_ms = total_ms / frame_times.len() as f64;
    let dynamic_total_ms: f64 = dynamic_frame_times
        .iter()
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .sum();
    let avg_dynamic_frame_ms = dynamic_total_ms / dynamic_frame_times.len() as f64;
    let max_frame_ms = frame_times
        .iter()
        .chain(dynamic_frame_times.iter())
        .map(|elapsed| elapsed.as_millis())
        .max()
        .unwrap_or_default();

    Ok(GpuRenderReport {
        adapter: adapter_info.name,
        backend: format!("{:?}", adapter_info.backend),
        layout_lines,
        glyphs,
        nonblack_pixels,
        first_frame_ms: frame_times[0].as_millis(),
        avg_frame_ms,
        avg_dynamic_frame_ms,
        max_frame_ms,
        fps_estimate: 1000.0 / avg_frame_ms.max(0.001),
        artifact_path: output_path,
        visual_checks,
    })
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    viewport: &mut Viewport,
    atlas: &mut TextAtlas,
    text_renderer: &mut TextRenderer,
    buffer: &Buffer,
    input_buffer: Option<&Buffer>,
    view: &wgpu::TextureView,
) -> Result<()> {
    let mut text_areas = vec![TextArea {
        buffer,
        left: 16.0,
        top: 16.0,
        scale: 1.0,
        bounds: TextBounds {
            left: 0,
            top: 0,
            right: WIDTH as i32,
            bottom: (HEIGHT - 48) as i32,
        },
        default_color: Color::rgb(230, 238, 248),
        custom_glyphs: &[],
    }];

    if let Some(input_buffer) = input_buffer {
        text_areas.push(TextArea {
            buffer: input_buffer,
            left: 16.0,
            top: (HEIGHT - 34) as f32,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: (HEIGHT - 48) as i32,
                right: WIDTH as i32,
                bottom: HEIGHT as i32,
            },
            default_color: Color::rgb(150, 220, 170),
            custom_glyphs: &[],
        });
    }

    text_renderer.prepare(
        device,
        queue,
        font_system,
        atlas,
        viewport,
        text_areas,
        swash_cache,
    )?;

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("PandaMUX Phase 2 GPU render smoke encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("PandaMUX Phase 2 GPU render smoke pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(wgpu::Color::BLACK),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        text_renderer.render(atlas, viewport, &mut pass)?;
    }

    queue.submit(Some(encoder.finish()));
    atlas.trim();
    Ok(())
}

fn read_texture_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<u8>> {
    let bytes_per_pixel = 4;
    let unpadded_bytes_per_row = WIDTH * bytes_per_pixel;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
    let buffer_size = padded_bytes_per_row as u64 * HEIGHT as u64;
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("PandaMUX Phase 2 render readback"),
        size: buffer_size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("PandaMUX Phase 2 readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(HEIGHT),
            },
        },
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let (tx, rx) = mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device.poll(PollType::Wait {
        submission_index: None,
        timeout: None,
    })?;
    rx.recv_timeout(Duration::from_secs(10))??;

    let mapped = readback.slice(..).get_mapped_range().to_vec();
    let mut compact = Vec::with_capacity((WIDTH * HEIGHT * bytes_per_pixel) as usize);
    for row in 0..HEIGHT as usize {
        let start = row * padded_bytes_per_row as usize;
        let end = start + unpadded_bytes_per_row as usize;
        compact.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    readback.unmap();

    Ok(compact)
}

fn count_nonblack_pixels(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[0] > 8 || pixel[1] > 8 || pixel[2] > 8)
        .count()
}

fn visual_row_checks(pixels: &[u8]) -> Vec<VisualRowCheck> {
    [
        ("box", 2),
        ("wide", 3),
        ("emoji", 4),
        ("combining", 5),
        ("powerline", 6),
        ("rtl", 7),
        ("ligature", 8),
    ]
    .into_iter()
    .map(|(label, row)| {
        let nonblack_pixels = count_row_band_nonblack_pixels(pixels, row);
        VisualRowCheck {
            label,
            nonblack_pixels,
            passed: nonblack_pixels > 140,
        }
    })
    .collect()
}

fn count_row_band_nonblack_pixels(pixels: &[u8], row: usize) -> usize {
    let top = 16usize.saturating_add(row * 22).saturating_sub(2);
    let bottom = (top + 24).min(HEIGHT as usize);

    (top..bottom)
        .flat_map(|y| {
            let start = y * WIDTH as usize * 4;
            let end = start + WIDTH as usize * 4;
            pixels[start..end].chunks_exact(4)
        })
        .filter(|pixel| pixel[0] > 8 || pixel[1] > 8 || pixel[2] > 8)
        .count()
}

fn write_bmp(path: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    let row_stride = (width * 4) as usize;
    let image_size = row_stride * height as usize;
    let file_size = 14 + 40 + image_size;
    let mut writer = BufWriter::new(File::create(path)?);

    writer.write_all(b"BM")?;
    writer.write_all(&(file_size as u32).to_le_bytes())?;
    writer.write_all(&[0, 0, 0, 0])?;
    writer.write_all(&(54u32).to_le_bytes())?;
    writer.write_all(&(40u32).to_le_bytes())?;
    writer.write_all(&(width as i32).to_le_bytes())?;
    writer.write_all(&(-(height as i32)).to_le_bytes())?;
    writer.write_all(&(1u16).to_le_bytes())?;
    writer.write_all(&(32u16).to_le_bytes())?;
    writer.write_all(&(0u32).to_le_bytes())?;
    writer.write_all(&(image_size as u32).to_le_bytes())?;
    writer.write_all(&(2835i32).to_le_bytes())?;
    writer.write_all(&(2835i32).to_le_bytes())?;
    writer.write_all(&(0u32).to_le_bytes())?;
    writer.write_all(&(0u32).to_le_bytes())?;
    writer.write_all(pixels)?;
    writer.flush()?;

    Ok(())
}

fn layout_counts(buffer: &Buffer) -> (usize, usize) {
    let mut lines = 0;
    let mut glyphs = 0;
    for run in buffer.layout_runs() {
        lines += 1;
        glyphs += run.glyphs.len();
    }
    (lines, glyphs)
}

fn terminal_sample_text() -> String {
    [
        "PandaMUX Phase 2 glyphon/wgpu render smoke",
        "ascii: abcdefghijklmnopqrstuvwxyz 0123456789",
        "box: ┌──────┬──────┐ │ pane │ diff │ └──────┴──────┘",
        "wide: 猫 日本語 한글",
        "emoji: 🚀 ✅ ⚙️",
        "combining: cafe\u{0301} naive\u{0308}",
        "powerline:    ",
        "rtl: مرحبا بالعالم",
        "ligature text: != == >= <= -> <- =>",
    ]
    .join("\n")
}
