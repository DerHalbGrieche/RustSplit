//! Draws a complete layout.

use std::path::{Path, PathBuf};

use egui::{
    emath::TSTransform, ColorImage, Rect, TextureHandle, TextureId, TextureOptions, Ui, Vec2,
};
use image::RgbaImage;

use rustsplit_core::Timer;

use crate::canvas::paint_background;
use crate::components::ComponentContext;
use crate::icon::IconCache;
use crate::layout::{BackgroundStyle, Layout, LayoutMode, LayoutSettings};
use crate::layout_xml::EMBEDDED_IMAGE_PREFIX;
use crate::world_record::WorldRecord;

pub struct Renderer {
    pub icons: IconCache,
    /// Last rectangle used for each component, in screen coordinates.  Useful
    /// for editors and debugging.
    pub last_component_rects: Vec<Rect>,
    /// External world record data for the World Record component.
    pub world_record: Option<WorldRecord>,
    background: Option<Background>,
}

/// The decoded layout background image and its blurred texture.
struct Background {
    path: PathBuf,
    /// Decoded source pixels, kept so a blur change does not read and decode
    /// the file again. `None` when the image could not be loaded, which also
    /// stops a broken path from being retried every frame.
    source: Option<RgbaImage>,
    /// The blur baked into `texture`.
    blur: f32,
    texture: Option<TextureHandle>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self {
            icons: IconCache::new(),
            last_component_rects: Vec::new(),
            world_record: None,
            background: None,
        }
    }
}

impl Renderer {
    pub fn natural_height(&self, layout: &Layout, timer: &Timer) -> f32 {
        layout
            .components
            .iter()
            .map(|component| component.component.natural_height(timer, 1.0))
            .sum::<f32>()
            .max(1.0)
    }

    pub fn natural_width(&self, layout: &Layout, timer: &Timer) -> f32 {
        layout
            .components
            .iter()
            .map(|component| component.component.natural_width(timer, 1.0))
            .sum::<f32>()
            .max(1.0)
    }

    /// Load the background texture, decoding only when the path changes and
    /// blurring only when the path or the blur setting changes. Returns the
    /// texture and the source image size; the size is used for the aspect
    /// ratio because large blurs are baked into a downscaled texture.
    fn ensure_background(
        &mut self,
        ctx: &egui::Context,
        path: &PathBuf,
        blur: f32,
    ) -> Option<(TextureId, Vec2)> {
        let blur = effective_blur(blur);
        if self
            .background
            .as_ref()
            .is_none_or(|background| background.path != *path)
        {
            self.background = Some(Background {
                path: path.clone(),
                source: decode_background(path),
                blur,
                texture: None,
            });
        }
        let background = self.background.as_mut()?;
        let source = background.source.as_ref()?;
        if background.texture.is_none() || background.blur != blur {
            background.blur = blur;
            background.texture = Some(ctx.load_texture(
                "rustsplit-background",
                background_pixels(source, blur),
                TextureOptions::LINEAR,
            ));
        }
        let size = egui::vec2(source.width() as f32, source.height() as f32);
        background
            .texture
            .as_ref()
            .map(|texture| (texture.id(), size))
    }

    pub fn draw(&mut self, ui: &mut Ui, layout: &Layout, timer: &Timer) {
        let available = ui.max_rect();
        if available.width() <= 1.0 || available.height() <= 1.0 {
            return;
        }

        // Match LiveSplit's scaling: a vertical layout scales with the window
        // height, and a horizontal layout scales with the window width. The
        // other axis simply fills the window, so resizing the width of a
        // vertical layout does not change the text size.
        let (scale, virtual_width, virtual_height) = match layout.mode {
            LayoutMode::Vertical => {
                let natural_height = self.natural_height(layout, timer).max(1.0);
                let scale = (available.height() / natural_height).clamp(0.05, 16.0);
                (scale, available.width() / scale, available.height() / scale)
            }
            LayoutMode::Horizontal => {
                let natural_width = self.natural_width(layout, timer).max(1.0);
                let scale = (available.width() / natural_width).clamp(0.05, 16.0);
                (scale, available.width() / scale, available.height() / scale)
            }
        };

        // Draw in unscaled layout coordinates and apply one visual transform.
        // This keeps component code simple and produces LiveSplit-style
        // proportional resizing without double-scaling fonts or row heights.
        let transform = TSTransform::new(available.min.to_vec2(), scale);
        let virtual_rect =
            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(virtual_width, virtual_height));

