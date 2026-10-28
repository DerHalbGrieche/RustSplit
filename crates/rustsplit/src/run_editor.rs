//! The splits (run) editor.

use std::collections::HashMap;

use egui::{Context, Grid, Key, RichText, ScrollArea, TextEdit};

use livesplit_core::{format_time, Run, Segment, TimeAccuracy, TimeSpan};

use crate::config::AppConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEditorEvent {
    None,
    Apply,
    ApplyAndClose,
    Save,
    SaveAs,
    Cancel,
}

pub struct RunEditor {
    pub open: bool,
    draft: Option<Run>,
    selected_segment: usize,
    comparison: String,
    time_buffers: HashMap<String, String>,
    new_comparison: String,
    error: Option<String>,
}

impl Default for RunEditor {
    fn default() -> Self {
        Self {
            open: false,
            draft: None,
            selected_segment: 0,
            comparison: livesplit_core::PERSONAL_BEST.to_string(),
            time_buffers: HashMap::new(),
            new_comparison: String::new(),
            error: None,
        }
    }
}

impl RunEditor {
    pub fn open(&mut self, run: &Run) {
        self.draft = Some(run.clone());
        self.open = true;
        self.selected_segment = 0;
        self.comparison = livesplit_core::PERSONAL_BEST.to_string();
        self.time_buffers.clear();
        self.new_comparison.clear();
        self.error = None;
    }

    pub fn draft_cloned(&self) -> Option<Run> {
        self.draft.clone()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.draft = None;
        self.time_buffers.clear();
    }

