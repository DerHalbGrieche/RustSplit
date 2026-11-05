//! World records from the speedrun.com REST API for the World Record
//! component.
//!
//! Lookups only happen when the user enabled them in the settings and the
//! layout contains a World Record component. Requests run on a background
//! thread; the UI thread only polls a channel and reads an in-memory cache.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustsplit_core::{TimeSpan, Timer, TimingMethod};
use rustsplit_ui::components::{Component, TimingMethodSetting, WorldRecordSettings};
use rustsplit_ui::{Layout, WorldRecord};
use serde::Deserialize;

const API_BASE: &str = "https://www.speedrun.com/api/v1";
const USER_AGENT: &str = concat!("RustSplit/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a fetched record is shown before it is refreshed.
const RECORD_TTL: Duration = Duration::from_secs(60 * 60);
/// Delay before retrying after a network or API error.
const ERROR_RETRY: Duration = Duration::from_secs(5 * 60);
/// Delay before retrying after a rate limit without a `Retry-After` header.
const RATE_LIMIT_RETRY: Duration = Duration::from_secs(60);
/// Upper bound for a server-provided `Retry-After`.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// Everything that determines which record is shown. A change to any field
/// triggers a new lookup.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Query {
    game: String,
    category: String,
    run_id: Option<String>,
    /// `None` follows the leaderboard's own default timing, like LiveSplit.
    timing_method: Option<TimingMethod>,
    platform: Option<String>,
    emulated: Option<bool>,
    region: Option<String>,
    /// Speedrun.com variables (name -> value label) from the run metadata.
    variables: BTreeMap<String, String>,
    subcategories: bool,
    other_variables: bool,
}

impl Query {
    /// The query for the first World Record component in `layout`, if any.
    ///
    /// Only that component drives the lookup. A layout with several World
    /// Record components fetches one record, and all of them show it with
    /// the timing this lookup picked (by default the leaderboard's own),
    /// whatever their own timing method or filters say.
    pub fn for_layout(layout: &Layout, timer: &Timer) -> Option<Self> {
        layout
            .components
            .iter()
            .find_map(|component| match &component.component {
                Component::WorldRecord(settings) => Some(settings),
                _ => None,
            })
            .and_then(|settings| Self::new(timer, settings))
    }

    fn new(timer: &Timer, settings: &WorldRecordSettings) -> Option<Self> {
        let run = &timer.run;
        let metadata = &run.metadata;
        let game = run.game_name.trim();
        let category = run.category_name.trim();
        if game.is_empty() || category.is_empty() {
            return None;
        }
        let timing_method = match settings.timing_method {
            // The world record is the leaderboard's record, so "current" does
            // not re-rank the board by the timer's method: a game ranked by
            // in-game time would otherwise show a slower real-time "record".
            TimingMethodSetting::Current => None,
            TimingMethodSetting::RealTime => Some(TimingMethod::RealTime),
            TimingMethodSetting::GameTime => Some(TimingMethod::GameTime),
        };
        let filter_platform = filter_enabled(&settings.filter_platform);
        let filter_region = filter_enabled(&settings.filter_region);
        let subcategories = settings.filter_subcategories;
        let other_variables = settings.filter_variables;
        Some(Self {
            game: game.to_string(),
            category: category.to_string(),
            run_id: metadata.run_id.clone(),
            timing_method,
            platform: metadata.platform_name.clone().filter(|_| filter_platform),
            emulated: filter_platform.then_some(metadata.uses_emulator),
            region: metadata.region_name.clone().filter(|_| filter_region),
            variables: if subcategories || other_variables {
                metadata.variables.clone()
            } else {
                BTreeMap::new()
            },
            subcategories,
            other_variables,
        })
    }
}

/// Mirrors the layout's filter strings, which LiveSplit stores as booleans.
fn filter_enabled(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && !value.eq_ignore_ascii_case("false") && value != "0"
}

// ---------------------------------------------------------------------------
// UI-side service
// ---------------------------------------------------------------------------

type Response = (Query, Result<Option<WorldRecord>, FetchError>);

struct Worker {
    requests: Sender<Query>,
    responses: Receiver<Response>,
}

struct Entry {
    record: Option<WorldRecord>,
    refresh_at: Instant,
}

/// Owns the background worker and the in-memory cache. Call [`update`] once
/// per frame; it never blocks.
///
/// [`update`]: WorldRecordService::update
#[derive(Default)]
pub struct WorldRecordService {
    worker: Option<Worker>,
    entries: HashMap<Query, Entry>,
    pending: Option<Query>,
}