        let transparent =
            layout.settings.transparent_background || !layout.settings.has_background();
        let mut render_theme = layout.settings.clone();
        if transparent {
            apply_readable_text_theme(&mut render_theme);
        }

        let mut root = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(available)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        root.set_opacity(layout.settings.opacity.clamp(0.0, 1.0));
        // The shapes below are drawn in virtual layout coordinates, so the
        // clip rect must use the same coordinate space. `with_visual_transform`
        // scales both the shapes and their clip rects; leaving the screen-sized
        // clip rect here would shrink the clip to `window * scale`.
        root.set_clip_rect(virtual_rect);

        let gradient = layout.settings.background_type.gradient();
        let background_texture = match &layout.settings.background_image {
            Some(path) if gradient.is_none() => {
                self.ensure_background(ui.ctx(), path, layout.settings.image_blur)
            }
            _ => None,
        };
        let image_opacity = layout.settings.image_opacity.clamp(0.0, 1.0);
        let icons = &mut self.icons;
        let last_rects = &mut self.last_component_rects;
        let world_record = self.world_record.as_ref();

        root.with_visual_transform(transform, |ui| {
            match gradient {
                // Layout background. Use the shared background painter so a
                // "Plain" background is a solid color; interpolating to the
                // second color would make opaque LiveSplit layouts fade out.
                Some(gradient) if !transparent => {
                    let background = BackgroundStyle {
                        color: layout.settings.background_color,
                        color2: layout.settings.background_color2,
                        gradient,
                    };
                    paint_background(ui.painter(), virtual_rect, &background);
                }
                Some(_) => {}
                // An image background draws only the image, never the colors.
                //
                // `ImageOpacity` is brightness, not transparency: LiveSplit
                // bakes it into the image's alpha and draws that over its
                // black window, and livesplit-core maps it to brightness for
                // that reason. Painting black first and the image at that
                // alpha on top gives the same result, including black behind
                // transparent pixels of the image.
                //
                // Transparent mode has no black window, so there the image is
                // alpha-blended over whatever is behind the overlay and the
                // setting is a real opacity. That is the point of the mode,
                // and the only way to get a see-through image.
                None => {
                    if !transparent {
                        ui.painter()
                            .rect_filled(virtual_rect, 0.0, egui::Color32::BLACK);
                    }
                    if let Some((texture, image_size)) = background_texture {
                        // Cover the layout and crop the overflow evenly, like
                        // LiveSplit.
                        let image_scale = (virtual_rect.width() / image_size.x)
                            .max(virtual_rect.height() / image_size.y);
                        let size = image_size * image_scale;
                        let rect = Rect::from_center_size(virtual_rect.center(), size);
                        ui.painter().image(
                            texture,
                            rect,
                            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::from_white_alpha((255.0 * image_opacity).round() as u8),
                        );
                    }
                }
            }

            last_rects.clear();

            match layout.mode {
                LayoutMode::Vertical => {
                    let mut y = 0.0;
                    for component in &layout.components {
                        let height = component.component.natural_height(timer, 1.0);
                        if height <= 0.0 {
                            last_rects.push(Rect::NOTHING);
                            continue;
                        }
                        let rect = Rect::from_min_size(
                            egui::pos2(0.0, y),
                            egui::vec2(virtual_width, height),
                        );
                        last_rects.push(Rect::from_min_max(
                            transform * rect.min,
                            transform * rect.max,
                        ));
                        let mut overridden_theme;
                        let theme = if component.font_overrides.has_overrides() {
                            overridden_theme = render_theme.clone();
                            let overrides = &component.font_overrides;
                            if overrides.override_timer_font {
                                overridden_theme.timer_font = overrides.timer_font.clone();
                            }
                            if overrides.override_times_font {
                                overridden_theme.times_font = overrides.times_font.clone();
                            }
                            if overrides.override_text_font {
                                overridden_theme.text_font = overrides.text_font.clone();
                            }
                            &overridden_theme
                        } else {
                            &render_theme
                        };
                        let mut ctx = ComponentContext::new(timer, theme, icons, 1.0);
                        ctx.world_record = world_record;
                        component.component.draw(ui, rect, &mut ctx);
                        y += height;
                    }
                }
                LayoutMode::Horizontal => {
                    let mut x = 0.0;
                    for component in &layout.components {
                        let width = component.component.natural_width(timer, 1.0);
                        if width <= 0.0 {
                            last_rects.push(Rect::NOTHING);
                            continue;
                        }
                        let rect = Rect::from_min_size(
                            egui::pos2(x, 0.0),
                            egui::vec2(width, virtual_height),
                        );
                        last_rects.push(Rect::from_min_max(
                            transform * rect.min,
                            transform * rect.max,
                        ));
                        let mut overridden_theme;
                        let theme = if component.font_overrides.has_overrides() {
                            overridden_theme = render_theme.clone();
                            let overrides = &component.font_overrides;
                            if overrides.override_timer_font {
                                overridden_theme.timer_font = overrides.timer_font.clone();
                            }
                            if overrides.override_times_font {
                                overridden_theme.times_font = overrides.times_font.clone();
                            }
                            if overrides.override_text_font {
                                overridden_theme.text_font = overrides.text_font.clone();
                            }
                            &overridden_theme
                        } else {
                            &render_theme
                        };
                        let mut ctx = ComponentContext::new(timer, theme, icons, 1.0);
                        ctx.world_record = world_record;
                        component.component.draw(ui, rect, &mut ctx);
                        x += width;
                    }
                }
            }
        });
    }
}

