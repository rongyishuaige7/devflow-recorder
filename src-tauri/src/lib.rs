use serde::{Deserialize, Serialize};
use serde_json::Value;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, thread};

const LOCAL_BRIDGE_ADDR: &str = "127.0.0.1:45173";
const GNOME_WINDOW_PATH: &str = "/v1/gnome/window";
const GNOME_WINDOW_STALE_MS: u128 = 120_000;
const FOCUS_SWITCH_CONFIRM_MS: u128 = 15_000;
const ACTIVITY_RESUME_WINDOW_MS: u128 = 30 * 60 * 1000;
const DB_FILE_NAME: &str = "devflow-recorder.sqlite";

#[derive(Debug)]
struct AppState {
    gnome_window: Mutex<Option<ReceivedGnomeWindow>>,
    timeline: Mutex<Vec<TrackedActivity>>,
    pending_focus: Mutex<Option<PendingFocus>>,
    db: Mutex<Option<Connection>>,
    recording_enabled: Mutex<bool>,
    bridge_token: Mutex<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            gnome_window: Mutex::new(None),
            timeline: Mutex::new(Vec::new()),
            pending_focus: Mutex::new(None),
            db: Mutex::new(None),
            recording_enabled: Mutex::new(true),
            bridge_token: Mutex::new(String::new()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderStatus {
    id: String,
    name: String,
    state: String,
    detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvironmentSnapshot {
    session_type: String,
    desktop: String,
    compositor: String,
    wayland_display: Option<String>,
    x11_display: Option<String>,
    providers: Vec<ProviderStatus>,
}

#[derive(Debug, Clone)]
struct ReceivedGnomeWindow {
    payload: GnomeWindowPayload,
    received_at_ms: u128,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ActivityEvent {
    id: String,
    started_at: String,
    ended_at: String,
    duration_seconds: u32,
    duration_minutes: u16,
    app: String,
    title: String,
    context: String,
    kind: String,
    confidence: u8,
    privacy: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ActiveWindow {
    app: String,
    title: String,
    source: String,
    pid: Option<u32>,
}

#[derive(Debug, Clone)]
struct TrackedActivity {
    id: String,
    activity_key: String,
    first_started_at_ms: u128,
    last_seen_at_ms: u128,
    app: String,
    title: String,
    normalized_title: String,
    source: String,
    pid: Option<u32>,
    segments: Vec<ActivitySegment>,
}

#[derive(Debug, Clone)]
struct ActivitySegment {
    started_at_ms: u128,
    ended_at_ms: Option<u128>,
}

#[derive(Debug, Clone)]
struct PendingFocus {
    window: ActiveWindow,
    started_at_ms: u128,
}

#[derive(Debug, Clone)]
struct TerminalCwdHint {
    path: String,
    exact: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecorderSnapshot {
    environment: EnvironmentSnapshot,
    active_window: Option<ActiveWindow>,
    events: Vec<ActivityEvent>,
}

#[derive(Debug, Deserialize)]
struct BrowserActivityPayload {
    title: String,
    url: String,
    browser: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GnomeWindowPayload {
    title: String,
    app: Option<String>,
    app_id: Option<String>,
    wm_class: Option<String>,
    pid: Option<u32>,
    workspace: Option<i64>,
    focused_at: Option<u128>,
}

#[tauri::command]
fn get_recorder_snapshot(state: tauri::State<'_, Arc<AppState>>) -> RecorderSnapshot {
    let environment = get_environment_snapshot(Some(state.inner()));
    let provider_window = read_active_window().or_else(|| read_gnome_window(state.inner()));
    let active_window = provider_window.or_else(|| read_open_timeline_window(state.inner()));
    let events = timeline_events(state.inner());

    RecorderSnapshot {
        environment,
        active_window,
        events,
    }
}

#[tauri::command]
fn ingest_browser_activity(
    state: tauri::State<'_, Arc<AppState>>,
    payload: BrowserActivityPayload,
) -> ActivityEvent {
    let app = payload.browser.unwrap_or_else(|| "Browser".to_string());
    let window = ActiveWindow {
        app,
        title: sanitize_title(&payload.title),
        source: format!("browser-extension · {}", sanitize_url(&payload.url)),
        pid: None,
    };
    record_window_activity(state.inner(), &window, now_ms());

    timeline_events(state.inner())
        .into_iter()
        .find(|event| event.title == window.title)
        .unwrap_or_else(|| ActivityEvent {
            id: format!("browser-{}", now_ms()),
            started_at: "刚刚".to_string(),
            ended_at: "记录中".to_string(),
            duration_seconds: 0,
            duration_minutes: 0,
            app: window.app,
            title: window.title,
            context: window.source,
            kind: "网页".to_string(),
            confidence: 96,
            privacy: "URL 参数已在本地脱敏".to_string(),
        })
}

#[tauri::command]
fn set_recording_enabled(state: tauri::State<'_, Arc<AppState>>, enabled: bool) -> bool {
    if let Ok(mut recording_enabled) = state.recording_enabled.lock() {
        *recording_enabled = enabled;
    }

    if !enabled {
        close_current_recording(state.inner(), now_ms());
    }

    enabled
}

fn get_environment_snapshot(state: Option<&Arc<AppState>>) -> EnvironmentSnapshot {
    let session_type = env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string());
    let desktop = env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| env::var("DESKTOP_SESSION"))
        .unwrap_or_else(|_| "unknown".to_string());
    let wayland_display = env::var("WAYLAND_DISPLAY").ok();
    let x11_display = env::var("DISPLAY").ok();
    let compositor = detect_compositor(&desktop);

    EnvironmentSnapshot {
        session_type,
        desktop,
        compositor,
        wayland_display,
        x11_display,
        providers: provider_statuses(state),
    }
}

fn provider_statuses(state: Option<&Arc<AppState>>) -> Vec<ProviderStatus> {
    let desktop = env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| env::var("DESKTOP_SESSION"))
        .unwrap_or_else(|_| "unknown".to_string());
    let desktop_lower = desktop.to_lowercase();
    let is_gnome = desktop_lower.contains("gnome") || desktop_lower.contains("ubuntu");
    let is_kde = desktop_lower.contains("kde") || desktop_lower.contains("plasma");

    let mut providers = vec![
        ProviderStatus {
            id: "hyprland".to_string(),
            name: "Hyprland activewindow".to_string(),
            state: if command_exists("hyprctl") && env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
                "ready"
            } else {
                "standby"
            }
            .to_string(),
            detail: "通过 hyprctl activewindow -j 读取当前窗口。".to_string(),
        },
        ProviderStatus {
            id: "sway".to_string(),
            name: "Sway tree".to_string(),
            state: if command_exists("swaymsg") && env::var("SWAYSOCK").is_ok() {
                "ready"
            } else {
                "standby"
            }
            .to_string(),
            detail: "通过 swaymsg -t get_tree 查找 focused 节点。".to_string(),
        },
        ProviderStatus {
            id: "browser-extension".to_string(),
            name: "浏览器扩展桥".to_string(),
            state: "planned".to_string(),
            detail: "扩展将把激活标签页标题和脱敏 URL 发给本地 Tauri 命令。".to_string(),
        },
    ];

    if is_gnome {
        let gnome_state = state
            .and_then(|state| state.gnome_window.lock().ok())
            .and_then(|window| window.as_ref().map(|window| now_ms().saturating_sub(window.received_at_ms)))
            .map(|age_ms| {
                let age_label = short_age_label(age_ms);
                if age_ms <= 10_000 {
                    ("ready", format!("GNOME Shell 扩展正在上报窗口元数据，最近一次在 {age_label} 前。"))
                } else {
                    ("partial", format!("本地接收端已启动，但 GNOME Shell 扩展最近一次上报是 {age_label} 前，可能已停用。"))
                }
            })
            .unwrap_or((
                "standby",
                "本地接收端已就绪；安装并启用 GNOME Shell 扩展后可上报焦点窗口。".to_string(),
            ));

        providers.push(ProviderStatus {
            id: "gnome-shell-extension".to_string(),
            name: "GNOME Shell 扩展".to_string(),
            state: gnome_state.0.to_string(),
            detail: gnome_state.1,
        });
    }

    if is_kde {
        providers.push(ProviderStatus {
            id: "kwin".to_string(),
            name: "KDE KWin DBus".to_string(),
            state: if command_exists("qdbus") || command_exists("qdbus6") {
                "partial"
            } else {
                "standby"
            }
            .to_string(),
            detail: "KWin 可通过脚本/DBus 扩展，默认接口不保证暴露窗口标题。".to_string(),
        });
    }

    if env::var("XDG_SESSION_TYPE").unwrap_or_default() == "x11" {
        providers.push(ProviderStatus {
            id: "x11-fallback".to_string(),
            name: "X11 fallback".to_string(),
            state: "available".to_string(),
            detail: "仅作为兼容兜底，不作为第一优先级。".to_string(),
        });
    }

    providers
}

fn read_active_window() -> Option<ActiveWindow> {
    read_hyprland_window().or_else(read_sway_window)
}

fn read_gnome_window(state: &Arc<AppState>) -> Option<ActiveWindow> {
    let received = state.gnome_window.lock().ok()?.clone()?;
    if now_ms().saturating_sub(received.received_at_ms) > GNOME_WINDOW_STALE_MS {
        return None;
    }

    active_window_from_gnome_payload(&received.payload)
}

fn read_open_timeline_window(state: &Arc<AppState>) -> Option<ActiveWindow> {
    let timeline = state.timeline.lock().ok()?;
    let activity = timeline
        .iter()
        .filter(|activity| activity_has_open_segment(activity))
        .max_by_key(|activity| activity.last_seen_at_ms)?;

    Some(ActiveWindow {
        app: activity.app.clone(),
        title: activity.title.clone(),
        source: activity.source.clone(),
        pid: activity.pid,
    })
}

fn active_window_from_gnome_payload(payload: &GnomeWindowPayload) -> Option<ActiveWindow> {
    if payload.title.trim().is_empty() {
        return None;
    }

    let app = payload
        .app
        .clone()
        .or_else(|| payload.app_id.clone())
        .or_else(|| payload.wm_class.clone())
        .unwrap_or_else(|| "GNOME Window".to_string());

    Some(ActiveWindow {
        app,
        title: sanitize_title(&payload.title),
        source: "gnome-shell-extension".to_string(),
        pid: payload.pid,
    })
}

fn read_hyprland_window() -> Option<ActiveWindow> {
    if !command_exists("hyprctl") || env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err() {
        return None;
    }

    let output = Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    let app = value
        .get("class")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled")
        .to_string();
    let pid = value.get("pid").and_then(Value::as_u64).map(|pid| pid as u32);

    Some(ActiveWindow {
        app,
        title: sanitize_title(&title),
        source: "hyprctl activewindow".to_string(),
        pid,
    })
}

fn read_sway_window() -> Option<ActiveWindow> {
    if !command_exists("swaymsg") || env::var("SWAYSOCK").is_err() {
        return None;
    }

    let output = Command::new("swaymsg")
        .args(["-t", "get_tree"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    let focused = find_focused_node(&value)?;
    let app = focused
        .get("app_id")
        .or_else(|| focused.get("window_properties").and_then(|props| props.get("class")))
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();
    let title = focused
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Untitled")
        .to_string();
    let pid = focused.get("pid").and_then(Value::as_u64).map(|pid| pid as u32);

    Some(ActiveWindow {
        app,
        title: sanitize_title(&title),
        source: "swaymsg get_tree".to_string(),
        pid,
    })
}

fn find_focused_node(value: &Value) -> Option<&Value> {
    if value.get("focused").and_then(Value::as_bool) == Some(true) {
        return Some(value);
    }

    for key in ["nodes", "floating_nodes"] {
        if let Some(nodes) = value.get(key).and_then(Value::as_array) {
            for node in nodes {
                if let Some(found) = find_focused_node(node) {
                    return Some(found);
                }
            }
        }
    }

    None
}

fn record_window_activity(state: &Arc<AppState>, window: &ActiveWindow, timestamp_ms: u128) {
    if should_ignore_activity(window) || !is_recording_enabled(state) {
        return;
    }

    let timeline_to_persist = {
        let Ok(mut timeline) = state.timeline.lock() else {
            return;
        };
        let Ok(mut pending_focus) = state.pending_focus.lock() else {
            return;
        };
        let activity_key = activity_key_for_window(window);

        if let Some(activity) = timeline
            .iter_mut()
            .find(|activity| activity.activity_key == activity_key && activity_has_open_segment(activity))
        {
            *pending_focus = None;
            update_activity_metadata(activity, window, timestamp_ms);
            Some(timeline.clone())
        } else if open_activity_index(&timeline).is_none() {
            *pending_focus = None;
            resume_or_push_activity(&mut timeline, window, timestamp_ms);
            prune_timeline(&mut timeline);
            Some(timeline.clone())
        } else {
            let pending = match pending_focus.as_mut() {
                Some(pending) if activity_key_for_window(&pending.window) == activity_key => {
                    pending.window = window.clone();
                    pending
                }
                _ => {
                    *pending_focus = Some(PendingFocus {
                        window: window.clone(),
                        started_at_ms: timestamp_ms,
                    });
                    return;
                }
            };

            if timestamp_ms.saturating_sub(pending.started_at_ms) < FOCUS_SWITCH_CONFIRM_MS {
                return;
            }

            let next_window = pending.window.clone();
            let next_started_at_ms = pending.started_at_ms;
            *pending_focus = None;

            close_open_activity(&mut timeline, next_started_at_ms);
            resume_or_push_activity(&mut timeline, &next_window, next_started_at_ms);
            prune_timeline(&mut timeline);
            Some(timeline.clone())
        }
    };

    if let Some(timeline) = timeline_to_persist {
        persist_timeline(state, &timeline);
    }
}

fn resume_or_push_activity(
    timeline: &mut Vec<TrackedActivity>,
    window: &ActiveWindow,
    timestamp_ms: u128,
) {
    let activity_key = activity_key_for_window(window);
    if let Some(activity) = timeline.iter_mut().find(|activity| {
        activity.activity_key == activity_key
            && timestamp_ms.saturating_sub(activity.last_seen_at_ms) <= ACTIVITY_RESUME_WINDOW_MS
    }) {
        update_activity_metadata(activity, window, timestamp_ms);
        activity.segments.push(ActivitySegment {
            started_at_ms: timestamp_ms,
            ended_at_ms: None,
        });
        return;
    }

    let display_title = display_title_for_window(window);
    let normalized_title = normalize_title_for_activity(&display_title);

    timeline.push(TrackedActivity {
        id: format!("activity-{timestamp_ms}"),
        activity_key,
        first_started_at_ms: timestamp_ms,
        last_seen_at_ms: timestamp_ms,
        app: window.app.clone(),
        title: display_activity_title(&display_title, &normalized_title),
        normalized_title,
        source: window.source.clone(),
        pid: window.pid,
        segments: vec![ActivitySegment {
            started_at_ms: timestamp_ms,
            ended_at_ms: None,
        }],
    });
}

fn close_open_activity(timeline: &mut [TrackedActivity], ended_at_ms: u128) {
    if let Some(activity) = timeline
        .iter_mut()
        .find(|activity| activity_has_open_segment(activity))
    {
        if let Some(segment) = activity.segments.last_mut() {
            if segment.ended_at_ms.is_none() {
                segment.ended_at_ms = Some(ended_at_ms);
                activity.last_seen_at_ms = ended_at_ms;
            }
        }
    }
}

fn update_activity_metadata(activity: &mut TrackedActivity, window: &ActiveWindow, timestamp_ms: u128) {
    let display_title = display_title_for_window(window);
    let normalized_title = normalize_title_for_activity(&display_title);
    activity.app = window.app.clone();
    activity.title = display_activity_title(&display_title, &normalized_title);
    activity.normalized_title = normalized_title;
    activity.source = window.source.clone();
    activity.pid = window.pid;
    activity.last_seen_at_ms = timestamp_ms;
}

fn open_activity_index(timeline: &[TrackedActivity]) -> Option<usize> {
    timeline.iter().position(activity_has_open_segment)
}

fn activity_has_open_segment(activity: &TrackedActivity) -> bool {
    activity
        .segments
        .last()
        .map(|segment| segment.ended_at_ms.is_none())
        .unwrap_or(false)
}

fn activity_key_for_window(window: &ActiveWindow) -> String {
    format!(
        "{}:{}",
        window.app.to_lowercase(),
        normalize_title_for_activity(&activity_key_title_for_window(window)).to_lowercase()
    )
}

fn activity_key_title_for_window(window: &ActiveWindow) -> String {
    if is_terminal_app(&window.app) {
        return terminal_cwd_hint(&window.title)
            .map(|hint| hint.path)
            .unwrap_or_else(|| terminal_title_tail(&window.title));
    }

    base_display_title_for_window(window)
}

fn prune_timeline(timeline: &mut Vec<TrackedActivity>) {
    while timeline.len() > 100 {
        if let Some(index) = timeline
            .iter()
            .enumerate()
            .min_by_key(|(_, activity)| activity.last_seen_at_ms)
            .map(|(index, _)| index)
        {
            timeline.remove(index);
        } else {
            break;
        }
    }
}

fn should_ignore_activity(window: &ActiveWindow) -> bool {
    let app = window.app.to_lowercase();
    let title = window.title.to_lowercase();
    app == "devflow_recorder" || title == "devflow recorder"
}

fn is_recording_enabled(state: &Arc<AppState>) -> bool {
    state
        .recording_enabled
        .lock()
        .map(|enabled| *enabled)
        .unwrap_or(true)
}

fn close_current_recording(state: &Arc<AppState>, ended_at_ms: u128) {
    let timeline_to_persist = {
        let Ok(mut timeline) = state.timeline.lock() else {
            return;
        };
        close_open_activity(&mut timeline, ended_at_ms);
        Some(timeline.clone())
    };

    if let Some(timeline) = timeline_to_persist {
        persist_timeline(state, &timeline);
    }
}

fn timeline_events(state: &Arc<AppState>) -> Vec<ActivityEvent> {
    let now = now_ms();
    state
        .timeline
        .lock()
        .map(|timeline| {
            let mut activities = timeline.iter().collect::<Vec<_>>();
            activities.sort_by(|left, right| {
                let left_open = activity_has_open_segment(left);
                let right_open = activity_has_open_segment(right);
                right_open
                    .cmp(&left_open)
                    .then_with(|| right.last_seen_at_ms.cmp(&left.last_seen_at_ms))
            });
            activities
                .into_iter()
                .map(|activity| activity_to_event(activity, now))
                .collect()
        })
        .unwrap_or_default()
}

fn activity_to_event(activity: &TrackedActivity, now: u128) -> ActivityEvent {
    let is_open = activity_has_open_segment(activity);
    let duration_ms = activity_duration_ms(activity, now);
    let duration_seconds = duration_ms.checked_div(1000).unwrap_or(0).min(u32::MAX as u128) as u32;
    let duration_minutes = duration_ms.checked_div(60_000).unwrap_or(0).min(u16::MAX as u128) as u16;

    ActivityEvent {
        id: activity.id.clone(),
        started_at: time_label(activity.first_started_at_ms),
        ended_at: if is_open {
            "记录中".to_string()
        } else {
            time_label(activity.last_seen_at_ms)
        },
        duration_seconds,
        duration_minutes,
        app: activity.app.clone(),
        title: activity.title.clone(),
        context: format!("来源：{} · {} 段累计", activity.source, activity.segments.len()),
        kind: "当前窗口".to_string(),
        confidence: 93,
        privacy: "Wayland provider 直接返回的元数据".to_string(),
    }
}

fn activity_duration_ms(activity: &TrackedActivity, now: u128) -> u128 {
    activity
        .segments
        .iter()
        .map(|segment| {
            segment
                .ended_at_ms
                .unwrap_or(now)
                .saturating_sub(segment.started_at_ms)
        })
        .sum()
}

fn initialize_database(state: &Arc<AppState>) {
    let Some(path) = database_path() else {
        eprintln!("DevFlow SQLite skipped: unable to resolve app data path");
        return;
    };

    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!("DevFlow SQLite skipped: unable to create {}: {err}", parent.display());
            return;
        }
    }

    let connection = match Connection::open(&path) {
        Ok(connection) => connection,
        Err(err) => {
            eprintln!("DevFlow SQLite skipped: unable to open {}: {err}", path.display());
            return;
        }
    };

    if let Err(err) = migrate_database(&connection) {
        eprintln!("DevFlow SQLite skipped: migration failed for {}: {err}", path.display());
        return;
    }
    if let Err(err) = close_stale_open_segments(&connection) {
        eprintln!("DevFlow SQLite stale segment cleanup failed: {err}");
    }

    match load_today_activities(&connection) {
        Ok(activities) => {
            if let Ok(mut timeline) = state.timeline.lock() {
                *timeline = activities;
            }
        }
        Err(err) => eprintln!("DevFlow SQLite load failed: {err}"),
    }

    if let Ok(mut db) = state.db.lock() {
        *db = Some(connection);
    }

    eprintln!("DevFlow SQLite ready: {}", path.display());
}

fn initialize_bridge_token(state: &Arc<AppState>) {
    let token = format!("{:x}-{:x}", now_ms(), std::process::id());
    let Some(path) = bridge_token_path() else {
        eprintln!("DevFlow bridge token skipped: unable to resolve app data path");
        return;
    };

    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!("DevFlow bridge token skipped: unable to create {}: {err}", parent.display());
            return;
        }
    }

    if let Err(err) = fs::write(&path, &token) {
        eprintln!("DevFlow bridge token skipped: unable to write {}: {err}", path.display());
        return;
    }

    if let Ok(mut bridge_token) = state.bridge_token.lock() {
        *bridge_token = token;
    }
}

fn database_path() -> Option<PathBuf> {
    let data_home = env::var("XDG_DATA_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/share")))?;

    Some(data_home.join("devflow-recorder").join(DB_FILE_NAME))
}