    pub fn show(&mut self, ctx: &Context, config: &AppConfig) -> RunEditorEvent {
        if !self.open {
            return RunEditorEvent::None;
        }
        let Some(draft) = self.draft.as_mut() else {
            self.open = false;
            return RunEditorEvent::Cancel;
        };
        let mut event = RunEditorEvent::None;
        let mut open = self.open;
        let mut delete_attempt: Option<i32> = None;
        let mut delete_comparison: Option<String> = None;
        let buffers = &mut self.time_buffers;
        let selected_segment = &mut self.selected_segment;
        let comparison = &mut self.comparison;
        let new_comparison = &mut self.new_comparison;
        let error = &mut self.error;

        egui::Window::new("Splits Editor")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(900.0)
            .default_height(680.0)
            .default_pos(egui::pos2(12.0, 12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Game:");
                    ui.add(TextEdit::singleline(&mut draft.game_name).desired_width(230.0));
                    ui.label("Category:");
                    ui.add(
                        TextEdit::singleline(&mut draft.category_name).desired_width(230.0),
                    );
                    ui.label("Offset:");
                    time_edit_single(
                        ui,
                        buffers,
                        "run-offset",
                        &mut draft.offset,
                        true,
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Attempt count:");
                    ui.add(egui::DragValue::new(&mut draft.attempt_count).range(0..=999_999));
                    ui.checkbox(&mut draft.has_changed, "Modified");
                    if let Some(path) = &draft.file_path {
                        ui.label(format!("File: {}", path.display()));
                    }
                });
                if let Some(message) = error.as_ref() {
                    ui.colored_label(egui::Color32::LIGHT_RED, message);
                }
                ui.separator();

                ScrollArea::vertical()
                    .id_salt("run-editor-segments")
                    .max_height(300.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading("Segments");
                            if ui.button("+ Add").clicked() {
                                draft.segments.push(Segment::new(format!(
                                    "Segment {}",
                                    draft.segments.len() + 1
                                )));
                                *selected_segment = draft.segments.len() - 1;
                            }
                            if ui
                                .add_enabled(
                                    draft.segments.len() > 1,
                                    egui::Button::new("Remove"),
                                )
                                .clicked()
                                && *selected_segment < draft.segments.len()
                            {
                                draft.segments.remove(*selected_segment);
                                *selected_segment = (*selected_segment)
                                    .saturating_sub(1)
                                    .min(draft.segments.len() - 1);
                            }
                            if ui
                                .add_enabled(*selected_segment > 0, egui::Button::new("↑"))
                                .clicked()
                            {
                                draft.segments.swap(*selected_segment, *selected_segment - 1);
                                *selected_segment -= 1;
                            }
                            if ui
                                .add_enabled(
                                    *selected_segment + 1 < draft.segments.len(),
                                    egui::Button::new("↓"),
                                )
                                .clicked()
                            {
                                draft.segments.swap(*selected_segment, *selected_segment + 1);
                                *selected_segment += 1;
                            }
                            ui.separator();
                            ui.label("Edit comparison:");
                            egui::ComboBox::from_id_salt("comparison-picker")
                                .selected_text(comparison.as_str())
                                .show_ui(ui, |ui| {
                                    for name in draft.comparisons() {
                                        ui.selectable_value(comparison, name.clone(), name);
                                    }
                                });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Custom comparisons:");
                            for name in draft.custom_comparisons.clone() {
                                if name == livesplit_core::PERSONAL_BEST {
                                    ui.label(RichText::new("Personal Best").strong());
                                } else {
                                    ui.label(&name);
                                    if ui.small_button("✕").clicked() {
                                        delete_comparison = Some(name);
                                    }
                                }
                            }
                            ui.add(
                                TextEdit::singleline(new_comparison)
                                    .desired_width(120.0)
                                    .hint_text("new comparison"),
                            );
                            if ui.button("Add comparison").clicked()
                                && !new_comparison.trim().is_empty()
                            {
                                let name = new_comparison.trim().to_string();
                                if !draft.custom_comparisons.contains(&name) {
                                    draft.custom_comparisons.push(name.clone());
                                    for segment in &mut draft.segments {
                                        segment.comparisons.entry(name.clone()).or_default();
                                    }
                                    *comparison = name;
                                    new_comparison.clear();
                                }
                            }
                        });
                        ui.separator();

                        let mut clicked_segment = None;
                        ScrollArea::horizontal()
                            .id_salt("run-editor-segments-horizontal")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                        Grid::new("segments-grid")
                            .num_columns(7)
                            .min_col_width(80.0)
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("#");
                                ui.strong("Name");
                                ui.strong("Comparison RTA");
                                ui.strong("Comparison IGT");
                                ui.strong("Best RTA");
                                ui.strong("Best IGT");
                                ui.strong("History");
                                ui.end_row();

                                for (index, segment) in draft.segments.iter_mut().enumerate() {
                                    let selected = index == *selected_segment;
                                    if ui
                                        .selectable_label(selected, format!("{}", index + 1))
                                        .clicked()
                                    {
                                        clicked_segment = Some(index);
                                    }
                                    ui.add(
                                        TextEdit::singleline(&mut segment.name)
                                            .desired_width(120.0),
                                    );
                                    let comp_key = format!("comp-real-{index}-{comparison}");
                                    let comp_key_game = format!("comp-game-{index}-{comparison}");
                                    let mut comp = if comparison == livesplit_core::PERSONAL_BEST
                                    {
                                        segment.personal_best()
                                    } else {
                                        segment.comparison(comparison)
                                    };
                                    time_edit(
                                        ui,
                                        buffers,
                                        &comp_key,
                                        &mut comp.real_time,
                                        false,
                                    );
                                    time_edit(
                                        ui,
                                        buffers,
                                        &comp_key_game,
                                        &mut comp.game_time,
                                        false,
                                    );
                                    if comparison == livesplit_core::PERSONAL_BEST {
                                        segment.set_personal_best(comp);
                                    } else {
                                        segment.set_comparison(comparison.clone(), comp);
                                    }

                                    let best_key = format!("best-real-{index}");
                                    let best_key_game = format!("best-game-{index}");
                                    time_edit(
                                        ui,
                                        buffers,
                                        &best_key,
                                        &mut segment.best_segment_time.real_time,
                                        false,
                                    );
                                    time_edit(
                                        ui,
                                        buffers,
                                        &best_key_game,
                                        &mut segment.best_segment_time.game_time,
                                        false,
                                    );
                                    ui.label(format!(
                                        "{} attempts",
                                        segment.segment_history.len()
                                    ));
                                    ui.end_row();
                                }
                            });
                            });
                        if let Some(index) = clicked_segment {
                            *selected_segment = index;
                        }
                    });

                ui.separator();
                ui.collapsing("Attempt history", |ui| {
                    if draft.attempts.is_empty() {
                        ui.label("No attempts recorded yet.");
                    }
                    ScrollArea::vertical()
                        .id_salt("run-editor-attempts")
                        .max_height(160.0)
                        .show(ui, |ui| {
                        Grid::new("attempts-grid").num_columns(6).striped(true).show(ui, |ui| {
                            ui.strong("ID");
                            ui.strong("Time");
                            ui.strong("Started");
                            ui.strong("Ended");
                            ui.strong("Pause");
                            ui.label("");
                            ui.end_row();
                            for attempt in draft.attempts.iter() {
                                ui.label(attempt.index.to_string());
                                ui.label(format_time(attempt.time.real_time, TimeAccuracy::Milliseconds));
                                ui.label(
                                    attempt
                                        .started
                                        .map(|date| date.format("%Y-%m-%d %H:%M:%S").to_string())
                                        .unwrap_or_default(),
                                );
                                ui.label(
                                    attempt
                                        .ended
                                        .map(|date| date.format("%Y-%m-%d %H:%M:%S").to_string())
                                        .unwrap_or_default(),
                                );
                                ui.label(
                                    attempt
                                        .pause_time
                                        .map(|time| time.to_short_string(TimeAccuracy::Tenths))
                                        .unwrap_or_default(),
                                );
                                if ui.small_button("Delete").clicked() {
                                    delete_attempt = Some(attempt.index);
                                }
                                ui.end_row();
                            }
                        });
                    });
                });
                ui.collapsing("Metadata", |ui| {
                    Grid::new("metadata-grid").num_columns(2).show(ui, |ui| {
                        ui.label("Run ID");
                        let mut run_id = draft.metadata.run_id.clone().unwrap_or_default();
                        if ui.text_edit_singleline(&mut run_id).changed() {
                            draft.metadata.run_id = if run_id.trim().is_empty() {
                                None
                            } else {
                                Some(run_id)
                            };
                        }
                        ui.end_row();
                        ui.label("Platform");
                        let mut platform = draft.metadata.platform_name.clone().unwrap_or_default();
                        if ui.text_edit_singleline(&mut platform).changed() {
                            draft.metadata.platform_name = if platform.trim().is_empty() {
                                None
                            } else {
                                Some(platform)
                            };
                        }
                        ui.end_row();
                        ui.label("Region");
                        let mut region = draft.metadata.region_name.clone().unwrap_or_default();
                        if ui.text_edit_singleline(&mut region).changed() {
                            draft.metadata.region_name = if region.trim().is_empty() {
                                None
                            } else {
                                Some(region)
                            };
                        }
                        ui.end_row();
                        ui.label("Uses emulator");
                        ui.checkbox(&mut draft.metadata.uses_emulator, "");
                        ui.end_row();
                    });
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        if let Err(message) = validate(draft) {
                            *error = Some(message);
                        } else {
                            draft.ensure_personal_best();
                            event = RunEditorEvent::Apply;
                        }
                    }
                    if ui.button("OK").clicked() {
                        if let Err(message) = validate(draft) {
                            *error = Some(message);
                        } else {
                            draft.ensure_personal_best();
                            event = RunEditorEvent::ApplyAndClose;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        event = RunEditorEvent::Cancel;
                    }
                    ui.separator();
                    if ui.button("Save").clicked() {
                        if let Err(message) = validate(draft) {
                            *error = Some(message);
                        } else {
                            draft.ensure_personal_best();
                            event = RunEditorEvent::Save;
                        }
                    }
                    if ui.button("Save As…").clicked() {
                        if let Err(message) = validate(draft) {
                            *error = Some(message);
                        } else {
                            draft.ensure_personal_best();
                            event = RunEditorEvent::SaveAs;
                        }
                    }
                    if config.general.autosave {
                        ui.label(
                            RichText::new("Autosave is on")
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
                if ctx.input(|input| input.key_pressed(Key::Escape)) {
                    event = RunEditorEvent::Cancel;
                }
            });

        if !open || event == RunEditorEvent::Cancel {
            self.close();
            return RunEditorEvent::Cancel;
        }
        self.open = open;

        if let Some(attempt) = delete_attempt {
            if let Some(draft) = self.draft.as_mut() {
                draft.attempts.retain(|item| item.index != attempt);
                for segment in &mut draft.segments {
                    segment.segment_history.remove(&attempt);
                }
                draft.fix_splits();
            }
        }
        if let Some(comparison) = delete_comparison {
            if let Some(draft) = self.draft.as_mut() {
                if comparison != livesplit_core::PERSONAL_BEST {
                    draft.custom_comparisons.retain(|name| name != &comparison);
                    for segment in &mut draft.segments {
                        segment.comparisons.remove(&comparison);
                    }
                    if self.comparison == comparison {
                        self.comparison = livesplit_core::PERSONAL_BEST.to_string();
                    }
                }
            }
        }
        event
    }
}