/// LiveSplit stores `ImageBlur` as a fraction from 0 to 1 (`.lsl` files and
/// livesplit-core use the same range). As in livesplit-core the blur is
/// relative to the image, so it looks the same at any resolution:
/// `sigma = BLUR_FACTOR * blur * max(width, height)` pixels of gaussian blur.
/// Values outside 0..1 are clamped.
const BLUR_FACTOR: f32 = 0.05;

/// Blurs wider than this sigma (in pixels) are computed on a downscaled copy.
/// A blurred image has no detail left at that scale, so the linear texture
/// filter upscales it without visible loss, and the work stays bounded: the
/// long side is at most `MAX_BLUR_SIGMA / (BLUR_FACTOR * blur)` pixels.
const MAX_BLUR_SIGMA: f32 = 4.0;

/// Upper bound for the long side of a blurred texture, so small blurs on very
/// large images do not blur tens of megapixels.
const MAX_BLURRED_SIZE: f32 = 2048.0;

/// The stored blur clamped to LiveSplit's range. NaN counts as no blur.
fn effective_blur(blur: f32) -> f32 {
    if blur > 0.0 {
        blur.min(1.0)
    } else {
        0.0
    }
}

fn decode_background(path: &Path) -> Option<RgbaImage> {
    let path_text = path.to_string_lossy();
    let bytes = if let Some(encoded) = path_text.strip_prefix(EMBEDDED_IMAGE_PREFIX) {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .ok()?
    } else {
        std::fs::read(path).ok()?
    };
    let image = image::load_from_memory(&bytes)
        .or_else(|error| {
            dotnet_bitmap_data(&bytes)
                .ok_or(error)
                .and_then(image::load_from_memory)
        })
        .ok()?
        .to_rgba8();
    (image.width() > 0 && image.height() > 0).then_some(image)
}

/// The encoded image inside a .NET `BinaryFormatter` serialization of a
/// `System.Drawing.Bitmap`, which is what LiveSplit embeds in `.lsl` files.
/// The bitmap's only member, `Data`, is a byte array holding the image file
/// (usually PNG). Its record is the last one before the message end:
/// `0x0F` (ArraySinglePrimitive), object id, `u32` length, `0x02` (Byte),
/// the bytes, then `0x0B` (MessageEnd). livesplit-core reads the same bytes
/// at a fixed offset; finding the record also copes with other assembly
/// names in the header.
fn dotnet_bitmap_data(blob: &[u8]) -> Option<&[u8]> {
    const RECORD_HEADER: usize = 10;
    let (&0x0B, body) = blob.split_last()? else {
        return None;
    };
    (0..body.len().checked_sub(RECORD_HEADER)?).find_map(|start| {
        let header = &body[start..start + RECORD_HEADER];
        let length = u32::from_le_bytes(header[5..9].try_into().ok()?) as usize;
        let data = &body[start + RECORD_HEADER..];
        (header[0] == 0x0F && header[9] == 0x02 && data.len() == length).then_some(data)
    })
}