fn bridge_token_path() -> Option<PathBuf> {
    database_path().map(|path| path.with_file_name("bridge-token"))
}

fn migrate_database(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS activities (
            id TEXT PRIMARY KEY,
            activity_key TEXT NOT NULL,
            first_started_at_ms INTEGER NOT NULL,
            last_seen_at_ms INTEGER NOT NULL,
            app TEXT NOT NULL,
            title TEXT NOT NULL,
            normalized_title TEXT NOT NULL,
            source TEXT NOT NULL,
            pid INTEGER,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS activity_segments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            activity_id TEXT NOT NULL,
            started_at_ms INTEGER NOT NULL,
            ended_at_ms INTEGER,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE(activity_id, started_at_ms),
            FOREIGN KEY(activity_id) REFERENCES activities(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_activities_last_seen
            ON activities(last_seen_at_ms);
        CREATE INDEX IF NOT EXISTS idx_segments_activity
            ON activity_segments(activity_id, started_at_ms);
        ",
    )
}

fn close_stale_open_segments(connection: &Connection) -> rusqlite::Result<()> {
    let now = i64_from_u128(now_ms());
    connection.execute(
        "
        UPDATE activity_segments
        SET ended_at_ms = (
                SELECT last_seen_at_ms
                FROM activities
                WHERE activities.id = activity_segments.activity_id
            ),
            updated_at_ms = ?
        WHERE ended_at_ms IS NULL
        ",
        params![now],
    )?;
    Ok(())
}

fn load_today_activities(connection: &Connection) -> rusqlite::Result<Vec<TrackedActivity>> {
    let day_start = i64_from_u128(day_start_ms(now_ms()));
    let mut statement = connection.prepare(
        "
        SELECT id, activity_key, first_started_at_ms, last_seen_at_ms, app, title,
               normalized_title, source, pid
        FROM activities
        WHERE last_seen_at_ms >= ?
        ORDER BY first_started_at_ms ASC
        ",
    )?;

    let rows = statement.query_map(params![day_start], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, Option<i64>>(8)?,
        ))
    })?;

    let mut activities = Vec::new();
    for row in rows {
        let (
            id,
            activity_key,
            first_started_at_ms,
            last_seen_at_ms,
            app,
            title,
            normalized_title,
            source,
            pid,
        ) = row?;
        let segments = load_activity_segments(connection, &id)?;
        activities.push(TrackedActivity {
            id,
            activity_key,
            first_started_at_ms: u128_from_i64(first_started_at_ms),
            last_seen_at_ms: u128_from_i64(last_seen_at_ms),
            app,
            title,
            normalized_title,
            source,
            pid: pid.and_then(|pid| u32::try_from(pid).ok()),
            segments,
        });
    }

    Ok(activities)
}

