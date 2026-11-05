//! Draws a complete layout.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

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
    /// The layout's image file or embedded image. Embedded images have no
    /// file and are matched by content, which is a pointer comparison as
    /// long as the layout keeps the same `Arc`.
    image: Arc<Path>,
    /// The file's modification time and length when it was read, to notice
    /// when it is replaced. `None` for embedded images and missing files.
    stamp: Option<FileStamp>,
    /// When `stamp` was last compared with the file, in egui time (seconds).
    checked_at: f64,
    /// Decoded source pixels, kept so a blur change does not read and decode
    /// the file again. `None` when the image could not be loaded.
    source: Option<RgbaImage>,
    /// Decodes that failed in a row, for the retry backoff.
    failures: u32,
    /// When to decode a broken image file again.
    retry_at: f64,
    /// The blur baked into `texture`.
    blur: f32,
    texture: Option<TextureHandle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// How often, in seconds, an image file is checked for changes.
const FILE_CHECK_INTERVAL: f64 = 1.0;

/// The first and the longest wait, in seconds, before decoding an image file
/// that failed to load again. The wait doubles after every failure.
const FIRST_RETRY_DELAY: f64 = 1.0;
const MAX_RETRY_DELAY: f64 = 60.0;

/// Upper bound for the long side of the decoded image and of its texture, so
/// a huge image neither keeps hundreds of megabytes around nor exceeds what
/// the GPU accepts.
const MAX_BACKGROUND_SIDE: u32 = 4096;

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

    /// Load the background texture, decoding only when the image or its file
    /// changes and blurring only when the image or the blur setting changes.
    /// Returns the texture and the source image size; the size is used for
    /// the aspect ratio because large blurs are baked into a downscaled
    /// texture.
    fn ensure_background(
        &mut self,
        ctx: &egui::Context,
        image: &Arc<Path>,
        blur: f32,
    ) -> Option<(TextureId, Vec2)> {
        let blur = effective_blur(blur);
        let (now, max_texture_side) = ctx.input(|input| (input.time, input.max_texture_side));
        let embedded = is_embedded(image);
        // `Some(failures so far)` when the image has to be decoded.
        let reload = match &mut self.background {
            Some(background)
                if Arc::ptr_eq(&background.image, image)
                    || background.image.as_os_str() == image.as_os_str() =>
            {
                // A reloaded layout has a new `Arc` for the same image; keep
                // it so the next frames compare pointers only.
                if !Arc::ptr_eq(&background.image, image) {
                    background.image = Arc::clone(image);
                }
                // Files can be replaced, so check them now and then, and give
                // a broken one another chance after a while.
                if !embedded && now - background.checked_at >= FILE_CHECK_INTERVAL {
                    background.checked_at = now;
                    let changed = FileStamp::of(image) != background.stamp;
                    (changed || now >= background.retry_at).then_some(background.failures)
                } else {
                    None
                }
            }
            _ => Some(0),
        };
        if let Some(failures) = reload {
            // Read the stamp first: a change during the decode is then
            // noticed by the next check.
            let stamp = if embedded { None } else { FileStamp::of(image) };
            let source = decode_background(image)
                .map(|source| fit_within(&source, MAX_BACKGROUND_SIDE).unwrap_or(source));
            let failures = if source.is_some() { 0 } else { failures + 1 };
            // An embedded image cannot change, so retrying it is pointless.
            let retry_at = if source.is_some() || embedded {
                f64::INFINITY
            } else {
                now + (FIRST_RETRY_DELAY * 2f64.powi(failures as i32 - 1)).min(MAX_RETRY_DELAY)
            };
            self.background = Some(Background {
                image: Arc::clone(image),
                stamp,
                checked_at: now,
                source,
                failures,
                retry_at,
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
                background_pixels(source, blur, max_texture_side),
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
            Some(image) if gradient.is_none() => {
                self.ensure_background(ui.ctx(), image, layout.settings.image_blur)
            }
            _ => {
                // Free the decoded image and its texture.
                self.background = None;
                None
            }
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

/// Runs every frame, so it looks at the prefix only; an embedded image can be
/// megabytes long.
fn is_embedded(image: &Path) -> bool {
    image
        .as_os_str()
        .as_encoded_bytes()
        .starts_with(EMBEDDED_IMAGE_PREFIX.as_bytes())
}

/// `image` shrunk to at most `max_side` pixels on its long side, keeping the
/// aspect ratio, or `None` when it already fits.
fn fit_within(image: &RgbaImage, max_side: u32) -> Option<RgbaImage> {
    let longest = image.width().max(image.height());
    (longest > max_side).then(|| downscale(image, max_side as f32 / longest as f32, max_side))
}

/// Scales `image` by `scale` (below 1) with an area-averaging filter, so no
/// detail aliases. No side ends up longer than `max_side` or shorter than 1.
fn downscale(image: &RgbaImage, scale: f32, max_side: u32) -> RgbaImage {
    let scaled = |side: u32| ((side as f32 * scale).round() as u32).clamp(1, max_side.max(1));
    image::imageops::thumbnail(image, scaled(image.width()), scaled(image.height()))
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
/// per image or blur change, never per frame. The long side never exceeds
/// `max_texture_side` (the GPU limit, which egui asserts) or
/// [`MAX_BACKGROUND_SIDE`].
fn background_pixels(source: &RgbaImage, blur: f32, max_texture_side: usize) -> ColorImage {
    let max_side = MAX_BACKGROUND_SIDE.min(max_texture_side.try_into().unwrap_or(u32::MAX));
    let (width, height) = source.dimensions();
    let longest = width.max(height) as f32;
    let sigma = BLUR_FACTOR * effective_blur(blur) * longest;
    // Sub-pixel blurs are invisible; skip the work.
    if sigma < 0.5 {
        let fitted = fit_within(source, max_side);
        let image = fitted.as_ref().unwrap_or(source);
        return ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
    }

    let scale = (MAX_BLUR_SIGMA / sigma)
        .min(MAX_BLURRED_SIZE.min(max_side as f32) / longest)
        .min(1.0);
    let mut working = if scale < 1.0 {
        downscale(source, scale, max_side)
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

    /// A typical `GL_MAX_TEXTURE_SIZE`, large enough not to matter.
    const GPU_LIMIT: usize = 16384;

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
            let pixels = background_pixels(&source, blur, GPU_LIMIT);
            assert_eq!(pixels.size, [8, 4]);
            assert_eq!(pixels.pixels[9], egui::Color32::from_rgb(30, 60, 7));
        }
    }

    #[test]
    fn blur_is_relative_to_the_image_size() {
        // sigma = 0.05 * 0.4 * 100 = 2 px, small enough to blur at full size.
        let small = background_pixels(&line_image(100), 0.4, GPU_LIMIT);
        assert_eq!(small.size, [100, 100]);
        assert!(red_at(&small, 50, 50) < 255);
        assert!(red_at(&small, 52, 50) > 0);
        assert_eq!(red_at(&small, 70, 50), 0);

        // The same setting on an image twice as large spreads twice as far.
        let large = background_pixels(&line_image(200), 0.4, GPU_LIMIT);
        assert!(red_at(&large, 104, 100) > red_at(&small, 54, 50));
        assert!(red_at(&large, 100, 100) < red_at(&small, 50, 50));
    }

    #[test]
    fn large_blurs_are_computed_on_a_downscaled_copy() {
        let color = image::Rgba([10, 20, 30, 255]);
        let source = RgbaImage::from_pixel(4000, 2000, color);
        // sigma = 0.05 * 1.0 * 4000 = 200 px, so the image is scaled by
        // MAX_BLUR_SIGMA / 200 before blurring.
        let pixels = background_pixels(&source, 1.0, GPU_LIMIT);
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
        let pixels = background_pixels(&RgbaImage::new(8192, 1024), 0.01, GPU_LIMIT);
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
        let pixels = background_pixels(&source, 0.5, GPU_LIMIT);
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
            Some(Path::new(&format!("{EMBEDDED_IMAGE_PREFIX}{encoded}")).into());
        layout.settings.transparent_background = false;
        let timer = timer();
        let ctx = Context::default();
        let mut renderer = Renderer::default();
        let mut uploads = |layout: &Layout| {
            background_uploads(&ctx, &mut renderer, layout, &timer, RawInput::default()).len()
        };

        assert_eq!(uploads(&layout), 1);
        assert_eq!(uploads(&layout), 0, "an unchanged blur must not re-blur");
        layout.settings.image_blur = 0.5;
        assert_eq!(uploads(&layout), 1, "a new blur must rebuild the texture");
        assert_eq!(uploads(&layout), 0);

        // A reloaded layout holds a new copy of the same embedded image.
        let mut reloaded = layout.clone();
        reloaded.settings.background_image = layout
            .settings
            .background_image
            .as_deref()
            .map(|image| Path::new(image.as_os_str()).into());
        assert_eq!(
            uploads(&reloaded),
            0,
            "the same image must not be decoded again"
        );
    }

    /// Draws one frame and returns the sizes of the textures uploaded from
    /// scratch, other than egui's font atlas.
    fn background_uploads(
        ctx: &Context,
        renderer: &mut Renderer,
        layout: &Layout,
        timer: &Timer,
        input: RawInput,
    ) -> Vec<[usize; 2]> {
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 300.0))),
                ..input
            },
            |ui| renderer.draw(ui, layout, timer),
        );
        let sizes = output
            .textures_delta
            .set
            .iter()
            .filter(|(id, _)| **id != egui::TextureId::default())
            .flat_map(|(_, deltas)| deltas.iter())
            .filter(|delta| delta.pos.is_none())
            .map(|delta| delta.image.size())
            .collect();
        output.textures_delta.clear();
        sizes
    }

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustsplit-render-test-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_png(path: &Path, width: u32, height: u32) {
        RgbaImage::from_pixel(width, height, image::Rgba([40, 80, 120, 255]))
            .save_with_format(path, image::ImageFormat::Png)
            .unwrap();
    }

    fn image_layout(image: &Path) -> Layout {
        let mut layout = Layout::default();
        layout.settings.background_type = BackgroundType::Image;
        layout.settings.background_image = Some(image.into());
        layout.settings.transparent_background = false;
        layout
    }

    #[test]
    fn huge_unblurred_images_fit_the_texture_limit() {
        // A panorama far wider than any GPU texture.
        let pixels = background_pixels(&RgbaImage::new(10000, 500), 0.0, GPU_LIMIT);
        assert_eq!(pixels.size, [4096, 205]);
        let pixels = background_pixels(&RgbaImage::new(500, 10000), 0.0, 1024);
        assert_eq!(pixels.size, [51, 1024]);
        // The blur path honors a GPU limit below its own cap too.
        let pixels = background_pixels(&RgbaImage::new(8192, 1024), 0.01, 1024);
        assert_eq!(pixels.size, [1024, 128]);
        // Images that fit are left alone.
        assert_eq!(
            background_pixels(&RgbaImage::new(1024, 1), 0.0, 1024).size,
            [1024, 1]
        );

        let dir = TestDir::new("huge");
        let path = dir.0.join("wide.png");
        write_png(&path, 5000, 500);
        let ctx = Context::default();
        let mut renderer = Renderer::default();
        let input = RawInput {
            max_texture_side: Some(2048),
            ..Default::default()
        };
        let uploads =
            background_uploads(&ctx, &mut renderer, &image_layout(&path), &timer(), input);
        assert_eq!(uploads, [[2048, 205]]);
    }

    #[test]
    fn background_files_are_reloaded_when_they_change() {
        let dir = TestDir::new("reload");
        let path = dir.0.join("background.png");
        let layout = image_layout(&path);
        let timer = timer();
        let ctx = Context::default();
        let mut renderer = Renderer::default();
        let uploads = |renderer: &mut Renderer, time: f64| {
            let input = RawInput {
                time: Some(time),
                ..Default::default()
            };
            background_uploads(&ctx, renderer, &layout, &timer, input)
        };
        let r = &mut renderer;

        // A missing file is retried after a backoff, not every frame.
        assert!(uploads(r, 0.0).is_empty());
        write_png(&path, 8, 4);
        assert!(uploads(r, 0.5).is_empty(), "too early to look again");
        assert_eq!(uploads(r, 1.5), [[8, 4]]);

        // A replaced file is noticed on the next check.
        write_png(&path, 6, 3);
        assert!(
            uploads(r, 1.6).is_empty(),
            "files are checked about once a second"
        );
        assert_eq!(uploads(r, 2.6), [[6, 3]]);
        assert!(
            uploads(r, 4.0).is_empty(),
            "an unchanged file is not decoded again"
        );

        // A broken file waits longer after every failure.
        std::fs::write(&path, b"not an image").unwrap();
        assert!(uploads(r, 5.0).is_empty());
        let background = r.background.as_ref().unwrap();
        assert!(background.source.is_none());
        assert_eq!(background.retry_at, 6.0);
        assert!(uploads(r, 6.0).is_empty());
        assert_eq!(r.background.as_ref().unwrap().retry_at, 8.0);
    }

    #[test]
    fn background_is_freed_when_no_image_is_drawn() {
        let dir = TestDir::new("free");
        let path = dir.0.join("background.png");
        write_png(&path, 8, 4);
        let mut layout = image_layout(&path);
        let timer = timer();
        let ctx = Context::default();
        let mut renderer = Renderer::default();
        let frame = |layout: &Layout, renderer: &mut Renderer| {
            background_uploads(&ctx, renderer, layout, &timer, RawInput::default()).len()
        };

        assert_eq!(frame(&layout, &mut renderer), 1);
        assert!(renderer.background.is_some());
        layout.settings.background_type = BackgroundType::VerticalGradient;
        frame(&layout, &mut renderer);
        assert!(renderer.background.is_none());
        layout.settings.background_type = BackgroundType::Image;
        layout.settings.background_image = None;
        assert_eq!(frame(&layout, &mut renderer), 0);
        assert!(renderer.background.is_none());
    }

    #[test]
    fn image_background_without_an_image_is_black() {
        let mut layout = Layout::default();
        layout.settings.background_type = BackgroundType::Image;
        layout.settings.background_image = None;
        layout.settings.transparent_background = false;
        let (_, shapes) = render(&layout, &timer(), vec2(300.0, 500.0));
        assert!(shapes.iter().any(|(_, shape)| matches!(
            shape,
            egui::Shape::Rect(rect)
                if rect.fill == egui::Color32::BLACK && rect.rect.size() == vec2(300.0, 500.0)
        )));
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
        let path = format!("{EMBEDDED_IMAGE_PREFIX}{encoded}");
        let image = decode_background(Path::new(&path)).expect("the embedded bitmap must decode");
        assert_eq!(image.dimensions(), (16, 16));
    }
}
