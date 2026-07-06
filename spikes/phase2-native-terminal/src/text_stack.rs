use anyhow::{Result, anyhow};
use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};

#[derive(Debug)]
pub struct TextStackReport {
    pub layout_lines: usize,
    pub glyphs: usize,
    pub glyphon_renderer: &'static str,
    pub glyphon_atlas: &'static str,
    pub swash_font_ref: &'static str,
    pub wgpu_format: wgpu::TextureFormat,
    pub russh_config: &'static str,
}

impl TextStackReport {
    pub fn summary(&self) -> String {
        format!(
            "text_stack_ok lines={} glyphs={} format={:?}\nglyphon_renderer={}\nglyphon_atlas={}\nswash_font_ref={}\nrussh_config={}",
            self.layout_lines,
            self.glyphs,
            self.wgpu_format,
            self.glyphon_renderer,
            self.glyphon_atlas,
            self.swash_font_ref,
            self.russh_config
        )
    }
}

pub fn run_text_stack_smoke() -> Result<TextStackReport> {
    let mut font_system = FontSystem::new();
    let mut buffer = Buffer::new(&mut font_system, Metrics::new(14.0, 20.0));

    buffer.set_size(Some(640.0), Some(240.0));
    buffer.set_text(
        "ascii | box \u{2500} | CJK \u{732b} | emoji \u{1f680} | rtl \u{0645}\u{0631}\u{062d}\u{0628}\u{0627}",
        &Attrs::new(),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut font_system, true);

    let mut layout_lines = 0;
    let mut glyphs = 0;
    for run in buffer.layout_runs() {
        layout_lines += 1;
        glyphs += run.glyphs.len();
    }

    if layout_lines == 0 || glyphs == 0 {
        return Err(anyhow!("cosmic-text produced an empty layout"));
    }

    Ok(TextStackReport {
        layout_lines,
        glyphs,
        glyphon_renderer: std::any::type_name::<glyphon::TextRenderer>(),
        glyphon_atlas: std::any::type_name::<glyphon::TextAtlas>(),
        swash_font_ref: std::any::type_name::<swash::FontRef<'static>>(),
        wgpu_format: wgpu::TextureFormat::Bgra8UnormSrgb,
        russh_config: std::any::type_name::<russh::client::Config>(),
    })
}