fn load_activity_segments(
    connection: &Connection,
    activity_id: &str,
) -> rusqlite::Result<Vec<ActivitySegment>> {
    let mut statement = connection.prepare(
        "
        SELECT started_at_ms, ended_at_ms
        FROM activity_segments
        WHERE activity_id = ?
        ORDER BY started_at_ms ASC
        ",
    )?;

    let rows = statement.query_map(params![activity_id], |row| {
        Ok(ActivitySegment {
            started_at_ms: u128_from_i64(row.get::<_, i64>(0)?),
            ended_at_ms: row.get::<_, Option<i64>>(1)?.map(u128_from_i64),
        })
    })?;

    rows.collect()
}

fn persist_timeline(state: &Arc<AppState>, timeline: &[TrackedActivity]) {
    let Ok(mut db) = state.db.lock() else {
        return;
    };
    let Some(connection) = db.as_mut() else {
        return;
    };

    if let Err(err) = persist_timeline_inner(connection, timeline) {
        eprintln!("DevFlow SQLite persist failed: {err}");
    }
}

fn persist_timeline_inner(
    connection: &mut Connection,
    timeline: &[TrackedActivity],
) -> rusqlite::Result<()> {
    let now = i64_from_u128(now_ms());
    let tx = connection.transaction()?;

    for activity in timeline {
        tx.execute(
            "
            INSERT INTO activities (
                id, activity_key, first_started_at_ms, last_seen_at_ms, app, title,
                normalized_title, source, pid, created_at_ms, updated_at_ms
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                activity_key = excluded.activity_key,
                first_started_at_ms = excluded.first_started_at_ms,
                last_seen_at_ms = excluded.last_seen_at_ms,
                app = excluded.app,
                title = excluded.title,
                normalized_title = excluded.normalized_title,
                source = excluded.source,
                pid = excluded.pid,
                updated_at_ms = excluded.updated_at_ms
            ",
            params![
                activity.id,
                activity.activity_key,
                i64_from_u128(activity.first_started_at_ms),
                i64_from_u128(activity.last_seen_at_ms),
                activity.app,
                activity.title,
                activity.normalized_title,
                activity.source,
                activity.pid.map(i64::from),
                now,
                now,
            ],
        )?;

        for segment in &activity.segments {
            tx.execute(
                "
                INSERT INTO activity_segments (
                    activity_id, started_at_ms, ended_at_ms, created_at_ms, updated_at_ms
                )
                VALUES (?, ?, ?, ?, ?)
                ON CONFLICT(activity_id, started_at_ms) DO UPDATE SET
                    ended_at_ms = excluded.ended_at_ms,
                    updated_at_ms = excluded.updated_at_ms
                ",
                params![
                    activity.id,
                    i64_from_u128(segment.started_at_ms),
                    segment.ended_at_ms.map(i64_from_u128),
                    now,
                    now,
                ],
            )?;
        }
    }

    tx.commit()
}

fn day_start_ms(timestamp_ms: u128) -> u128 {
    let offset_ms = u128::from(local_timezone_offset_seconds()) * 1000;
    ((timestamp_ms + offset_ms) / 86_400_000) * 86_400_000 - offset_ms
}

fn i64_from_u128(value: u128) -> i64 {
    value.min(i64::MAX as u128) as i64
}

fn u128_from_i64(value: i64) -> u128 {
    u128::try_from(value).unwrap_or_default()
}

fn time_label(timestamp_ms: u128) -> String {
    let total_seconds = (timestamp_ms / 1000) as u64;
    let seconds_in_day = (total_seconds % 86_400) + local_timezone_offset_seconds();
    let seconds_in_day = seconds_in_day % 86_400;
    let hours = seconds_in_day / 3600;
    let minutes = (seconds_in_day % 3600) / 60;
    format!("{hours:02}:{minutes:02}")
}

fn short_age_label(age_ms: u128) -> String {
    if age_ms < 1_000 {
        return format!("{age_ms}ms");
    }

    let seconds = age_ms / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }

    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }

    format!("{}h", minutes / 60)
}

