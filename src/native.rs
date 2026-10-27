//! Pixel-addressed ASCII tiles on Wayland. The simulation and rasterizer are shared with ANSI;
//! only presentation changes. Glyphs are rasterized once at the output's real pixel density, then
//! copied 1:1, never enlarged from a terminal-sized bitmap.

use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};
use sdl3::event::{Event, WindowEvent};
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{BlendMode, FRect, ScaleMode, Texture, TextureCreator, WindowCanvas};
use sdl3::video::WindowContext;

use koi_screensaver::render::{unpack, PondRenderer, DARK};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct Native {
    canvas: WindowCanvas,
    events: sdl3::EventPump,
    _sdl: sdl3::Sdl,
}

impl Native {
    pub fn new(width: u32, height: u32, monitor: Option<&str>) -> Result<Self> {
        sdl3::hint::set("SDL_APP_ID", crate::SCREENSAVER_CLASS);
        sdl3::hint::set("SDL_VIDEO_DRIVER", "wayland");
        sdl3::hint::set("SDL_VIDEO_ALLOW_SCREENSAVER", "1");
        sdl3::hint::set("SDL_MOUSE_FOCUS_CLICKTHROUGH", "1");
        if let Some(monitor) = monitor {
            sdl3::hint::set("SDL_VIDEO_DISPLAY_PRIORITY", monitor);
        }
        let sdl = sdl3::init()?;
        let video = sdl.video()?;
        let display = video.get_primary_display()?;
        let position = sdl3::sys::video::SDL_WINDOWPOS_CENTERED_DISPLAY(display.to_ll());
        let window = video
            .window("Koi pond", width, height)
            .position(position, position)
            .high_pixel_density()
            .fullscreen()
            .build()?;
        let canvas = window.into_canvas();
        sdl.mouse().show_cursor(false);
        Ok(Self {
            canvas,
            events: sdl.event_pump()?,
            _sdl: sdl,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.canvas.window().size()
    }

    pub fn pixel_size(&self) -> (u32, u32) {
        self.canvas.window().size_in_pixels()
    }

    pub fn texture_creator(&self) -> TextureCreator<WindowContext> {
        self.canvas.texture_creator()
    }

    pub fn quit_requested(&mut self) -> bool {
        self.events.poll_iter().any(|e| match e {
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none(),
            e => matches!(
                e,
                Event::Quit { .. }
                    | Event::KeyDown { .. }
                    | Event::MouseButtonDown { .. }
                    | Event::Window {
                        win_event: WindowEvent::CloseRequested,
                        ..
                    }
            ),
        })
    }

    pub fn paint(&mut self, pond: &PondRenderer, atlas: &mut GlyphAtlas<'_>) -> Result<()> {
        let (width, height) = self.size();
        let (pw, ph) = self.pixel_size();
        let scale = (pw as f64 / width as f64, ph as f64 / height as f64);
        let (cw, ch) = pond.cell_size();
        atlas.resize(cw, ch, scale)?;
        let [r, g, b] = DARK.bg;
        self.canvas.set_draw_color(Color::RGB(r, g, b));
        self.canvas.clear();

        let frame = &pond.frame;
        for i in 0..frame.glyph.len() {
            let x = (i % frame.cols) as f64 * cw;
            let y = (i / frame.cols) as f64 * ch;
            if frame.bg[i] != koi_screensaver::term::DEFAULT_BG {
                let [r, g, b] = unpack(frame.bg[i]);
                self.canvas.set_draw_color(Color::RGB(r, g, b));
                // The website leaves a one-logical-pixel water gutter between every skin tile.
                // Snap to physical pixels at fractional desktop scales instead of filtering a bitmap.
                let left = ((x + 1.0) * scale.0).round();
                let top = ((y + 1.0) * scale.1).round();
                self.canvas.fill_rect(FRect::new(
                    left as f32,
                    top as f32,
                    (((x + cw) * scale.0).round() - left) as f32,
                    (((y + ch) * scale.1).round() - top) as f32,
                ))?;
            }
            let code = frame.glyph[i];
            if code == b' ' {
                continue;
            }
            let [r, g, b] = unpack(frame.fg[i]);
            atlas.texture.set_color_mod(r, g, b);
            let cell = (code - 32) as u32;
            let src = FRect::new(
                ((cell % 16) * atlas.width) as f32,
                ((cell / 16) * atlas.height) as f32,
                atlas.width as f32,
                atlas.height as f32,
            );
            let dst = FRect::new(
                ((x + cw / 2.0) * scale.0 - atlas.width as f64 / 2.0).round() as f32,
                ((y + ch / 2.0) * scale.1 - atlas.height as f64 / 2.0).round() as f32,
                atlas.width as f32,
                atlas.height as f32,
            );
            self.canvas.copy(&atlas.texture, src, dst)?;
        }
        self.canvas.present();
        Ok(())
    }
}

/// A font atlas is tied to its texture creator, not to a borrowed window. Rebuilt only on a cell
/// size or display-density change, so no font rasterization or image upload happens per frame.
pub struct GlyphAtlas<'a> {
    creator: &'a TextureCreator<WindowContext>,
    texture: Texture<'a>,
    width: u32,
    height: u32,
    geometry: (f64, f64, f64, f64),
}

impl<'a> GlyphAtlas<'a> {
    pub fn new(creator: &'a TextureCreator<WindowContext>) -> Result<Self> {
        Ok(Self {
            creator,
            texture: creator.create_texture_static(PixelFormat::ARGB8888, 1, 1)?,
            width: 1,
            height: 1,
            geometry: (0.0, 0.0, 0.0, 0.0),
        })
    }

    fn resize(&mut self, cw: f64, ch: f64, scale: (f64, f64)) -> Result<()> {
        let geometry = (cw, ch, scale.0, scale.1);
        if self.geometry == geometry {
            return Ok(());
        }
        self.width = (cw * scale.0).ceil() as u32;
        self.height = (ch * scale.1).ceil() as u32;
        let mut surface = ImageSurface::create(
            Format::ARgb32,
            (self.width * 16) as i32,
            (self.height * 6) as i32,
        )?;
        let ctx = Context::new(&surface)?;
        ctx.select_font_face("monospace", FontSlant::Normal, FontWeight::Bold);
        ctx.set_font_size((ch * 0.72).round() * scale.1);
        ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let font = ctx.font_extents()?;
        for code in 33u8..127 {
            let text = (code as char).to_string();
            let extent = ctx.text_extents(&text)?;
            let cell = (code - 32) as u32;
            let x = ((cell % 16) * self.width) as f64;
            let y = ((cell / 16) * self.height) as f64;
            ctx.move_to(
                x + (self.width as f64 - extent.x_advance()) / 2.0,
                y + self.height as f64 / 2.0 + (font.ascent() - font.descent()) / 2.0 + scale.1,
            );
            ctx.show_text(&text)?;
        }
        drop(ctx);
        surface.flush();
        let stride = surface.stride() as usize;
        let mut texture = self.creator.create_texture_static(
            PixelFormat::ARGB8888,
            self.width * 16,
            self.height * 6,
        )?;
        let mut pixels = surface.data()?;
        // Cairo stores premultiplied white; SDL's standard texture blend expects straight alpha.
        // Leaving the RGB channels premultiplied would apply coverage twice and thin the glyphs.
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[..3].fill(255);
        }
        texture.update(None, &pixels, stride)?;
        texture.set_blend_mode(BlendMode::Blend);
        texture.set_scale_mode(ScaleMode::Nearest);
        self.texture = texture;
        self.geometry = geometry;
        Ok(())
    }
}