impl WorldRecordService {
    /// The record for the current run and layout, or `None` when disabled,
    /// not fetched yet or unavailable. Starts a background lookup when the
    /// cached value is missing or stale.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        enabled: bool,
        layout: &Layout,
        timer: &Timer,
    ) -> Option<WorldRecord> {
        self.poll();
        if !enabled {
            return None;
        }
        let query = Query::for_layout(layout, timer)?;
        let now = Instant::now();
        let stale = self
            .entries
            .get(&query)
            .is_none_or(|entry| now >= entry.refresh_at);
        if stale && self.pending.is_none() {
            self.request(ctx, query.clone(), now);
        }
        self.entries
            .get(&query)
            .and_then(|entry| entry.record.clone())
    }

    fn request(&mut self, ctx: &egui::Context, query: Query, now: Instant) {
        if self.worker.is_none() {
            match spawn_worker(ctx.clone()) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    log::warn!("could not start the speedrun.com worker: {error}");
                    self.retry_later(query, now + ERROR_RETRY);
                    return;
                }
            }
        }
        let sent = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.requests.send(query.clone()).is_ok());
        if sent {
            self.pending = Some(query);
        } else {
            log::warn!("the speedrun.com worker stopped");
            self.worker = None;
            self.retry_later(query, now + ERROR_RETRY);
        }
    }

    fn poll(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        let mut finished = Vec::new();
        let disconnected = loop {
            match worker.responses.try_recv() {
                Ok(response) => finished.push(response),
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if disconnected {
            // The worker thread died (e.g. a panic). Start a new one for the
            // next lookup instead of waiting for an answer forever.
            log::warn!("the speedrun.com worker stopped");
            self.worker = None;
            if let Some(query) = self.pending.take() {
                self.retry_later(query, retry_at(Instant::now(), ERROR_RETRY));
            }
        }
        for (query, result) in finished {
            if self.pending.as_ref() == Some(&query) {
                self.pending = None;
            }
            let now = Instant::now();
            match result {
                Ok(record) => {
                    self.entries.insert(
                        query,
                        Entry {
                            record,
                            refresh_at: now + RECORD_TTL,
                        },
                    );
                }
                Err(error) => {
                    log::warn!("speedrun.com world record lookup failed: {error}");
                    self.retry_later(query, retry_at(now, error.retry_delay()));
                }
            }
        }
    }

    /// Keeps any cached record and schedules the next attempt.
    fn retry_later(&mut self, query: Query, at: Instant) {
        self.entries
            .entry(query)
            .and_modify(|entry| entry.refresh_at = at)
            .or_insert(Entry {
                record: None,
                refresh_at: at,
            });
    }
}

/// `now + delay`, saturating far in the future instead of overflowing.
fn retry_at(now: Instant, delay: Duration) -> Instant {
    now.checked_add(delay.min(MAX_RETRY_AFTER)).unwrap_or(now)
}

fn spawn_worker(ctx: egui::Context) -> std::io::Result<Worker> {
    let (request_sender, requests) = mpsc::channel::<Query>();
    let (response_sender, responses) = mpsc::channel();
    std::thread::Builder::new()
        .name("speedrun-com".to_string())
        .spawn(move || {
            let mut client = Client::new(UreqTransport::new());
            while let Ok(query) = requests.recv() {
                let result = client.world_record(&query);
                if response_sender.send((query, result)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        })?;
    Ok(Worker {
        requests: request_sender,
        responses,
    })
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum FetchError {
    RateLimited(Option<Duration>),
    Status(u16, String),
    Transport(String),
    Parse(String, serde_json::Error),
}

impl FetchError {
    fn retry_delay(&self) -> Duration {
        match self {
            Self::RateLimited(delay) => delay.unwrap_or(RATE_LIMIT_RETRY),
            _ => ERROR_RETRY,
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited(_) => write!(f, "rate limited by speedrun.com"),
            Self::Status(status, url) => write!(f, "HTTP {status} from {url}"),
            Self::Transport(error) => write!(f, "{error}"),
            Self::Parse(url, error) => write!(f, "unexpected response from {url}: {error}"),
        }
    }
}

trait Transport {
    fn get(&mut self, url: &str) -> Result<String, FetchError>;
}

struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
    }
}

impl Transport for UreqTransport {
    fn get(&mut self, url: &str) -> Result<String, FetchError> {
        log::debug!("GET {url}");
        let mut response = self
            .agent
            .get(url)
            .header("Accept", "application/json")
            .call()
            .map_err(|error| FetchError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        // speedrun.com answers 420 when the client exceeds its rate limit.
        if status == 420 || status == 429 {
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(parse_retry_after);
            return Err(FetchError::RateLimited(delay));
        }
        if !(200..300).contains(&status) {
            return Err(FetchError::Status(status, url.to_string()));
        }
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| FetchError::Transport(error.to_string()))
    }
}

/// The delay of a `Retry-After: <seconds>` header, at most an hour.
fn parse_retry_after(value: &str) -> Option<Duration> {
    let seconds: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

/// Resolves games and categories and fetches leaderboards. Game data is
/// cached for the lifetime of the worker.
struct Client<T> {
    transport: T,
    game_ids: HashMap<String, String>,
    games: HashMap<String, Arc<Game>>,
    runs: HashMap<String, RunData>,
}

impl<T: Transport> Client<T> {
    fn new(transport: T) -> Self {
        Self {
            transport,
            game_ids: HashMap::new(),
            games: HashMap::new(),
            runs: HashMap::new(),
        }
    }

    fn world_record(&mut self, query: &Query) -> Result<Option<WorldRecord>, FetchError> {
        let Some((game, category_id)) = self.resolve(query)? else {
            log::info!(
                "no speedrun.com leaderboard found for {} - {}",
                query.game,
                query.category
            );
            return Ok(None);
        };
        let timing = select_timing(query.timing_method, &game.ruleset);
        let filters = leaderboard_filters(query, &game, &category_id);
        let url = leaderboard_url(&game.id, &category_id, timing, &filters);
        let board: Envelope<Leaderboard> = self.get_json(&url)?;
        Ok(world_record_from_leaderboard(
            &board.data,
            timing,
            game.ruleset.show_milliseconds,
        ))
    }

    /// Finds the game and full-game category. A stored run id is used when it
    /// agrees with the run's game and category names, or when the names do
    /// not match a speedrun.com leaderboard.
    fn resolve(&mut self, query: &Query) -> Result<Option<(Arc<Game>, String)>, FetchError> {
        let mut from_run = None;
        if let Some(run_id) = &query.run_id {
            match self.run(run_id) {
                Ok(Some(run)) => {
                    let game = self.game(&run.game)?;
                    let consistent = game.has_name(&query.game)
                        && game
                            .category_by_id(&run.category)
                            .is_some_and(|category| same_name(&category.name, &query.category));
                    if consistent {
                        return Ok(Some((game, run.category)));
                    }
                    from_run = Some((game, run.category));
                }
                Ok(None) => {}
                Err(FetchError::Status(404, _)) => {
                    log::info!("speedrun.com run {run_id} no longer exists");
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(game_id) = self.find_game_id(&query.game)? {
            let game = self.game(&game_id)?;
            if let Some(category) = game.category_by_name(&query.category) {
                let category_id = category.id.clone();
                return Ok(Some((game, category_id)));
            }
        }
        Ok(from_run)
    }

    fn run(&mut self, run_id: &str) -> Result<Option<RunData>, FetchError> {
        if let Some(run) = self.runs.get(run_id) {
            return Ok(Some(run.clone()));
        }
        let url = format!("{API_BASE}/runs/{}", encode(run_id));
        let run: Envelope<RunData> = self.get_json(&url)?;
        // Individual-level runs do not identify a full-game leaderboard.
        if run.data.level.is_some() {
            return Ok(None);
        }
        self.runs.insert(run_id.to_string(), run.data.clone());
        Ok(Some(run.data))
    }

    fn find_game_id(&mut self, name: &str) -> Result<Option<String>, FetchError> {
        let key = name.to_lowercase();
        if let Some(id) = self.game_ids.get(&key) {
            return Ok(Some(id.clone()));
        }
        let url = game_search_url(name);
        let games: Envelope<Vec<GameSummary>> = self.get_json(&url)?;
        let Some(id) = pick_game(&games.data, name) else {
            return Ok(None);
        };
        self.game_ids.insert(key, id.clone());
        Ok(Some(id))
    }

    fn game(&mut self, id: &str) -> Result<Arc<Game>, FetchError> {
        if let Some(game) = self.games.get(id) {
            return Ok(game.clone());
        }
        let url = game_url(id);
        let game: Envelope<Game> = self.get_json(&url)?;
        let game = Arc::new(game.data);
        self.games.insert(id.to_string(), game.clone());
        Ok(game)
    }

    fn get_json<D: for<'de> Deserialize<'de>>(&mut self, url: &str) -> Result<D, FetchError> {
        let body = self.transport.get(url)?;
        serde_json::from_str(&body).map_err(|error| FetchError::Parse(url.to_string(), error))
    }
}

// ---------------------------------------------------------------------------
// URLs, filters and timing
// ---------------------------------------------------------------------------

fn game_search_url(name: &str) -> String {
    format!("{API_BASE}/games?name={}&max=20", encode(name))
}

fn game_url(id: &str) -> String {
    format!(
        "{API_BASE}/games/{}?embed=categories,variables,platforms,regions",
        encode(id)
    )
}

fn leaderboard_url(
    game: &str,
    category: &str,
    timing: &str,
    filters: &[(String, String)],
) -> String {
    let mut url = format!(
        "{API_BASE}/leaderboards/{}/category/{}?top=1&embed=players",
        encode(game),
        encode(category)
    );
    if !timing.is_empty() {
        url.push_str("&timing=");
        url.push_str(&encode(timing));
    }
    for (key, value) in filters {
        url.push('&');
        url.push_str(&encode(key));
        url.push('=');
        url.push_str(&encode(value));
    }
    url
}

/// Leaderboard query parameters for the component's filters. Names that the
/// game does not know are skipped rather than producing an empty leaderboard.
fn leaderboard_filters(query: &Query, game: &Game, category_id: &str) -> Vec<(String, String)> {
    let mut filters = Vec::new();
    if let Some(name) = &query.platform {
        match game
            .platforms
            .data
            .iter()
            .find(|p| same_name(&p.name, name))
        {
            Some(platform) => filters.push(("platform".to_string(), platform.id.clone())),
            None => log::debug!("speedrun.com platform {name:?} not found"),
        }
    }
    if let Some(emulated) = query.emulated {
        filters.push(("emulators".to_string(), emulated.to_string()));
    }
    if let Some(name) = &query.region {
        match game.regions.data.iter().find(|r| same_name(&r.name, name)) {
            Some(region) => filters.push(("region".to_string(), region.id.clone())),
            None => log::debug!("speedrun.com region {name:?} not found"),
        }
    }
    for (name, label) in &query.variables {
        let Some(variable) =
            game.variables.data.iter().find(|variable| {
                same_name(&variable.name, name) && variable.applies_to(category_id)
            })
        else {
            continue;
        };
        let wanted = if variable.is_subcategory {
            query.subcategories
        } else {
            query.other_variables
        };
        if !wanted {
            continue;
        }
        if let Some((value_id, _)) = variable
            .values
            .values
            .iter()
            .find(|(_, value)| same_name(&value.label, label))
        {
            filters.push((format!("var-{}", variable.id), value_id.clone()));
        }
    }
    filters
}

/// The speedrun.com timing for `method`: real time when the game tracks it,
/// and in-game or load-removed time for game time. Falls back to the game's
/// default timing.
fn select_timing(method: Option<TimingMethod>, ruleset: &Ruleset) -> &str {
    let tracked = |name: &str| ruleset.run_times.iter().any(|time| time == name);
    let default = ruleset.default_time.as_str();
    match method {
        Some(TimingMethod::RealTime) if tracked("realtime") => "realtime",
        Some(TimingMethod::GameTime) if matches!(default, "ingame" | "realtime_noloads") => default,
        Some(TimingMethod::GameTime) if tracked("ingame") => "ingame",
        Some(TimingMethod::GameTime) if tracked("realtime_noloads") => "realtime_noloads",
        _ => default,
    }
}

fn world_record_from_leaderboard(
    board: &Leaderboard,
    timing: &str,
    show_milliseconds: bool,
) -> Option<WorldRecord> {
    let leaders: Vec<_> = board
        .runs
        .iter()
        .filter(|placed| placed.place == 1)
        .map(|placed| &placed.run)
        .collect();
    let first = leaders.first()?;
    let time = first.times.get(timing).or(first.times.primary_t)?;
    let runners = leaders
        .iter()
        .map(|run| {
            run.players
                .iter()
                .filter_map(|player| board.player_name(player))
                .collect::<Vec<_>>()
                .join(" & ")
        })
        .filter(|names| !names.is_empty())
        .collect();
    Some(WorldRecord {
        time: TimeSpan::from_millis((time * 1000.0).round() as i64),
        runners,
        show_milliseconds,
    })
}

fn pick_game(games: &[GameSummary], name: &str) -> Option<String> {
    games
        .iter()
        .find(|game| game.names.matches(name) || same_name(&game.abbreviation, name))
        .or(match games {
            [only] => Some(only),
            _ => None,
        })
        .map(|game| game.id.clone())
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Percent-encodes a URL path segment or query value.
fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

// ---------------------------------------------------------------------------
// API types (only the fields RustSplit uses)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Deserialize)]
struct Embedded<T> {
    data: Vec<T>,
}

impl<T> Default for Embedded<T> {
    fn default() -> Self {
        Self { data: Vec::new() }
    }
}

#[derive(Deserialize)]
struct Names {
    international: String,
    japanese: Option<String>,
    twitch: Option<String>,
}

impl Names {
    fn matches(&self, name: &str) -> bool {
        same_name(&self.international, name)
            || [&self.japanese, &self.twitch]
                .into_iter()
                .flatten()
                .any(|alias| same_name(alias, name))
    }
}

#[derive(Deserialize)]
struct GameSummary {
    id: String,
    names: Names,
    #[serde(default)]
    abbreviation: String,
}

#[derive(Deserialize)]
struct Game {
    id: String,
    names: Names,
    #[serde(default)]
    abbreviation: String,
    ruleset: Ruleset,
    #[serde(default)]
    categories: Embedded<Category>,
    #[serde(default)]
    variables: Embedded<Variable>,
    #[serde(default)]
    platforms: Embedded<NamedItem>,
    #[serde(default)]
    regions: Embedded<NamedItem>,
}

impl Game {
    fn has_name(&self, name: &str) -> bool {
        self.names.matches(name) || same_name(&self.abbreviation, name)
    }

    fn category_by_id(&self, id: &str) -> Option<&Category> {
        self.categories
            .data
            .iter()
            .find(|category| category.id == id)
    }

    fn category_by_name(&self, name: &str) -> Option<&Category> {
        self.categories
            .data
            .iter()
            .find(|category| category.kind == "per-game" && same_name(&category.name, name))
    }
}

#[derive(Deserialize)]
struct Ruleset {
    #[serde(rename = "show-milliseconds", default)]
    show_milliseconds: bool,
    #[serde(rename = "run-times", default)]
    run_times: Vec<String>,
    #[serde(rename = "default-time", default)]
    default_time: String,
}

#[derive(Deserialize)]
struct Category {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct NamedItem {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct Variable {
    id: String,
    name: String,
    category: Option<String>,
    scope: Scope,
    #[serde(rename = "is-subcategory", default)]
    is_subcategory: bool,
    values: VariableValues,
}

impl Variable {
    fn applies_to(&self, category_id: &str) -> bool {
        matches!(self.scope.kind.as_str(), "global" | "full-game")
            && self
                .category
                .as_deref()
                .is_none_or(|category| category == category_id)
    }
}

#[derive(Deserialize)]
struct Scope {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct VariableValues {
    values: BTreeMap<String, VariableValue>,
}

#[derive(Deserialize)]
struct VariableValue {
    label: String,
}

#[derive(Clone, Deserialize)]
struct RunData {
    game: String,
    category: String,
    level: Option<String>,
}

#[derive(Deserialize)]
struct Leaderboard {
    runs: Vec<PlacedRun>,
    #[serde(default)]
    players: Embedded<Player>,
}

impl Leaderboard {
    fn player_name(&self, player: &PlayerRef) -> Option<String> {
        match player.rel.as_str() {
            "user" => {
                let id = player.id.as_deref()?;
                let name = self
                    .players
                    .data
                    .iter()
                    .find(|user| user.id.as_deref() == Some(id))
                    .and_then(|user| user.names.as_ref())
                    .map_or(id, |names| names.international.as_str());
                Some(name.to_string())
            }
            "guest" => player.name.clone(),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct PlacedRun {
    place: u32,
    run: LeaderboardRun,
}

#[derive(Deserialize)]
struct LeaderboardRun {
    players: Vec<PlayerRef>,
    times: Times,
}

/// A player reference in a run: `{"rel": "user", "id": ...}` or
/// `{"rel": "guest", "name": ...}`.
#[derive(Deserialize)]
struct PlayerRef {
    rel: String,
    id: Option<String>,
    name: Option<String>,
}

/// An embedded player. Users have `names`, guests only a `name`.
#[derive(Deserialize)]
struct Player {
    id: Option<String>,
    names: Option<Names>,
}

/// Times in seconds. speedrun.com reports 0 for timings a run lacks.
#[derive(Deserialize)]
struct Times {
    primary_t: Option<f64>,
    realtime_t: Option<f64>,
    realtime_noloads_t: Option<f64>,
    ingame_t: Option<f64>,
}

impl Times {
    fn get(&self, timing: &str) -> Option<f64> {
        match timing {
            "realtime" => self.realtime_t,
            "realtime_noloads" => self.realtime_noloads_t,
            "ingame" => self.ingame_t,
            _ => self.primary_t,
        }
        .filter(|seconds| *seconds > 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustsplit_core::Run;
    use rustsplit_ui::LayoutComponent;

    const GAMES: &str = include_str!("../tests/fixtures/speedrun_com/games.json");
    const GAME: &str = include_str!("../tests/fixtures/speedrun_com/game.json");
    const LEADERBOARD: &str = include_str!("../tests/fixtures/speedrun_com/leaderboard.json");
    const RUN: &str = include_str!("../tests/fixtures/speedrun_com/run.json");

    const SMS: &str = "v1pxjz68";
    const ANY: &str = "n2y3r8do";

    /// Serves fixtures by URL and records every request.
    #[derive(Default)]
    struct FakeTransport {
        responses: HashMap<String, &'static str>,
        requests: Vec<String>,
    }

    impl Transport for FakeTransport {
        fn get(&mut self, url: &str) -> Result<String, FetchError> {
            self.requests.push(url.to_string());
            self.responses
                .get(url)
                .map(|body| body.to_string())
                .ok_or_else(|| FetchError::Status(404, url.to_string()))
        }
    }

    fn game() -> Game {
        serde_json::from_str::<Envelope<Game>>(GAME).unwrap().data
    }

    fn sms_query() -> Query {
        Query {
            game: "Super Mario Sunshine".to_string(),
            category: "Any%".to_string(),
            run_id: None,
            timing_method: Some(TimingMethod::RealTime),
            platform: None,
            emulated: None,
            region: None,
            variables: BTreeMap::from([
                ("Game Version".to_string(), "GCN".to_string()),
                ("Arbitrary Code Execution".to_string(), "No ACE".to_string()),
                ("Timing Method".to_string(), "Hacked File".to_string()),
            ]),
            subcategories: true,
            other_variables: false,
        }
    }

    fn sms_leaderboard_url() -> String {
        format!(
            "{API_BASE}/leaderboards/{SMS}/category/{ANY}?top=1&embed=players&timing=realtime\
             &var-ylqxgz78=1gn4rz8l&var-j845md4n=gq7on0vl"
        )
    }

    #[test]
    fn parses_recorded_game() {
        let game = game();
        assert_eq!(game.id, SMS);
        assert!(game.has_name("super mario sunshine"));
        assert!(game.has_name("SMS"));
        assert!(game.ruleset.show_milliseconds);
        assert_eq!(game.ruleset.default_time, "realtime");
        assert_eq!(game.category_by_name("any%").unwrap().id, ANY);
        assert!(game.category_by_name("Any% (No ACE)").is_none());
        assert_eq!(game.variables.data.len(), 3);
        assert_eq!(game.platforms.data[0].name, "GameCube");
        assert_eq!(game.regions.data[2].name, "JPN / NTSC");
    }

    #[test]
    fn parses_recorded_leaderboard() {
        let board: Envelope<Leaderboard> = serde_json::from_str(LEADERBOARD).unwrap();
        let record = world_record_from_leaderboard(&board.data, "realtime", true).unwrap();
        assert_eq!(record.time, TimeSpan::from_seconds(4326));
        assert_eq!(record.runners, vec!["inkstar".to_string()]);
        assert!(record.show_milliseconds);
    }

    #[test]
    fn parses_ties_guests_and_missing_timings() {
        let json = r#"{"data": {"runs": [
            {"place": 1, "run": {"players": [
                {"rel": "user", "id": "u1", "uri": "..."},
                {"rel": "guest", "name": "Guest", "uri": "..."}
            ], "times": {"primary_t": 61.5, "realtime_t": 61.5, "ingame_t": 0}}},
            {"place": 1, "run": {"players": [{"rel": "user", "id": "u2"}],
                "times": {"primary_t": 61.5, "realtime_t": 61.5, "ingame_t": 0}}},
            {"place": 3, "run": {"players": [{"rel": "user", "id": "u3"}],
                "times": {"primary_t": 70, "realtime_t": 70, "ingame_t": 0}}}
        ], "players": {"data": [
            {"rel": "user", "id": "u1", "names": {"international": "Alice", "japanese": null}},
            {"rel": "guest", "name": "Guest"}
        ]}}}"#;
        let board: Envelope<Leaderboard> = serde_json::from_str(json).unwrap();
        // The in-game time is missing, so the primary time is used. Users that
        // are not embedded fall back to their id.
        let record = world_record_from_leaderboard(&board.data, "ingame", false).unwrap();
        assert_eq!(record.time, TimeSpan::from_millis(61_500));
        assert_eq!(
            record.runners,
            vec!["Alice & Guest".to_string(), "u2".to_string()]
        );

        let empty: Envelope<Leaderboard> =
            serde_json::from_str(r#"{"data": {"runs": []}}"#).unwrap();
        assert_eq!(
            world_record_from_leaderboard(&empty.data, "realtime", true),
            None
        );
    }

    #[test]
    fn selects_timing_method() {
        let ruleset = |run_times: &[&str], default: &str| Ruleset {
            show_milliseconds: false,
            run_times: run_times.iter().map(|time| time.to_string()).collect(),
            default_time: default.to_string(),
        };
        let rta_only = ruleset(&["realtime"], "realtime");
        assert_eq!(
            select_timing(Some(TimingMethod::RealTime), &rta_only),
            "realtime"
        );
        assert_eq!(
            select_timing(Some(TimingMethod::GameTime), &rta_only),
            "realtime"
        );

        let igt = ruleset(&["realtime", "ingame"], "ingame");
        assert_eq!(
            select_timing(Some(TimingMethod::RealTime), &igt),
            "realtime"
        );
        assert_eq!(select_timing(Some(TimingMethod::GameTime), &igt), "ingame");

        let load_removed = ruleset(
            &["realtime", "realtime_noloads", "ingame"],
            "realtime_noloads",
        );
        assert_eq!(
            select_timing(Some(TimingMethod::GameTime), &load_removed),
            "realtime_noloads"
        );

        let rta_default = ruleset(&["realtime", "realtime_noloads"], "realtime");
        assert_eq!(
            select_timing(Some(TimingMethod::GameTime), &rta_default),
            "realtime_noloads"
        );

        assert_eq!(select_timing(None, &igt), "ingame");
        assert_eq!(select_timing(None, &rta_default), "realtime");

        let igt_only = ruleset(&["ingame"], "ingame");
        assert_eq!(
            select_timing(Some(TimingMethod::RealTime), &igt_only),
            "ingame"
        );
    }

    #[test]
    fn builds_filters() {
        let game = game();
        let mut query = sms_query();
        assert_eq!(
            leaderboard_filters(&query, &game, ANY),
            vec![
                ("var-ylqxgz78".to_string(), "1gn4rz8l".to_string()),
                ("var-j845md4n".to_string(), "gq7on0vl".to_string()),
            ]
        );

        // Category-specific variables only apply to their category.
        assert_eq!(
            leaderboard_filters(&query, &game, "z27o9gd0"),
            vec![("var-j845md4n".to_string(), "gq7on0vl".to_string())]
        );

        query.subcategories = false;
        query.other_variables = true;
        query.platform = Some("gamecube".to_string());
        query.emulated = Some(false);
        query.region = Some("JPN / NTSC".to_string());
        assert_eq!(
            leaderboard_filters(&query, &game, ANY),
            vec![
                ("platform".to_string(), "4p9z06rn".to_string()),
                ("emulators".to_string(), "false".to_string()),
                ("region".to_string(), "o316x197".to_string()),
                ("var-2lggwk6l".to_string(), "21gok6xq".to_string()),
            ]
        );

        // Unknown names are ignored instead of emptying the leaderboard.
        query.platform = Some("Dreamcast".to_string());
        query.region = None;
        query.variables = BTreeMap::from([("Timing Method".to_string(), "Nope".to_string())]);
        assert_eq!(
            leaderboard_filters(&query, &game, ANY),
            vec![("emulators".to_string(), "false".to_string())]
        );
    }

    #[test]
    fn builds_urls() {
        assert_eq!(
            game_search_url("Pokémon Red/Blue"),
            format!("{API_BASE}/games?name=Pok%C3%A9mon%20Red%2FBlue&max=20")
        );
        assert_eq!(
            game_url(SMS),
            format!("{API_BASE}/games/{SMS}?embed=categories,variables,platforms,regions")
        );
        assert_eq!(
            leaderboard_url(
                SMS,
                ANY,
                "realtime",
                &[("var-ylqxgz78".to_string(), "1gn4rz8l".to_string())]
            ),
            format!(
                "{API_BASE}/leaderboards/{SMS}/category/{ANY}?top=1&embed=players\
                 &timing=realtime&var-ylqxgz78=1gn4rz8l"
            )
        );
        assert_eq!(
            leaderboard_url(SMS, ANY, "", &[]),
            format!("{API_BASE}/leaderboards/{SMS}/category/{ANY}?top=1&embed=players")
        );
        assert!(USER_AGENT.starts_with("RustSplit/"));
    }

    #[test]
    fn fetches_by_game_and_category_name() {
        let mut transport = FakeTransport::default();
        transport
            .responses
            .insert(game_search_url("Super Mario Sunshine"), GAMES);
        transport.responses.insert(game_url(SMS), GAME);
        transport
            .responses
            .insert(sms_leaderboard_url(), LEADERBOARD);
        let mut client = Client::new(transport);

        let record = client.world_record(&sms_query()).unwrap().unwrap();
        assert_eq!(record.time, TimeSpan::from_seconds(4326));
        assert_eq!(record.runners, vec!["inkstar".to_string()]);

        // Game data is cached; only the leaderboard is fetched again.
        client.world_record(&sms_query()).unwrap();
        assert_eq!(client.transport.requests.len(), 4);
        assert_eq!(client.transport.requests[3], sms_leaderboard_url());
    }

    #[test]
    fn fetches_by_run_id() {
        let mut transport = FakeTransport::default();
        transport
            .responses
            .insert(format!("{API_BASE}/runs/zxrpn9kz"), RUN);
        transport
            .responses
            .insert(game_search_url("Sunshine"), r#"{"data": []}"#);
        transport.responses.insert(game_url(SMS), GAME);
        transport
            .responses
            .insert(sms_leaderboard_url(), LEADERBOARD);
        let mut client = Client::new(transport);

        let mut query = sms_query();
        query.run_id = Some("zxrpn9kz".to_string());
        // The game is named differently in the splits, so the name search
        // fails and the run id identifies the leaderboard.
        query.game = "Sunshine".to_string();
        let record = client.world_record(&query).unwrap().unwrap();
        assert_eq!(record.runners, vec!["inkstar".to_string()]);
    }

    #[test]
    fn unknown_game_has_no_record() {
        let mut transport = FakeTransport::default();
        transport
            .responses
            .insert(game_search_url("Super Mario"), GAMES);
        let mut client = Client::new(transport);
        let mut query = sms_query();
        query.game = "Super Mario".to_string();
        assert_eq!(client.world_record(&query).unwrap(), None);
    }

    #[test]
    fn picks_exact_game_match() {
        let games: Envelope<Vec<GameSummary>> = serde_json::from_str(GAMES).unwrap();
        assert_eq!(
            pick_game(&games.data, "super mario sunshine").as_deref(),
            Some(SMS)
        );
        assert_eq!(pick_game(&games.data, "sms64").as_deref(), Some("pdv4n4dw"));
        assert_eq!(pick_game(&games.data, "Sunshine"), None);
        assert_eq!(
            pick_game(&games.data[..1], "Sunshine").as_deref(),
            Some(SMS)
        );
    }

    #[test]
    fn query_requires_a_world_record_component() {
        let mut run = Run::new("Super Mario Sunshine", "Any%");
        run.metadata.platform_name = Some("GameCube".to_string());
        run.metadata.uses_emulator = true;
        run.metadata
            .variables
            .insert("Game Version".to_string(), "GCN".to_string());
        let mut timer = Timer::with_system_clock(run);
        timer.current_timing_method = TimingMethod::GameTime;

        let mut layout = Layout::default();
        assert_eq!(Query::for_layout(&layout, &timer), None);

        let settings = WorldRecordSettings {
            filter_platform: "true".to_string(),
            ..WorldRecordSettings::default()
        };
        layout.components.push(LayoutComponent {
            path: "LiveSplit.WorldRecord.dll".to_string(),
            component: Component::WorldRecord(settings),
            font_overrides: Default::default(),
        });
        let query = Query::for_layout(&layout, &timer).unwrap();
        // "Current timing method" follows the leaderboard, not the timer.
        assert_eq!(query.timing_method, None);
        assert_eq!(query.platform.as_deref(), Some("GameCube"));
        assert_eq!(query.emulated, Some(true));
        assert_eq!(query.region, None);
        // Variables only matter when a variable filter is enabled.
        assert!(query.variables.is_empty());

        timer.run.game_name.clear();
        assert_eq!(Query::for_layout(&layout, &timer), None);
    }

    #[test]
    fn retry_after_is_clamped() {
        assert_eq!(parse_retry_after(" 120 "), Some(Duration::from_secs(120)));
        assert_eq!(
            parse_retry_after("18446744073709551615"),
            Some(MAX_RETRY_AFTER)
        );
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
        let now = Instant::now();
        assert_eq!(retry_at(now, Duration::MAX), now + MAX_RETRY_AFTER);
        assert_eq!(retry_at(now, ERROR_RETRY), now + ERROR_RETRY);
    }

    #[test]
    fn lookups_recover_after_the_worker_died() {
        let (requests, _request_receiver) = mpsc::channel();
        let (response_sender, responses) = mpsc::channel::<Response>();
        let mut service = WorldRecordService {
            worker: Some(Worker {
                requests,
                responses,
            }),
            entries: HashMap::new(),
            pending: Some(sms_query()),
        };
        // The worker thread panics before answering.
        drop(response_sender);
        service.poll();
        assert!(service.worker.is_none());
        assert!(service.pending.is_none());
        let entry = &service.entries[&sms_query()];
        assert!(entry.record.is_none());
        assert!(entry.refresh_at > Instant::now());
    }
}