fn local_timezone_offset_seconds() -> u64 {
    env::var("TZ")
        .ok()
        .and_then(|tz| parse_fixed_timezone_offset(&tz))
        .unwrap_or(8 * 3600)
}

fn parse_fixed_timezone_offset(tz: &str) -> Option<u64> {
    if tz == "UTC" {
        return Some(0);
    }

    let offset = tz.strip_prefix("UTC+")?.parse::<u64>().ok()?;
    Some(offset.saturating_mul(3600))
}

fn normalize_title_for_activity(title: &str) -> String {
    title
        .trim_start_matches(|ch: char| {
            ch.is_whitespace()
                || ('\u{2800}'..='\u{28ff}').contains(&ch)
                || matches!(
                    ch,
                    '⠁' | '⠂' | '⠄' | '⡀' | '⢀' | '⠠' | '⠐' | '⠈' | '⠋' | '⠙' | '⠹'
                        | '⠸' | '⠼' | '⠴' | '⠦' | '⠧' | '⠇' | '⠏'
                )
        })
        .trim()
        .to_string()
}

fn display_activity_title(raw_title: &str, normalized_title: &str) -> String {
    if normalized_title.is_empty() {
        sanitize_title(raw_title)
    } else {
        normalized_title.to_string()
    }
}

fn display_title_for_window(window: &ActiveWindow) -> String {
    let base_title = base_display_title_for_window(window);
    if is_terminal_app(&window.app) {
        if let Some(pid) = window.pid {
            let cwd_hint = terminal_cwd_hint(&window.title);
            let tools = terminal_process_names(pid, cwd_hint.as_ref());
            if !tools.is_empty() {
                return format!("{base_title} · {}", tools.join(", "));
            }
        }
    }

    base_title
}