/// Build the background texture pixels with the blur applied. This runs once
/// per image or blur change, never per frame.
fn background_pixels(source: &RgbaImage, blur: f32) -> ColorImage {
    let (width, height) = source.dimensions();
    let longest = width.max(height) as f32;
    let sigma = BLUR_FACTOR * effective_blur(blur) * longest;
    // Sub-pixel blurs are invisible; skip the work.
    if sigma < 0.5 {
        return ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            source.as_raw(),
        );
    }

    let scale = (MAX_BLUR_SIGMA / sigma)
        .min(MAX_BLURRED_SIZE / longest)
        .min(1.0);
    let mut working = if scale < 1.0 {
        let scaled = |side: u32| ((side as f32 * scale).round() as u32).max(1);
        image::imageops::thumbnail(source, scaled(width), scaled(height))
    } else {
        source.clone()
    };
    // `fast_blur` expects premultiplied alpha; without it fully transparent
    // pixels would bleed their arbitrary color into the visible ones.
    for pixel in working.pixels_mut() {
        let alpha = pixel[3] as u32;
        for channel in &mut pixel.0[..3] {
            *channel = ((*channel as u32 * alpha + 127) / 255) as u8;
        }
    }
    // Three box blur passes approximate a gaussian in O(pixels), independent
    // of the radius.
    let blurred = image::imageops::fast_blur(&working, sigma * scale);
    ColorImage::from_rgba_premultiplied(
        [blurred.width() as usize, blurred.height() as usize],
        blurred.as_raw(),
    )
}

/// Make text legible over an arbitrary background without painting a
/// background of our own. White or colored text gets a dark outline; very dark
/// text gets a light outline.
fn apply_readable_text_theme(theme: &mut LayoutSettings) {
    if theme.text_outline_color.a() == 0 {
        let [r, g, b, _] = theme.text_color.to_srgba_unmultiplied();
        let luminance = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
        theme.text_outline_color = if luminance < 96.0 {
            egui::Color32::from_white_alpha(220)
        } else {
            egui::Color32::from_black_alpha(220)
        };
    }
    if theme.shadows_color.a() == 0 {
        theme.shadows_color = egui::Color32::from_black_alpha(180);
    }
    theme.drop_shadows = true;
}

/// Layout settings used when the application is in an overlay-only mode.
pub fn transparent_theme() -> LayoutSettings {
    LayoutSettings::default()
}

