//! Timer sound effects, modelled on LiveSplit's Sound Settings.
//!
//! The app takes a [`Snapshot`] of the timer before each timer action and
//! passes it to [`SoundPlayer::play_timer_change`] afterwards.
//! [`sound_events`] works out what happened by comparing the two, and the
//! player sends the matching file to an audio worker thread.
//!
//! The worker decodes files ahead of time, caches the decoded samples and
//! opens the audio device the first time something plays. The UI thread only
//! sends a message, so a slow decoder or audio device can't delay the timer.
//! Without the `sounds` cargo feature, the player does nothing.

use std::path::{Path, PathBuf};

use egui::{Color32, RichText, Ui};
use rustsplit_core::analysis::check_best_segment;
use rustsplit_core::{TimeSpan, Timer, TimerPhase};

use crate::config::SoundConfig;

/// A timer event that can have a sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundEvent {
    Split,
    SplitAhead,
    SplitBehind,
    BestSegment,
    UndoSplit,
    SkipSplit,
    PersonalBest,
    NotAPersonalBest,
    Reset,
    Pause,
    Resume,
    StartTimer,
}

impl SoundEvent {
    pub const ALL: [SoundEvent; 12] = [
        SoundEvent::Split,
        SoundEvent::SplitAhead,
        SoundEvent::SplitBehind,
        SoundEvent::BestSegment,
        SoundEvent::UndoSplit,
        SoundEvent::SkipSplit,
        SoundEvent::PersonalBest,
        SoundEvent::NotAPersonalBest,
        SoundEvent::Reset,
        SoundEvent::Pause,
        SoundEvent::Resume,
        SoundEvent::StartTimer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SoundEvent::Split => "Split",
            SoundEvent::SplitAhead => "Split (ahead of comparison)",
            SoundEvent::SplitBehind => "Split (behind comparison)",
            SoundEvent::BestSegment => "Best segment",
            SoundEvent::UndoSplit => "Undo split",
            SoundEvent::SkipSplit => "Skip split",
            SoundEvent::PersonalBest => "Personal best",
            SoundEvent::NotAPersonalBest => "Not a personal best",
            SoundEvent::Reset => "Reset",
            SoundEvent::Pause => "Pause",
            SoundEvent::Resume => "Resume",
            SoundEvent::StartTimer => "Start timer",
        }
    }

    pub fn path(self, config: &SoundConfig) -> Option<&Path> {
        let path = match self {
            SoundEvent::Split => &config.split,
            SoundEvent::SplitAhead => &config.split_ahead,
            SoundEvent::SplitBehind => &config.split_behind,
            SoundEvent::BestSegment => &config.best_segment,
            SoundEvent::UndoSplit => &config.undo_split,
            SoundEvent::SkipSplit => &config.skip_split,
            SoundEvent::PersonalBest => &config.personal_best,
            SoundEvent::NotAPersonalBest => &config.not_a_personal_best,
            SoundEvent::Reset => &config.reset,
            SoundEvent::Pause => &config.pause,
            SoundEvent::Resume => &config.resume,
            SoundEvent::StartTimer => &config.start_timer,
        };
        path.as_deref().filter(|path| !path.as_os_str().is_empty())
    }

    pub fn path_mut(self, config: &mut SoundConfig) -> &mut Option<PathBuf> {
        match self {
            SoundEvent::Split => &mut config.split,
            SoundEvent::SplitAhead => &mut config.split_ahead,
            SoundEvent::SplitBehind => &mut config.split_behind,
            SoundEvent::BestSegment => &mut config.best_segment,
            SoundEvent::UndoSplit => &mut config.undo_split,
            SoundEvent::SkipSplit => &mut config.skip_split,
            SoundEvent::PersonalBest => &mut config.personal_best,
            SoundEvent::NotAPersonalBest => &mut config.not_a_personal_best,
            SoundEvent::Reset => &mut config.reset,
            SoundEvent::Pause => &mut config.pause,
            SoundEvent::Resume => &mut config.resume,
            SoundEvent::StartTimer => &mut config.start_timer,
        }
    }
}

/// The timer state that event detection compares against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    phase: TimerPhase,
    split_index: i32,
}

impl Snapshot {
    pub fn capture(timer: &Timer) -> Self {
        Self {
            phase: timer.phase,
            split_index: timer.current_split_index,
        }
    }
}