fn base_display_title_for_window(window: &ActiveWindow) -> String {
    if is_terminal_app(&window.app) {
        return terminal_title_tail(&window.title);
    }

    sanitize_title(&window.title)
}

fn is_terminal_app(app: &str) -> bool {
    let app = app.to_lowercase();
    app.contains("terminal") || app.contains("gnome-terminal") || app == "终端"
}

fn terminal_title_tail(title: &str) -> String {
    let without_prompt = title
        .rsplit_once(": ")
        .map(|(_, tail)| tail)
        .unwrap_or(title)
        .split_once(" · ")
        .map(|(base, _)| base)
        .unwrap_or_else(|| title.rsplit_once(": ").map(|(_, tail)| tail).unwrap_or(title))
        .trim();

    if without_prompt == "~" || without_prompt.is_empty() {
        return sanitize_title(without_prompt);
    }

    without_prompt
        .trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .map(sanitize_title)
        .unwrap_or_else(|| sanitize_title(without_prompt))
}

fn terminal_cwd_hint(title: &str) -> Option<TerminalCwdHint> {
    let cwd = title
        .rsplit_once(": ")
        .map(|(_, tail)| tail)
        .unwrap_or(title)
        .split_once(" · ")
        .map(|(base, _)| base)
        .unwrap_or_else(|| title.rsplit_once(": ").map(|(_, tail)| tail).unwrap_or(title))
        .trim();
    if cwd.is_empty() {
        return None;
    }

    if cwd == "~" {
        return env::var("HOME").ok().map(|path| TerminalCwdHint { path, exact: true });
    }

    if cwd.starts_with("~/") {
        let home = env::var("HOME").ok()?;
        return Some(TerminalCwdHint {
            path: format!("{home}/{}", cwd.trim_start_matches("~/")).trim_end_matches('/').to_string(),
            exact: false,
        });
    }

    if cwd.starts_with('/') {
        return Some(TerminalCwdHint {
            path: cwd.trim_end_matches('/').to_string(),
            exact: false,
        });
    }

    Some(TerminalCwdHint {
        path: cwd.trim_end_matches('/').to_string(),
        exact: true,
    })
}