fn validate(run: &Run) -> Result<(), String> {
    if run.segments.is_empty() {
        return Err("A run needs at least one segment.".to_string());
    }
    if run
        .segments
        .iter()
        .any(|segment| segment.name.trim().is_empty())
    {
        return Err("Segment names must not be empty.".to_string());
    }
    Ok(())
}

fn time_edit_single(
    ui: &mut egui::Ui,
    buffers: &mut HashMap<String, String>,
    key: &str,
    value: &mut TimeSpan,
    allow_negative: bool,
) {
    let mut option = Some(*value);
    time_edit(ui, buffers, key, &mut option, allow_negative);
    *value = option.unwrap_or_default();
}

fn time_edit(
    ui: &mut egui::Ui,
    buffers: &mut HashMap<String, String>,
    key: &str,
    value: &mut Option<TimeSpan>,
    allow_negative: bool,
) {
    let buffer = buffers
        .entry(key.to_string())
        .or_insert_with(|| {
            value
                .map(|time| {
                    let mut formatter = livesplit_core::GeneralTimeFormatter::default();
                    formatter.accuracy = TimeAccuracy::Milliseconds;
                    formatter.null_format = livesplit_core::NullFormat::ZeroValue;
                    formatter.show_days = false;
                    formatter.format(Some(time))
                })
                .unwrap_or_default()
        });
    let response = ui.add(
        TextEdit::singleline(buffer)
            .desired_width(86.0)
            .hint_text("0.00"),
    );
    if response.changed() {
        let text = buffer.trim();
        if text.is_empty() {
            *value = None;
        } else if let Ok(parsed) = TimeSpan::parse(text) {
            if allow_negative || !parsed.is_negative() {
                *value = Some(parsed);
            }
        }
    }
    if response.lost_focus() {
        if let Some(time) = *value {
            let mut formatter = livesplit_core::GeneralTimeFormatter::default();
            formatter.accuracy = TimeAccuracy::Milliseconds;
            formatter.null_format = livesplit_core::NullFormat::ZeroValue;
            *buffer = formatter.format(Some(time));
        } else {
            buffer.clear();
        }
    }
}
