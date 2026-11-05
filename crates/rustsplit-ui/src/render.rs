//! Draws a complete layout.

use std::path::PathBuf;

use egui::{emath::TSTransform, ColorImage, Rect, TextureHandle, TextureOptions, Ui};

use rustsplit_core::Timer;

use crate::canvas::paint_background;
use crate::components::ComponentContext;
use crate::icon::IconCache;
use crate::layout::{BackgroundStyle, Layout, LayoutMode, LayoutSettings};

pub struct Renderer {
    pub icons: IconCache,
    /// Last rectangle used for each component, in screen coordinates.  Useful
    /// for editors and debugging.
    pub last_component_rects: Vec<Rect>,
    background: Option<(PathBuf, TextureHandle)>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self {
            icons: IconCache::new(),
            last_component_rects: Vec::new(),
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

    fn ensure_background(&mut self, ctx: &egui::Context, path: &PathBuf) {
        if self
            .background
            .as_ref()
            .is_some_and(|(current, _)| current == path)
        {
            return;
        }
        self.background = None;
        let path_text = path.to_string_lossy();
        let bytes = if let Some(encoded) = path_text.strip_prefix("livesplit-embedded-image:") {
            use base64::Engine;
            match base64::engine::general_purpose::STANDARD.decode(encoded.trim()) {
                Ok(bytes) => bytes,
                Err(_) => return,
            }
        } else {
            match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(_) => return,
            }
        };
        let Ok(image) = image::load_from_memory(&bytes) else {
            return;
        };
        let rgba = image.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        if size[0] == 0 || size[1] == 0 {
            return;
        }
        let color_image = ColorImage::from_rgba_unmultiplied(size, &rgba.into_raw());
        let texture = ctx.load_texture(
            format!("rustsplit-background-{}", path.display()),
            color_image,
            TextureOptions::LINEAR,
        );
        self.background = Some((path.clone(), texture));
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

        if !transparent {
            if let Some(path) = &layout.settings.background_image {
                self.ensure_background(ui.ctx(), path);
            }
        }
        let background_texture = if transparent {
            None
        } else {
            self.background
                .as_ref()
                .map(|(_, texture)| (texture.id(), texture.size_vec2()))
        };
        let image_opacity = layout.settings.image_opacity.clamp(0.0, 1.0);
        let icons = &mut self.icons;
        let last_rects = &mut self.last_component_rects;

        root.with_visual_transform(transform, |ui| {
            // Layout background image.
            if let Some((texture, image_size)) = background_texture {
                let image_scale =
                    (virtual_rect.width() / image_size.x).max(virtual_rect.height() / image_size.y);
                let size = image_size * image_scale;
                let rect = Rect::from_center_size(virtual_rect.center(), size);
                ui.painter().image(
                    texture,
                    rect,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha((255.0 * image_opacity) as u8),
                );
            }

            // Layout background. Use the shared background painter so a
            // "Plain" background is a solid color; interpolating to the
            // second color would make opaque LiveSplit layouts fade out.
            if !transparent {
                let background = BackgroundStyle {
                    color: layout.settings.background_color,
                    color2: layout.settings.background_color2,
                    gradient: layout.settings.background_gradient,
                };
                paint_background(ui.painter(), virtual_rect, &background);
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
                        component.component.draw(ui, rect, &mut ctx);
                        x += width;
                    }
                }
            }
        });
    }
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

    use crate::layout::GradientType;

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
        layout.settings.background_gradient = GradientType::Plain;
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
}