fn terminal_process_names(root_pid: u32, cwd_hint: Option<&TerminalCwdHint>) -> Vec<String> {
    let mut queue = VecDeque::from([root_pid]);
    let mut visited = HashSet::new();
    let mut counts = HashMap::<String, u16>::new();

    while let Some(pid) = queue.pop_front() {
        if !visited.insert(pid) || visited.len() > 80 {
            continue;
        }

        if pid != root_pid && process_matches_terminal_cwd(pid, cwd_hint) {
            if let Some(name) = read_process_display_name(pid) {
                if is_interesting_terminal_process(&name) {
                    *counts.entry(name).or_insert(0) += 1;
                }
            }
        }

        for child_pid in read_child_pids(pid) {
            if !visited.contains(&child_pid) {
                queue.push_back(child_pid);
            }
        }
    }

    let mut items = counts.into_iter().collect::<Vec<_>>();
    items.sort_by(|left, right| {
        terminal_process_rank(&left.0)
            .cmp(&terminal_process_rank(&right.0))
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.0.cmp(&right.0))
    });

    items
        .into_iter()
        .take(4)
        .map(|(name, count)| {
            if count > 1 {
                format!("{name} x{count}")
            } else {
                name
            }
        })
        .collect()
}

fn process_matches_terminal_cwd(pid: u32, cwd_hint: Option<&TerminalCwdHint>) -> bool {
    let Some(cwd_hint) = cwd_hint else {
        return false;
    };
    let Some(process_cwd) = read_process_cwd(pid) else {
        return false;
    };

    if cwd_hint.path.starts_with('/') {
        if cwd_hint.exact {
            return process_cwd == cwd_hint.path;
        }

        return process_cwd == cwd_hint.path || process_cwd.starts_with(&format!("{}/", cwd_hint.path));
    }

    process_cwd
        .rsplit('/')
        .next()
        .map(|name| name == cwd_hint.path)
        .unwrap_or(false)
}

