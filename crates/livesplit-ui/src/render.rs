//! Draws a complete layout.

use std::path::PathBuf;

use egui::{
    emath::TSTransform, ColorImage, Rect, TextureHandle, TextureOptions, Ui,
};

use livesplit_core::Timer;

use crate::canvas::paint_gradient;
use crate::components::ComponentContext;
use crate::icon::IconCache;
use crate::layout::{Layout, LayoutMode, LayoutSettings};

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

        let base_height = match layout.mode {
            LayoutMode::Vertical => self.natural_height(layout, timer),
            LayoutMode::Horizontal => (layout.horizontal_height.max(50) as f32).max(1.0),
        };
        let base_width = match layout.mode {
            LayoutMode::Vertical => (layout.vertical_width.max(50) as f32).max(1.0),
            LayoutMode::Horizontal => self.natural_width(layout, timer),
        };

        let scale = (available.width() / base_width)
            .min(available.height() / base_height)
            .clamp(0.05, 16.0);


        let mut root = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(available)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        root.set_opacity(layout.settings.opacity.clamp(0.0, 1.0));

        // Draw in unscaled layout coordinates and apply one visual transform.
        // This keeps component code simple and produces LiveSplit-style
        // proportional resizing without double-scaling fonts or row heights.
        let transform = TSTransform::new(available.min.to_vec2(), scale);
        let virtual_rect = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(base_width, base_height));

        if let Some(path) = &layout.settings.background_image {
            self.ensure_background(ui.ctx(), path);
        }
        let background_texture = self
            .background
            .as_ref()
            .map(|(_, texture)| (texture.id(), texture.size_vec2()));
        let image_opacity = layout.settings.image_opacity.clamp(0.0, 1.0);
        let icons = &mut self.icons;
        let last_rects = &mut self.last_component_rects;

        root.with_visual_transform(transform, |ui| {
            // Layout background image.
            if let Some((texture, image_size)) = background_texture {
                let image_scale = (virtual_rect.width() / image_size.x)
                    .max(virtual_rect.height() / image_size.y);
                let size = image_size * image_scale;
                let rect = Rect::from_center_size(virtual_rect.center(), size);
                ui.painter().image(
                    texture,
                    rect,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha((255.0 * image_opacity) as u8),
                );
            }

            // Layout background.
            if layout.settings.background_color.a() > 0
                || layout.settings.background_color2.a() > 0
            {
                paint_gradient(
                    ui.painter(),
                    virtual_rect,
                    layout.settings.background_color,
                    layout.settings.background_color2,
                    layout.settings.background_gradient.is_horizontal(),
                );
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
                            egui::vec2(base_width, height),
                        );
                        last_rects.push(Rect::from_min_max(
                            transform * rect.min,
                            transform * rect.max,
                        ));
                        let mut overridden_theme;
                        let theme = if component.font_overrides.has_overrides() {
                            overridden_theme = layout.settings.clone();
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
                            &layout.settings
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
                            egui::vec2(width, base_height),
                        );
                        last_rects.push(Rect::from_min_max(
                            transform * rect.min,
                            transform * rect.max,
                        ));
                        let mut overridden_theme;
                        let theme = if component.font_overrides.has_overrides() {
                            overridden_theme = layout.settings.clone();
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
                            &layout.settings
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

/// Layout settings used when the application is in an overlay-only mode.
pub fn transparent_theme() -> LayoutSettings {
    LayoutSettings::default()
}

/// Helper for previews in the layout editor.
pub fn preview_layout_height(layout: &Layout, timer: &Timer, renderer: &Renderer) -> f32 {
    renderer.natural_height(layout, timer).max(1.0)
}
