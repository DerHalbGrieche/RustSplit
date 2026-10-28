//! Global hotkey registration.

use std::collections::HashMap;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::config::HotkeyConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    StartSplit,
    Reset,
    Undo,
    Skip,
    Pause,
    UndoAllPauses,
    SwitchComparisonNext,
    SwitchComparisonPrevious,
    ToggleTimingMethod,
    EditRun,
    EditLayout,
}

impl HotkeyAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::StartSplit => "Start / Split",
            Self::Reset => "Reset",
            Self::Undo => "Undo Split",
            Self::Skip => "Skip Split",
            Self::Pause => "Pause / Resume",
            Self::UndoAllPauses => "Undo All Pauses",
            Self::SwitchComparisonNext => "Next Comparison",
            Self::SwitchComparisonPrevious => "Previous Comparison",
            Self::ToggleTimingMethod => "Toggle Timing Method",
            Self::EditRun => "Edit Splits",
            Self::EditLayout => "Edit Layout",
        }
    }

    pub fn all() -> &'static [HotkeyAction] {
        &[
            Self::StartSplit,
            Self::Reset,
            Self::Undo,
            Self::Skip,
            Self::Pause,
            Self::UndoAllPauses,
            Self::SwitchComparisonNext,
            Self::SwitchComparisonPrevious,
            Self::ToggleTimingMethod,
            Self::EditRun,
            Self::EditLayout,
        ]
    }

    pub fn config_value<'a>(self, config: &'a HotkeyConfig) -> &'a str {
        match self {
            Self::StartSplit => &config.start_split,
            Self::Reset => &config.reset,
            Self::Undo => &config.undo,
            Self::Skip => &config.skip,
            Self::Pause => &config.pause,
            Self::UndoAllPauses => &config.undo_all_pauses,
            Self::SwitchComparisonNext => &config.switch_comparison_next,
            Self::SwitchComparisonPrevious => &config.switch_comparison_previous,
            Self::ToggleTimingMethod => &config.toggle_timing_method,
            Self::EditRun => &config.edit_run,
            Self::EditLayout => &config.edit_layout,
        }
    }

    pub fn config_value_mut<'a>(self, config: &'a mut HotkeyConfig) -> &'a mut String {
        match self {
            Self::StartSplit => &mut config.start_split,
            Self::Reset => &mut config.reset,
            Self::Undo => &mut config.undo,
            Self::Skip => &mut config.skip,
            Self::Pause => &mut config.pause,
            Self::UndoAllPauses => &mut config.undo_all_pauses,
            Self::SwitchComparisonNext => &mut config.switch_comparison_next,
            Self::SwitchComparisonPrevious => &mut config.switch_comparison_previous,
            Self::ToggleTimingMethod => &mut config.toggle_timing_method,
            Self::EditRun => &mut config.edit_run,
            Self::EditLayout => &mut config.edit_layout,
        }
    }
}

pub struct GlobalHotkeys {
    manager: Option<GlobalHotKeyManager>,
    actions: HashMap<u32, HotkeyAction>,
    registered: Vec<HotKey>,
    pub available: bool,
    pub error: Option<String>,
}

impl Default for GlobalHotkeys {
    fn default() -> Self {
        Self {
            manager: None,
            actions: HashMap::new(),
            registered: Vec::new(),
            available: true,
            error: None,
        }
    }
}

impl GlobalHotkeys {
    pub fn re_register(&mut self, config: &HotkeyConfig) {
        self.actions.clear();
        self.error = None;
        if !config.enabled {
            if let Some(manager) = &self.manager {
                if !self.registered.is_empty() {
                    let _ = manager.unregister_all(&self.registered);
                    self.registered.clear();
                }
            }
            return;
        }
        if self.manager.is_none() {
            match GlobalHotKeyManager::new() {
                Ok(manager) => self.manager = Some(manager),
                Err(error) => {
                    self.available = false;
                    self.error = Some(format!("Global hotkeys unavailable: {error}"));
                    return;
                }
            }
        }
        let Some(manager) = &self.manager else {
            return;
        };
        if !self.registered.is_empty() {
            let _ = manager.unregister_all(&self.registered);
            self.registered.clear();
        }
        for action in HotkeyAction::all() {
            let text = action.config_value(config).trim();
            if text.is_empty() {
                continue;
            }
            match text.parse::<HotKey>() {
                Ok(hotkey) => {
                    if let Err(error) = manager.register(hotkey) {
                        self.error = Some(format!(
                            "Could not register {} ({}): {error}",
                            action.label(),
                            text
                        ));
                    } else {
                        self.actions.insert(hotkey.id(), *action);
                        self.registered.push(hotkey);
                    }
                }
                Err(error) => {
                    self.error = Some(format!(
                        "Invalid hotkey for {}: {} ({error})",
                        action.label(),
                        text
                    ));
                }
            }
        }
    }

    pub fn poll(&mut self) -> Vec<HotkeyAction> {
        let mut actions = Vec::new();
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed {
                if let Some(action) = self.actions.get(&event.id) {
                    actions.push(*action);
                }
            }
        }
        actions
    }
}