fn read_process_cwd(pid: u32) -> Option<String> {
    fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|path| path.to_string_lossy().trim_end_matches('/').to_string())
        .filter(|path| !path.is_empty())
}

fn read_child_pids(pid: u32) -> Vec<u32> {
    fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .ok()
        .map(|children| {
            children
                .split_whitespace()
                .filter_map(|pid| pid.parse::<u32>().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn read_process_display_name(pid: u32) -> Option<String> {
    let comm = fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|name| sanitize_process_name(&name))
        .filter(|name| !name.is_empty());
    let argv0 = read_process_argv0_basename(pid);

    if comm.as_deref().is_some_and(|name| name.contains("reasonix"))
        || argv0.as_deref().is_some_and(|name| name.contains("reasonix"))
    {
        return Some("reasonix".to_string());
    }

    comm.or(argv0)
}

fn read_process_argv0_basename(pid: u32) -> Option<String> {
    let bytes = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv0 = bytes.split(|byte| *byte == 0).next()?;
    if argv0.is_empty() {
        return None;
    }

    let argv0 = String::from_utf8_lossy(argv0);
    let basename = argv0.rsplit('/').next().unwrap_or(&argv0);
    let sanitized = sanitize_process_name(basename);
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn sanitize_process_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .take(48)
        .collect()
}

fn is_interesting_terminal_process(name: &str) -> bool {
    let name = name.to_lowercase();
    !matches!(
        name.as_str(),
        "bash"
            | "zsh"
            | "fish"
            | "sh"
            | "dash"
            | "tmux"
            | "screen"
            | "sudo"
            | "su"
            | "login"
            | "gnome-terminal"
            | "gnome-terminal-server"
            | "kgx"
            | "konsole"
            | "xterm"
            | "alacritty"
            | "wezterm"
    )
}

fn terminal_process_rank(name: &str) -> u8 {
    match name {
        "reasonix" => 0,
        "hermes" => 1,
        "codex" => 2,
        "npm" | "npmrundev" => 3,
        "node" => 4,
        _ => 10,
    }
}

fn detect_compositor(desktop: &str) -> String {
    if env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        "Hyprland".to_string()
    } else if env::var("SWAYSOCK").is_ok() {
        "Sway".to_string()
    } else if desktop.to_lowercase().contains("kde") {
        "KDE / KWin".to_string()
    } else if desktop.to_lowercase().contains("gnome") {
        "GNOME / Mutter".to_string()
    } else {
        "unknown".to_string()
    }
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {command}")])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn sanitize_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.chars().take(160).collect()
    }
}

fn sanitize_url(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment)
        .to_string()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn start_local_bridge(state: Arc<AppState>) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(LOCAL_BRIDGE_ADDR) {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("DevFlow local bridge failed to bind {LOCAL_BRIDGE_ADDR}: {err}");
                return;
            }
        };

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => handle_bridge_request(stream, &state),
                Err(err) => eprintln!("DevFlow local bridge connection failed: {err}"),
            }
        }
    });
}

