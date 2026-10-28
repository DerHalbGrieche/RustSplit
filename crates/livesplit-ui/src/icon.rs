//! Lazy decoding and caching of split icons.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};

#[derive(Default)]
pub struct IconCache {
    textures: HashMap<u64, Option<TextureHandle>>,
}

impl IconCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or decode a texture for the given image bytes.
    pub fn texture(&mut self, ctx: &Context, bytes: &[u8]) -> Option<TextureId> {
        if bytes.is_empty() {
            return None;
        }
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let key = hasher.finish();
        if let Some(texture) = self.textures.get(&key) {
            return texture.as_ref().map(TextureHandle::id);
        }

        let decoded = image::load_from_memory(bytes).ok().and_then(|image| {
            let rgba = image.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = rgba.into_raw();
            if size[0] == 0 || size[1] == 0 {
                return None;
            }
            let image = ColorImage::from_rgba_unmultiplied(size, &pixels);
            Some(ctx.load_texture(
                format!("rustsplit-icon-{key:016x}"),
                image,
                TextureOptions::LINEAR,
            ))
        });
        let id = decoded.as_ref().map(TextureHandle::id);
        self.textures.insert(key, decoded);
        id
    }

    pub fn clear(&mut self) {
        self.textures.clear();
    }
}