/// The sounds for the change from `before` to `after`, most specific first.
///
/// The first event with a configured file plays, so a best segment falls
/// back to the ahead/behind sound and then to the plain split sound. The
/// result is empty when nothing that has a sound happened.
pub fn sound_events(before: &Snapshot, after: &Timer) -> Vec<SoundEvent> {
    use TimerPhase::{Ended, NotRunning, Paused, Running};

    match (before.phase, after.phase) {
        (NotRunning, Running | Paused) => return vec![SoundEvent::StartTimer],
        // Like LiveSplit, resetting a finished run is silent.
        (Running | Paused, NotRunning) => return vec![SoundEvent::Reset],
        (_, NotRunning) => return Vec::new(),
        _ => {}
    }

    let index = after.current_split_index;
    if index < before.split_index {
        return vec![SoundEvent::UndoSplit];
    }
    if index == before.split_index + 1 {
        let completed = before.split_index;
        let method = after.current_timing_method;
        let Some(segment) = after.run.segments.get(completed.max(0) as usize) else {
            return Vec::new();
        };
        // `Timer::split` always records a real time, `skip_split` clears it.
        if segment.split_time.real_time.is_none() {
            return vec![SoundEvent::SkipSplit];
        }
        if after.phase == Ended {
            let final_time = segment.split_time.get(method);
            let pb_time = segment.personal_best().get(method);
            let is_pb = match (final_time, pb_time) {
                (Some(final_time), Some(pb_time)) => final_time < pb_time,
                _ => pb_time.is_none(),
            };
            let event = if is_pb {
                SoundEvent::PersonalBest
            } else {
                SoundEvent::NotAPersonalBest
            };
            return vec![event, SoundEvent::Split];
        }
        let mut events = Vec::with_capacity(3);
        if check_best_segment(after, completed, method) {
            events.push(SoundEvent::BestSegment);
        }
        let comparison = segment.comparison(&after.current_comparison).get(method);
        if let (Some(split), Some(comparison)) = (segment.split_time.get(method), comparison) {
            events.push(if split - comparison < TimeSpan::ZERO {
                SoundEvent::SplitAhead
            } else {
                SoundEvent::SplitBehind
            });
        }
        events.push(SoundEvent::Split);
        return events;
    }

    match (before.phase, after.phase) {
        (Running, Paused) => vec![SoundEvent::Pause],
        (Paused, Running) => vec![SoundEvent::Resume],
        _ => Vec::new(),
    }
}

/// Plays configured sounds without blocking the caller.
#[derive(Default)]
pub struct SoundPlayer {
    #[cfg(feature = "sounds")]
    backend: Option<backend::Backend>,
}

impl SoundPlayer {
    /// Whether this build can play sounds at all.
    pub const SUPPORTED: bool = cfg!(feature = "sounds");

    /// Play the sound for the timer change since `before`, if one is set.
    pub fn play_timer_change(&mut self, before: &Snapshot, after: &Timer, config: &SoundConfig) {
        if *before == Snapshot::capture(after) {
            return;
        }
        let path = sound_events(before, after)
            .into_iter()
            .find_map(|event| event.path(config));
        if let Some(path) = path {
            self.play_file(path, config.volume);
        }
    }

    /// Queue `path` for playback at `volume` (0–100).
    pub fn play_file(&mut self, path: &Path, volume: u8) {
        if volume == 0 {
            return;
        }
        let volume = f32::from(volume.min(100)) / 100.0;
        #[cfg(feature = "sounds")]
        if let Some(backend) = self.backend() {
            backend.send(backend::Command::Play(path.to_path_buf(), volume));
        }
        #[cfg(not(feature = "sounds"))]
        let _ = (path, volume);
    }

    /// Decode the configured files in the background so that the first play
    /// doesn't wait for the decoder. Files that are no longer configured are
    /// dropped from the cache.
    pub fn preload(&mut self, config: &SoundConfig) {
        let paths: Vec<PathBuf> = SoundEvent::ALL
            .into_iter()
            .filter_map(|event| event.path(config))
            .map(Path::to_path_buf)
            .collect();
        #[cfg(feature = "sounds")]
        {
            // Without a worker there is no cache to clear.
            let backend = if paths.is_empty() {
                self.backend.as_ref()
            } else {
                self.backend()
            };
            if let Some(backend) = backend {
                backend.send(backend::Command::Preload(paths));
            }
        }
        #[cfg(not(feature = "sounds"))]
        let _ = paths;
    }

    /// Whether opening the audio device failed.
    pub fn device_unavailable(&self) -> bool {
        #[cfg(feature = "sounds")]
        if let Some(backend) = &self.backend {
            return backend.device_unavailable();
        }
        false
    }

