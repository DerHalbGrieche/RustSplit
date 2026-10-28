//! Low-level painting helpers shared by the components.

use egui::{
    Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2,
};

use crate::layout::{BackgroundStyle, GradientType};

/// Fill a rectangle with a solid color or a two-color gradient.
pub fn paint_background(painter: &Painter, rect: Rect, style: &BackgroundStyle) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    if style.gradient == GradientType::Plain
        || style.gradient == GradientType::PlainWithDeltaColor
        || style.color2 == style.color
    {
        if style.color.a() > 0 {
            painter.rect_filled(rect, 0.0, style.color);
        }
        return;
    }
    paint_gradient(
        painter,
        rect,
        style.color,
        style.color2,
        style.gradient.is_horizontal(),
    );
}

pub fn paint_gradient(
    painter: &Painter,
    rect: Rect,
    start: Color32,
    end: Color32,
    horizontal: bool,
) {
    if start.a() == 0 && end.a() == 0 {
        return;
    }
    let mut mesh = Mesh::default();
    let (c1, c2, c3, c4) = if horizontal {
        (
            start,
            end,
            start,
            end,
        )
    } else {
        (
            start,
            start,
            end,
            end,
        )
    };
    mesh.colored_vertex(rect.left_top(), c1);
    mesh.colored_vertex(rect.right_top(), c2);
    mesh.colored_vertex(rect.left_bottom(), c3);
    mesh.colored_vertex(rect.right_bottom(), c4);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    painter.add(Shape::mesh(mesh));
}

/// Draw text with optional drop shadow and outline.
#[allow(clippy::too_many_arguments)]
pub fn paint_text(
    painter: &Painter,
    pos: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    shadow: Option<Color32>,
    outline: Option<Color32>,
) {
    if text.is_empty() {
        return;
    }
    if let Some(outline) = outline.filter(|color| color.a() > 0) {
        for (dx, dy) in [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 0.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
        ] {
            painter.text(
                pos + Vec2::new(dx, dy),
                align,
                text,
                font.clone(),
                outline,
            );
        }
    }
    if let Some(shadow) = shadow.filter(|color| color.a() > 0) {
        painter.text(
            pos + Vec2::new(1.5, 1.5),
            align,
            text,
            font.clone(),
            shadow,
        );
    }
    painter.text(pos, align, text, font, color);
}

/// Width of a string using the given font.
pub fn text_width(painter: &Painter, text: &str, font: &FontId) -> f32 {
    painter
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x
}

/// Truncate text with an ellipsis so it fits into `max_width`.
pub fn fit_text(painter: &Painter, text: &str, font: &FontId, max_width: f32) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    if text_width(painter, text, font) <= max_width {
        return text.to_owned();
    }
    let ellipsis = "…";
    let ellipsis_width = text_width(painter, ellipsis, font);
    if ellipsis_width >= max_width {
        return String::new();
    }
    let mut low = 0;
    let mut high = text.chars().count();
    while low < high {
        let mid = (low + high + 1) / 2;
        let candidate: String = text.chars().take(mid).collect::<String>() + ellipsis;
        if text_width(painter, &candidate, font) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let mut result: String = text.chars().take(low).collect();
    result.push_str(ellipsis);
    result
}

/// Draw a horizontal line.
pub fn divider(painter: &Painter, rect: Rect, color: Color32, y: f32) {
    painter.line_segment(
        [
            Pos2::new(rect.left(), y),
            Pos2::new(rect.right(), y),
        ],
        Stroke::new(1.0, color),
    );
}

/// A vertical gradient for alternating row backgrounds.
pub fn paint_alternating(
    painter: &Painter,
    rect: Rect,
    row: usize,
    style: &BackgroundStyle,
) {
    if style.gradient == GradientType::Alternating {
        let color = if row % 2 == 0 {
            style.color
        } else {
            style.color2
        };
        if color.a() > 0 {
            painter.rect_filled(rect, 0.0, color);
        }
    } else {
        paint_background(painter, rect, style);
    }
}