fn start_compositor_collector(state: Arc<AppState>) {
    thread::spawn(move || loop {
        if let Some(window) = read_active_window() {
            record_window_activity(&state, &window, now_ms());
        }
        thread::sleep(Duration::from_secs(5));
    });
}

fn handle_bridge_request(mut stream: TcpStream, state: &Arc<AppState>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = match read_http_request(&mut stream) {
        Ok(request) => request,
        Err(err) => {
            let _ = write_http_response(&mut stream, 400, "Bad Request", "text/plain", &err);
            return;
        }
    };

    let first_line = request.lines().next().unwrap_or_default();

    if first_line.starts_with("GET /health ") {
        let _ = write_http_response(&mut stream, 200, "OK", "application/json", r#"{"ok":true}"#);
        return;
    }

    if first_line.starts_with("OPTIONS ") {
        let _ = write_http_response(&mut stream, 204, "No Content", "text/plain", "");
        return;
    }

    if !first_line.starts_with(&format!("POST {GNOME_WINDOW_PATH} ")) {
        let _ = write_http_response(&mut stream, 404, "Not Found", "text/plain", "not found");
        return;
    }

    if !bridge_token_is_valid(&request, state) {
        let _ = write_http_response(&mut stream, 401, "Unauthorized", "text/plain", "unauthorized");
        return;
    }

    let body = request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_default();

    let payload: GnomeWindowPayload = match serde_json::from_str(body) {
        Ok(payload) => payload,
        Err(err) => {
            let _ = write_http_response(
                &mut stream,
                400,
                "Bad Request",
                "text/plain",
                &format!("invalid json: {err}"),
            );
            return;
        }
    };

    if payload.title.trim().is_empty() {
        let _ = write_http_response(&mut stream, 422, "Unprocessable Entity", "text/plain", "empty title");
        return;
    }

    if let Ok(mut gnome_window) = state.gnome_window.lock() {
        *gnome_window = Some(ReceivedGnomeWindow {
            payload: payload.clone(),
            received_at_ms: now_ms(),
        });
    }

    if let Some(window) = active_window_from_gnome_payload(&payload) {
        let timestamp_ms = payload.focused_at.unwrap_or_else(now_ms);
        record_window_activity(state, &window, timestamp_ms);
    }

    let _ = write_http_response(&mut stream, 204, "No Content", "text/plain", "");
}

fn bridge_token_is_valid(request: &str, state: &Arc<AppState>) -> bool {
    let expected = state
        .bridge_token
        .lock()
        .map(|token| token.clone())
        .unwrap_or_default();
    if expected.is_empty() {
        return false;
    }

    header_value(request, "x-devflow-token")
        .map(|token| token == expected)
        .unwrap_or(false)
}

fn header_value<'a>(request: &'a str, header_name: &str) -> Option<&'a str> {
    request
        .split_once("\r\n\r\n")
        .map(|(headers, _)| headers)
        .unwrap_or(request)
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case(header_name) {
                Some(value.trim())
            } else {
                None
            }
        })
}

fn read_http_request(stream: &mut TcpStream) -> Result<String, String> {
    let mut buffer = Vec::with_capacity(8192);
    let mut chunk = [0_u8; 2048];

    loop {
        let read = stream.read(&mut chunk).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..read]);

        if buffer.len() > 65_536 {
            return Err("request too large".to_string());
        }

        if let Some(header_end) = find_header_end(&buffer) {
            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
            let content_length = parse_content_length(&headers).unwrap_or(0);
            let full_len = header_end + 4 + content_length;

            while buffer.len() < full_len {
                let read = stream.read(&mut chunk).map_err(|err| err.to_string())?;
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }

            buffer.truncate(full_len);
            return String::from_utf8(buffer).map_err(|err| err.to_string());
        }
    }

    String::from_utf8(buffer).map_err(|err| err.to_string())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: http://localhost:1421\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type, x-devflow-token\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

pub fn run() {
    let state = Arc::new(AppState::default());
    initialize_database(&state);
    initialize_bridge_token(&state);
    start_local_bridge(state.clone());
    start_compositor_collector(state.clone());

    let shutdown_state = state.clone();
    let result = tauri::Builder::default()
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            get_recorder_snapshot,
            ingest_browser_activity,
            set_recording_enabled
        ])
        .run(tauri::generate_context!());

    close_current_recording(&shutdown_state, now_ms());
    result.expect("failed to run DevFlow Recorder");
}