    #[cfg(feature = "sounds")]
    fn backend(&mut self) -> Option<&backend::Backend> {
        if self.backend.is_none() {
            self.backend = backend::Backend::spawn();
        }
        self.backend.as_ref()
    }
}

/// The "Sounds" section of the Settings window. Returns the event whose
/// "Browse…" button was clicked.
pub fn settings_ui(
    ui: &mut Ui,
    config: &mut SoundConfig,
    player: &mut SoundPlayer,
) -> Option<SoundEvent> {
    let weak = ui.visuals().weak_text_color();
    if !SoundPlayer::SUPPORTED {
        ui.label(
            RichText::new(
                "Sounds are unavailable: RustSplit was built without the `sounds` feature.",
            )
            .small()
            .color(weak),
        );
        return None;
    }
    if player.device_unavailable() {
        ui.colored_label(Color32::LIGHT_RED, "No audio output device is available.");
    }
    ui.horizontal(|ui| {
        ui.label("Volume:");
        ui.add(egui::Slider::new(&mut config.volume, 0..=100).suffix("%"));
    });
    let volume = config.volume;
    let mut browse = None;
    egui::Grid::new("sound-grid")
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            for event in SoundEvent::ALL {
                ui.label(event.label());
                let path = event.path_mut(config);
                let mut text = path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text("No sound")
                        .desired_width(220.0),
                );
                if edit.changed() {
                    *path = (!text.trim().is_empty()).then(|| PathBuf::from(text));
                }
                ui.horizontal(|ui| {
                    if ui.button("Browse…").clicked() {
                        browse = Some(event);
                    }
                    let file = path.clone().filter(|path| !path.as_os_str().is_empty());
                    if ui
                        .add_enabled(file.is_some(), egui::Button::new("Test"))
                        .clicked()
                    {
                        if let Some(file) = file {
                            player.play_file(&file, volume);
                        }
                    }
                });
                ui.end_row();
            }
        });
    ui.label(
        RichText::new(
            "WAV, Ogg Vorbis, MP3 and FLAC files. Unset split sounds fall back to Split.",
        )
        .small()
        .color(weak),
    );
    browse
}