/// Helper for previews in the layout editor.
pub fn preview_layout_height(layout: &Layout, timer: &Timer, renderer: &Renderer) -> f32 {
    renderer.natural_height(layout, timer).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2, Context, RawInput, Rect};
    use rustsplit_core::{Run, Timer};

    use crate::layout::BackgroundType;

    fn render(
        layout: &Layout,
        timer: &Timer,
        size: egui::Vec2,
    ) -> (Vec<Rect>, Vec<(Rect, egui::Shape)>) {
        let ctx = Context::default();
        let mut renderer = Renderer::default();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
                ..Default::default()
            },
            |ui| renderer.draw(ui, layout, timer),
        );
        output.textures_delta.clear();
        let shapes = output
            .shapes
            .into_iter()
            .map(|shape| (shape.clip_rect, shape.shape))
            .collect();
        (renderer.last_component_rects.clone(), shapes)
    }

    fn timer() -> Timer {
        Timer::with_system_clock(Run::with_segments("", "", ["One", "Two", "Three"]))
    }

    #[test]
    fn vertical_layout_scales_with_height_and_fills_width() {
        let layout = Layout::parse_default().unwrap();
        let timer = timer();
        let (narrow, _) = render(&layout, &timer, vec2(300.0, 500.0));
        let (wide, _) = render(&layout, &timer, vec2(600.0, 500.0));

        assert_eq!(narrow.len(), wide.len());
        for (narrow, wide) in narrow.iter().zip(&wide) {
            assert!((narrow.height() - wide.height()).abs() < 0.01);
            assert!((narrow.min.y - wide.min.y).abs() < 0.01);
            assert!((wide.width() - 2.0 * narrow.width()).abs() < 0.5);
        }
    }

    #[test]
    fn horizontal_layout_scales_with_width_and_fills_height() {
        let mut layout = Layout::parse_default().unwrap();
        layout.mode = LayoutMode::Horizontal;
        let timer = timer();
        let (short, _) = render(&layout, &timer, vec2(600.0, 300.0));
        let (tall, _) = render(&layout, &timer, vec2(600.0, 600.0));

        assert_eq!(short.len(), tall.len());
        for (short, tall) in short.iter().zip(&tall) {
            assert!((short.width() - tall.width()).abs() < 0.01);
            assert!((short.min.x - tall.min.x).abs() < 0.01);
            assert!((tall.height() - 2.0 * short.height()).abs() < 0.5);
        }
    }

    #[test]
    fn plain_layout_background_is_solid() {
        let mut layout = Layout::default();
        layout.settings.background_color = egui::Color32::from_rgb(15, 15, 15);
        layout.settings.background_color2 = egui::Color32::TRANSPARENT;
        layout.settings.background_type = BackgroundType::SolidColor;
        layout.settings.transparent_background = false;
        let timer = timer();
        let (_, shapes) = render(&layout, &timer, vec2(300.0, 500.0));

        let background = shapes.iter().find_map(|(_, shape)| match shape {
            egui::Shape::Rect(rect) if rect.fill == layout.settings.background_color => {
                Some(rect.rect)
            }
            _ => None,
        });
        let background = background.expect("plain background should be drawn as a solid rect");
        assert_eq!(background.size(), vec2(300.0, 500.0));
    }

    #[test]
    fn transparent_mode_hides_the_layout_background() {
        let mut layout = Layout::parse_default().unwrap();
        layout.settings.transparent_background = true;
        let timer = timer();
        let (_, shapes) = render(&layout, &timer, vec2(286.0, 519.0));

        assert!(!shapes.iter().any(|(_, shape)| {
            matches!(
                shape,
                egui::Shape::Rect(rect) if rect.fill == layout.settings.background_color
            )
        }));
    }

    #[test]
    fn transparent_mode_adds_readable_text_contrast() {
        let mut settings = LayoutSettings {
            text_outline_color: egui::Color32::TRANSPARENT,
            shadows_color: egui::Color32::TRANSPARENT,
            drop_shadows: false,
            ..LayoutSettings::default()
        };

        apply_readable_text_theme(&mut settings);

        assert!(settings.text_outline_color.a() > 0);
        assert!(settings.shadows_color.a() > 0);
        assert!(settings.drop_shadows);
    }

    #[test]
    fn scaled_layout_is_not_clipped_to_the_unscaled_window_rect() {
        let layout = Layout::parse_default().unwrap();
        let timer = timer();
        let size = vec2(286.0, 519.0);
        let (_, shapes) = render(&layout, &timer, size);

        let (clip_rect, _) = shapes
            .iter()
            .find(|(_, shape)| {
                matches!(
                    shape,
                    egui::Shape::Rect(rect) if rect.fill == layout.settings.background_color
                )
            })
            .expect("the layout background should be drawn");

        assert_eq!(clip_rect.size(), size);
    }

    /// A black image with a one pixel white vertical line in the middle.
    fn line_image(size: u32) -> RgbaImage {
        RgbaImage::from_fn(size, size, |x, _| {
            if x == size / 2 {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 255])
            }
        })
    }

    fn red_at(image: &ColorImage, x: usize, y: usize) -> u8 {
        image.pixels[y * image.size[0] + x].r()
    }

    #[test]
    fn zero_blur_keeps_the_image_unchanged() {
        let source = RgbaImage::from_fn(8, 4, |x, y| {
            image::Rgba([x as u8 * 30, y as u8 * 60, 7, 255])
        });
        for blur in [0.0, -1.0, f32::NAN] {
            let pixels = background_pixels(&source, blur);
            assert_eq!(pixels.size, [8, 4]);
            assert_eq!(pixels.pixels[9], egui::Color32::from_rgb(30, 60, 7));
        }
    }

    #[test]
    fn blur_is_relative_to_the_image_size() {
        // sigma = 0.05 * 0.4 * 100 = 2 px, small enough to blur at full size.
        let small = background_pixels(&line_image(100), 0.4);
        assert_eq!(small.size, [100, 100]);
        assert!(red_at(&small, 50, 50) < 255);
        assert!(red_at(&small, 52, 50) > 0);
        assert_eq!(red_at(&small, 70, 50), 0);

        // The same setting on an image twice as large spreads twice as far.
        let large = background_pixels(&line_image(200), 0.4);
        assert!(red_at(&large, 104, 100) > red_at(&small, 54, 50));
        assert!(red_at(&large, 100, 100) < red_at(&small, 50, 50));
    }

    #[test]
    fn large_blurs_are_computed_on_a_downscaled_copy() {
        let color = image::Rgba([10, 20, 30, 255]);
        let source = RgbaImage::from_pixel(4000, 2000, color);
        // sigma = 0.05 * 1.0 * 4000 = 200 px, so the image is scaled by
        // MAX_BLUR_SIGMA / 200 before blurring.
        let pixels = background_pixels(&source, 1.0);
        assert_eq!(pixels.size, [80, 40]);
        // Edges are clamped, so a uniform image stays uniform.
        assert!(pixels.pixels.iter().all(|pixel| {
            pixel
                .to_array()
                .iter()
                .zip([10u8, 20, 30, 255])
                .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
        }));

        // Small blurs on huge images are capped as well.
        let pixels = background_pixels(&RgbaImage::new(8192, 1024), 0.01);
        assert_eq!(pixels.size, [2048, 256]);
    }

    #[test]
    fn blur_does_not_bleed_transparent_pixels() {
        // Left half: invisible red. Right half: opaque blue.
        let source = RgbaImage::from_fn(64, 8, |x, _| {
            if x < 32 {
                image::Rgba([255, 0, 0, 0])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let pixels = background_pixels(&source, 0.5);
        assert!(pixels.pixels.iter().all(|pixel| pixel.r() == 0));
        assert!(pixels.pixels[31].a() > 0, "blue should blur into the gap");
    }

    #[test]
    fn background_texture_is_built_once_per_blur_setting() {
        let mut png = Vec::new();
        line_image(64)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);

        let mut layout = Layout::default();
        layout.settings.background_type = BackgroundType::Image;
        layout.settings.background_image =
            Some(PathBuf::from(format!("{EMBEDDED_IMAGE_PREFIX}{encoded}")));
        layout.settings.transparent_background = false;
        let timer = timer();
        let ctx = Context::default();
        let mut renderer = Renderer::default();
        // Count uploads of textures other than egui's font atlas.
        let mut uploads = |layout: &Layout| {
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 300.0))),
                    ..Default::default()
                },
                |ui| renderer.draw(ui, layout, &timer),
            );
            let count = output
                .textures_delta
                .set
                .iter()
                .filter(|(id, _)| **id != egui::TextureId::default())
                .filter(|(_, deltas)| deltas.iter().any(|delta| delta.pos.is_none()))
                .count();
            output.textures_delta.clear();
            count
        };

        assert_eq!(uploads(&layout), 1);
        assert_eq!(uploads(&layout), 0, "an unchanged blur must not re-blur");
        layout.settings.image_blur = 0.5;
        assert_eq!(uploads(&layout), 1, "a new blur must rebuild the texture");
        assert_eq!(uploads(&layout), 0);
    }

    /// What .NET's `BinaryFormatter` writes for a `System.Drawing.Bitmap`
    /// holding `data`, as LiveSplit stores it in `<BackgroundImage>`.
    fn dotnet_bitmap(data: &[u8]) -> Vec<u8> {
        fn string(out: &mut Vec<u8>, text: &str) {
            out.push(text.len() as u8);
            out.extend_from_slice(text.as_bytes());
        }
        // SerializationHeader: root id 1, header id -1, version 1.0.
        let mut out = vec![
            0x00, 1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 1, 0, 0, 0, 0, 0, 0, 0,
        ];
        // BinaryLibrary 2.
        out.extend_from_slice(&[0x0C, 2, 0, 0, 0]);
        string(
            &mut out,
            "System.Drawing, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b03f5f7f11d50a3a",
        );
        // ClassWithMembersAndTypes 1: Bitmap { Data: byte[] } from library 2.
        out.extend_from_slice(&[0x05, 1, 0, 0, 0]);
        string(&mut out, "System.Drawing.Bitmap");
        out.extend_from_slice(&[1, 0, 0, 0]);
        string(&mut out, "Data");
        out.extend_from_slice(&[0x07, 0x02, 2, 0, 0, 0]);
        // MemberReference to object 3, then object 3 itself.
        out.extend_from_slice(&[0x09, 3, 0, 0, 0]);
        out.extend_from_slice(&[0x0F, 3, 0, 0, 0]);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.push(0x02);
        out.extend_from_slice(data);
        // MessageEnd.
        out.push(0x0B);
        out
    }

    #[test]
    fn livesplit_bitmap_blobs_are_decoded() {
        let mut png = Vec::new();
        line_image(16)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let blob = dotnet_bitmap(&png);
        // livesplit-core reads the image at this fixed offset.
        assert_eq!(&blob[161..blob.len() - 1], png.as_slice());
        assert_eq!(dotnet_bitmap_data(&blob), Some(png.as_slice()));
        assert_eq!(dotnet_bitmap_data(&png), None);
        assert_eq!(dotnet_bitmap_data(&[0x0B]), None);

        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&blob);
        let path = PathBuf::from(format!("{EMBEDDED_IMAGE_PREFIX}{encoded}"));
        let image = decode_background(&path).expect("the embedded bitmap must decode");
        assert_eq!(image.dimensions(), (16, 16));
    }
}