#[cfg(feature = "sounds")]
mod backend {
    use std::collections::{HashMap, HashSet};
    use std::fs::File;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use rodio::buffer::SamplesBuffer;
    use rodio::source::UniformSourceIterator;
    use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Source};

    /// Longer files are cut off so a mistaken choice can't fill memory (30 s
    /// of 48 kHz stereo are about 11 MB of samples).
    const MAX_DURATION: Duration = Duration::from_secs(30);

    pub enum Command {
        /// The configured files: decode them now and forget all others.
        Preload(Vec<PathBuf>),
        Play(PathBuf, f32),
    }

    pub struct Backend {
        sender: Sender<Command>,
        device_unavailable: Arc<AtomicBool>,
    }

    impl Backend {
        pub fn spawn() -> Option<Self> {
            let (sender, receiver) = mpsc::channel();
            let device_unavailable = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&device_unavailable);
            let spawned = std::thread::Builder::new()
                .name("rustsplit-sounds".to_string())
                .spawn(move || Worker::default().run(receiver, &flag));
            match spawned {
                Ok(_) => Some(Self {
                    sender,
                    device_unavailable,
                }),
                Err(error) => {
                    log::warn!("could not start the sound thread: {error}");
                    None
                }
            }
        }

        pub fn send(&self, command: Command) {
            let _ = self.sender.send(command);
        }

        pub fn device_unavailable(&self) -> bool {
            self.device_unavailable.load(Ordering::Relaxed)
        }
    }

    enum Device {
        Closed,
        Open(MixerDeviceSink),
        Unavailable,
    }

    struct Cached {
        modified: Option<SystemTime>,
        samples: Option<SamplesBuffer>,
    }

    struct Worker {
        device: Device,
        /// Decoded configured files. Other files (the Settings window's
        /// "Test" button) are decoded for every play instead.
        cache: HashMap<PathBuf, Cached>,
        configured: HashSet<PathBuf>,
    }

    impl Default for Worker {
        fn default() -> Self {
            Self {
                device: Device::Closed,
                cache: HashMap::new(),
                configured: HashSet::new(),
            }
        }
    }

    impl Worker {
        fn run(mut self, receiver: Receiver<Command>, device_unavailable: &AtomicBool) {
            while let Ok(command) = receiver.recv() {
                match command {
                    Command::Preload(paths) => self.preload(paths),
                    Command::Play(path, volume) => {
                        let Some(samples) = self.samples(&path) else {
                            continue;
                        };
                        if let Some(device) = self.device(device_unavailable) {
                            device.mixer().add(samples.amplify(volume));
                        }
                    }
                }
            }
        }

        fn device(&mut self, device_unavailable: &AtomicBool) -> Option<&MixerDeviceSink> {
            if let Device::Closed = self.device {
                self.device = match DeviceSinkBuilder::open_default_sink() {
                    Ok(mut device) => {
                        device.log_on_drop(false);
                        Device::Open(device)
                    }
                    Err(error) => {
                        log::warn!("no audio output device, sounds are disabled: {error}");
                        device_unavailable.store(true, Ordering::Relaxed);
                        Device::Unavailable
                    }
                };
            }
            match &self.device {
                Device::Open(device) => Some(device),
                _ => None,
            }
        }

        fn preload(&mut self, paths: Vec<PathBuf>) {
            self.configured = paths.into_iter().collect();
            let configured = &self.configured;
            self.cache.retain(|path, _| configured.contains(path));
            for path in self.configured.clone() {
                self.samples(&path);
            }
        }

        /// Decoded samples for `path`, decoding it again if the file changed.
        fn samples(&mut self, path: &Path) -> Option<SamplesBuffer> {
            let modified = std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok();
            if let Some(cached) = self.cache.get(path) {
                if cached.modified == modified {
                    return cached.samples.clone();
                }
            }
            let samples = match decode(path) {
                Ok(samples) => Some(samples),
                Err(error) => {
                    log::warn!("could not load sound {}: {error}", path.display());
                    None
                }
            };
            if !self.configured.contains(path) {
                return samples;
            }
            self.cache.insert(
                path.to_path_buf(),
                Cached {
                    modified,
                    samples: samples.clone(),
                },
            );
            samples
        }
    }

    fn decode(path: &Path) -> Result<SamplesBuffer, String> {
        let file = File::open(path).map_err(|error| error.to_string())?;
        let decoder = Decoder::try_from(file).map_err(|error| error.to_string())?;
        let channels = decoder.channels();
        let sample_rate = decoder.sample_rate();
        let source =
            UniformSourceIterator::new(decoder.take_duration(MAX_DURATION), channels, sample_rate);
        Ok(SamplesBuffer::new(
            channels,
            sample_rate,
            source.collect::<Vec<_>>(),
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A mono 16-bit PCM WAV file with `frames` samples at 8 kHz.
        fn wav(frames: u32) -> Vec<u8> {
            let data_len = frames * 2;
            let mut bytes = Vec::new();
            bytes.extend_from_slice(b"RIFF");
            bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
            bytes.extend_from_slice(b"WAVEfmt ");
            bytes.extend_from_slice(&16u32.to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
            bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
            bytes.extend_from_slice(&8000u32.to_le_bytes());
            bytes.extend_from_slice(&16000u32.to_le_bytes());
            bytes.extend_from_slice(&2u16.to_le_bytes());
            bytes.extend_from_slice(&16u16.to_le_bytes());
            bytes.extend_from_slice(b"data");
            bytes.extend_from_slice(&data_len.to_le_bytes());
            for index in 0..frames {
                let sample = if index % 2 == 0 { 8000i16 } else { -8000 };
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            bytes
        }

        #[test]
        fn decodes_and_caches_wav_files() {
            let directory =
                std::env::temp_dir().join(format!("rustsplit-sounds-test-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("split.wav");
            std::fs::write(&path, wav(800)).unwrap();

            let mut worker = Worker::default();
            worker.preload(vec![path.clone()]);
            assert!(worker.cache.contains_key(&path));
            let samples = worker.samples(&path).expect("the WAV file should decode");
            assert_eq!(samples.channels().get(), 1);
            assert_eq!(samples.sample_rate().get(), 8000);
            assert_eq!(samples.count(), 800);

            let missing = directory.join("missing.wav");
            assert!(worker.samples(&missing).is_none());
            let _ = std::fs::remove_dir_all(directory);
        }

        #[test]
        fn only_configured_files_stay_cached() {
            let directory = std::env::temp_dir().join(format!(
                "rustsplit-sounds-evict-test-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let split = directory.join("split.wav");
            let reset = directory.join("reset.wav");
            let test = directory.join("test.wav");
            for path in [&split, &reset, &test] {
                std::fs::write(path, wav(80)).unwrap();
            }

            let mut worker = Worker::default();
            worker.preload(vec![split.clone(), reset.clone()]);
            assert_eq!(worker.cache.len(), 2);
            // A file that is only tested in the Settings window plays but is
            // not kept.
            assert!(worker.samples(&test).is_some());
            assert!(!worker.cache.contains_key(&test));

            worker.preload(vec![reset.clone()]);
            assert!(!worker.cache.contains_key(&split));
            assert!(worker.cache.contains_key(&reset));
            worker.preload(Vec::new());
            assert!(worker.cache.is_empty());
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rustsplit_core::{Run, TestClock, Time, TimeSpan, Timer, BEST_SEGMENTS};

    use super::*;

    fn seconds(value: i64) -> TimeSpan {
        TimeSpan::from_seconds(value)
    }

    fn real(value: i64) -> Time {
        Time::new(Some(seconds(value)), None)
    }

    /// Three segments with a 10/20/30 personal best and 10-second golds.
    fn timer() -> (TestClock, Timer) {
        let mut run = Run::with_segments("Game", "Any%", ["One", "Two", "Three"]);
        for (index, segment) in run.segments.iter_mut().enumerate() {
            segment.set_personal_best(real(10 * (index as i64 + 1)));
            segment.best_segment_time = real(10);
        }
        let clock = TestClock::new();
        let timer = Timer::new(run, Arc::new(clock.clone()));
        (clock, timer)
    }

    fn events(timer: &mut Timer, action: impl FnOnce(&mut Timer)) -> Vec<SoundEvent> {
        let before = Snapshot::capture(timer);
        action(timer);
        sound_events(&before, timer)
    }

    #[test]
    fn start_plays_start_timer() {
        let (_, mut timer) = timer();
        assert_eq!(events(&mut timer, Timer::start), [SoundEvent::StartTimer]);
        // Pausing a stopped timer starts it, too.
        let (_, mut timer) = self::timer();
        assert_eq!(events(&mut timer, Timer::pause), [SoundEvent::StartTimer]);
    }

    #[test]
    fn split_ahead_of_comparison() {
        let (clock, mut timer) = timer();
        timer.start();
        clock.advance(seconds(11));
        assert_eq!(
            events(&mut timer, Timer::split),
            [SoundEvent::SplitBehind, SoundEvent::Split]
        );
        clock.advance(seconds(8));
        assert_eq!(
            events(&mut timer, Timer::split),
            [
                SoundEvent::BestSegment,
                SoundEvent::SplitAhead,
                SoundEvent::Split
            ]
        );
    }

    #[test]
    fn split_ahead_without_best_segment() {
        let (clock, mut timer) = timer();
        timer.run.segments[0].set_personal_best(real(20));
        timer.refresh_comparisons();
        timer.start();
        clock.advance(seconds(15));
        assert_eq!(
            events(&mut timer, Timer::split),
            [SoundEvent::SplitAhead, SoundEvent::Split]
        );
    }

    #[test]
    fn split_behind_comparison() {
        let (clock, mut timer) = timer();
        timer.start();
        clock.advance(seconds(12));
        assert_eq!(
            events(&mut timer, Timer::split),
            [SoundEvent::SplitBehind, SoundEvent::Split]
        );
    }

    #[test]
    fn best_segment_behind_comparison() {
        let (clock, mut timer) = timer();
        timer.run.segments[1].set_personal_best(real(15));
        timer.refresh_comparisons();
        timer.start();
        clock.advance(seconds(12));
        timer.split();
        // An 8-second segment beats the gold but is still behind the PB.
        clock.advance(seconds(8));
        assert_eq!(
            events(&mut timer, Timer::split),
            [
                SoundEvent::BestSegment,
                SoundEvent::SplitBehind,
                SoundEvent::Split
            ]
        );
    }

    #[test]
    fn split_without_comparison_time() {
        let (clock, mut timer) = timer();
        timer.set_current_comparison("Missing");
        timer.start();
        clock.advance(seconds(12));
        assert_eq!(events(&mut timer, Timer::split), [SoundEvent::Split]);

        timer.set_current_comparison(BEST_SEGMENTS);
        clock.advance(seconds(5));
        assert_eq!(
            events(&mut timer, Timer::split),
            [
                SoundEvent::BestSegment,
                SoundEvent::SplitAhead,
                SoundEvent::Split
            ]
        );
    }

    fn finish(clock: &TestClock, timer: &mut Timer, final_time: i64) -> Vec<SoundEvent> {
        timer.start();
        clock.advance(seconds(10));
        timer.split();
        clock.advance(seconds(10));
        timer.split();
        clock.advance(seconds(final_time - 20));
        events(timer, Timer::split)
    }

    #[test]
    fn finishing_with_a_personal_best() {
        let (clock, mut timer) = timer();
        assert_eq!(
            finish(&clock, &mut timer, 29),
            [SoundEvent::PersonalBest, SoundEvent::Split]
        );
        assert_eq!(timer.phase, TimerPhase::Ended);
    }

    #[test]
    fn finishing_without_a_personal_best() {
        let (clock, mut timer) = timer();
        assert_eq!(
            finish(&clock, &mut timer, 30),
            [SoundEvent::NotAPersonalBest, SoundEvent::Split]
        );
    }

    #[test]
    fn first_finish_is_a_personal_best() {
        let (clock, mut timer) = timer();
        timer.run.segments[2].set_personal_best(Time::default());
        timer.refresh_comparisons();
        assert_eq!(
            finish(&clock, &mut timer, 45),
            [SoundEvent::PersonalBest, SoundEvent::Split]
        );
    }

    #[test]
    fn undo_and_skip() {
        let (clock, mut timer) = timer();
        timer.start();
        clock.advance(seconds(10));
        timer.split();
        assert_eq!(
            events(&mut timer, Timer::skip_split),
            [SoundEvent::SkipSplit]
        );
        assert_eq!(
            events(&mut timer, Timer::undo_split),
            [SoundEvent::UndoSplit]
        );
        assert_eq!(
            events(&mut timer, Timer::undo_split),
            [SoundEvent::UndoSplit]
        );
        // Nothing left to undo.
        assert!(events(&mut timer, Timer::undo_split).is_empty());
    }

    #[test]
    fn undo_after_finish() {
        let (clock, mut timer) = timer();
        finish(&clock, &mut timer, 40);
        assert_eq!(
            events(&mut timer, Timer::undo_split),
            [SoundEvent::UndoSplit]
        );
        assert_eq!(timer.phase, TimerPhase::Running);
    }

    #[test]
    fn pause_and_resume() {
        let (clock, mut timer) = timer();
        timer.start();
        clock.advance(seconds(3));
        assert_eq!(events(&mut timer, Timer::pause), [SoundEvent::Pause]);
        // Splitting is ignored while paused.
        assert!(events(&mut timer, Timer::split).is_empty());
        assert_eq!(events(&mut timer, Timer::pause), [SoundEvent::Resume]);
        timer.pause();
        assert_eq!(
            events(&mut timer, Timer::undo_all_pauses),
            [SoundEvent::Resume]
        );
    }

    #[test]
    fn reset() {
        let (clock, mut timer) = timer();
        timer.start();
        clock.advance(seconds(3));
        assert_eq!(events(&mut timer, Timer::reset), [SoundEvent::Reset]);
        // Resetting a stopped timer does nothing.
        assert!(events(&mut timer, Timer::reset).is_empty());

        timer.start();
        timer.pause();
        assert_eq!(
            events(&mut timer, Timer::reset_and_set_attempt_as_pb),
            [SoundEvent::Reset]
        );
    }

    #[test]
    fn reset_after_finish_is_silent() {
        let (clock, mut timer) = timer();
        finish(&clock, &mut timer, 40);
        assert!(events(&mut timer, Timer::reset).is_empty());
    }

    #[test]
    fn unrelated_changes_are_silent() {
        let (clock, mut timer) = timer();
        assert!(events(&mut timer, Timer::toggle_timing_method).is_empty());
        timer.start();
        clock.advance(seconds(3));
        assert!(events(&mut timer, Timer::switch_comparison_next).is_empty());
        assert!(events(&mut timer, |timer| timer.set_game_time(Some(seconds(1)))).is_empty());
    }

    #[test]
    fn configured_path_falls_back_to_split() {
        let config = SoundConfig {
            split: Some(PathBuf::from("split.wav")),
            split_behind: Some(PathBuf::new()),
            ..SoundConfig::default()
        };
        let candidates = [
            SoundEvent::BestSegment,
            SoundEvent::SplitBehind,
            SoundEvent::Split,
        ];
        let path = candidates.into_iter().find_map(|event| event.path(&config));
        assert_eq!(path, Some(Path::new("split.wav")));
        assert_eq!(SoundEvent::Reset.path(&config), None);
    }
}
