//! System control skills: volume, brightness, launch app, focus window,
//! notify, screenshot. Each declares its capability manifest inline so the
//! registry's audit log records what the assistant actually invoked.

use std::sync::Arc;

use jarvis_desktop::{backend::NotificationPriority, Desktop, NotificationOptions};
use jarvis_skills_macros::skill;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::capabilities::Capabilities;
use crate::registry::Skill;

pub struct SystemSkills {
    desktop: Arc<dyn Desktop>,
}

impl SystemSkills {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self { desktop }
    }
}

async fn run_cmd(cmd: &str, args: &[&str]) -> Value {
    match Command::new(cmd).args(args).output().await {
        Ok(o) if o.status.success() => json!({"ok": true}),
        Ok(o) => json!({
            "ok": false,
            "error": format!("{cmd} exit {:?}", o.status.code()),
            "stderr": String::from_utf8_lossy(&o.stderr).into_owned(),
        }),
        Err(e) => json!({"ok": false, "error": format!("spawn {cmd}: {e}")}),
    }
}

// ---------------- Volume ----------------

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum VolumeAction {
    Up,
    Down,
    Set,
    Mute,
}

#[derive(Deserialize, JsonSchema)]
pub struct VolumeArgs {
    pub action: VolumeAction,
    #[serde(default)]
    pub value: Option<u32>,
}

pub struct VolumeSkill;

#[skill(
    name = "volume",
    description = "Adjust the default audio sink: 'up'/'down' tweaks by step, 'set' uses value (0-100), 'mute' toggles."
)]
impl VolumeSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["wpctl"])
    }
    async fn run(&self, args: VolumeArgs) -> Value {
        match args.action {
            VolumeAction::Up => {
                run_cmd("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", "5%+"]).await
            }
            VolumeAction::Down => {
                run_cmd("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]).await
            }
            VolumeAction::Mute => {
                run_cmd("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]).await
            }
            VolumeAction::Set => {
                let v = args.value.unwrap_or(50).min(100);
                let pct = format!("{}%", v);
                run_cmd("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &pct]).await
            }
        }
    }
}

// ---------------- Brightness ----------------

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BrightnessAction {
    Up,
    Down,
    Set,
}

#[derive(Deserialize, JsonSchema)]
pub struct BrightnessArgs {
    pub action: BrightnessAction,
    #[serde(default)]
    pub value: Option<u32>,
}

pub struct BrightnessSkill;

#[skill(
    name = "brightness",
    description = "Adjust display brightness via brightnessctl."
)]
impl BrightnessSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["brightnessctl"])
    }
    async fn run(&self, args: BrightnessArgs) -> Value {
        match args.action {
            BrightnessAction::Up => run_cmd("brightnessctl", &["set", "+10%"]).await,
            BrightnessAction::Down => run_cmd("brightnessctl", &["set", "10%-"]).await,
            BrightnessAction::Set => {
                let v = args.value.unwrap_or(50).clamp(1, 100);
                let pct = format!("{}%", v);
                run_cmd("brightnessctl", &["set", &pct]).await
            }
        }
    }
}

// ---------------- Launch app ----------------

#[derive(Deserialize, JsonSchema)]
pub struct LaunchAppArgs {
    pub name: String,
}

pub struct LaunchAppSkill;

#[skill(
    name = "launch_app",
    description = "Launch an application by its desktop name or executable."
)]
impl LaunchAppSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["gtk-launch", "xdg-open"])
    }
    async fn run(&self, args: LaunchAppArgs) -> Value {
        let name_clean = args.name.trim();
        let name_lower = name_clean.to_lowercase();

        // 1. Check known aliases on Cinnamon / Mint desktop
        let aliases: Vec<&str> = match name_lower.as_str() {
            "terminal" | "console" | "gnome-terminal" => vec!["org.gnome.Terminal", "gnome-terminal", "x-terminal-emulator"],
            "calculator" | "calc" | "gnome-calculator" => vec!["org.gnome.Calculator", "gnome-calculator"],
            "editor" | "text editor" | "xed" => vec!["xed", "gnome-text-editor", "gedit"],
            "browser" | "web browser" | "chrome" | "google chrome" => vec!["google-chrome", "chromium", "firefox"],
            "files" | "file manager" | "nemo" => vec!["nemo", "org.gnome.Nautilus"],
            _ => vec![],
        };

        for candidate in aliases {
            if let Ok(s) = Command::new("gtk-launch").arg(candidate).status().await {
                if s.success() {
                    return json!({"ok": true, "launched": candidate, "via": "gtk-launch"});
                }
            }
            if let Ok(child) = Command::new(candidate).spawn() {
                let _ = child.id();
                return json!({"ok": true, "launched": candidate, "via": "direct-spawn"});
            }
        }

        // 2. Try gtk-launch directly with the given name
        if let Ok(s) = Command::new("gtk-launch").arg(name_clean).status().await {
            if s.success() {
                return json!({"ok": true, "launched": name_clean, "via": "gtk-launch"});
            }
        }

        // 3. Try gtk-launch with org.gnome.<Capitalized>
        let capitalized = {
            let mut c = name_clean.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        };
        let gnome_candidate = format!("org.gnome.{}", capitalized);
        if let Ok(s) = Command::new("gtk-launch").arg(&gnome_candidate).status().await {
            if s.success() {
                return json!({"ok": true, "launched": gnome_candidate, "via": "gtk-launch"});
            }
        }

        // 4. Scan /usr/share/applications for matching .desktop file
        if let Ok(mut entries) = tokio::fs::read_dir("/usr/share/applications").await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.ends_with(".desktop") && file_name.to_lowercase().contains(&name_lower) {
                    let app_id = file_name.trim_end_matches(".desktop");
                    if let Ok(s) = Command::new("gtk-launch").arg(app_id).status().await {
                        if s.success() {
                            return json!({"ok": true, "launched": app_id, "via": "gtk-launch"});
                        }
                    }
                }
            }
        }

        // 5. Try direct spawn if executable exists
        if let Ok(status) = Command::new("which").arg(name_clean).status().await {
            if status.success() {
                if let Ok(child) = Command::new(name_clean).spawn() {
                    let _ = child.id();
                    return json!({"ok": true, "launched": name_clean, "via": "direct-spawn"});
                }
            }
        }

        // 6. Fall back to xdg-open
        match Command::new("xdg-open").arg(name_clean).status().await {
            Ok(s) if s.success() => json!({"ok": true, "launched": name_clean, "via": "xdg-open"}),
            Ok(s) => json!({"ok": false, "error": format!("launch failed (exit {:?})", s.code())}),
            Err(e) => json!({"ok": false, "error": format!("spawn xdg-open: {e}")}),
        }
    }
}

// ---------------- Focus window ----------------

#[derive(Deserialize, JsonSchema)]
pub struct FocusWindowArgs {
    pub pattern: String,
}

pub struct FocusWindowSkill {
    desktop: Arc<dyn Desktop>,
}

impl FocusWindowSkill {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self { desktop }
    }
}

#[skill(
    name = "focus_window",
    description = "Bring a window matching a substring pattern to the foreground."
)]
impl FocusWindowSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["wmctrl"])
    }
    async fn run(&self, args: FocusWindowArgs) -> Value {
        match self.desktop.focus_window(&args.pattern).await {
            Ok(()) => json!({"ok": true}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        }
    }
}

// ---------------- Notify ----------------

#[derive(Deserialize, JsonSchema)]
pub struct NotifyArgs {
    pub title: String,
    pub body: String,
}

pub struct NotifySkill {
    desktop: Arc<dyn Desktop>,
}

impl NotifySkill {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self { desktop }
    }
}

#[skill(name = "notify", description = "Show a desktop notification.")]
impl NotifySkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::dbus(["org.freedesktop.portal.Notification"])
    }
    async fn run(&self, args: NotifyArgs) -> Value {
        let res = self
            .desktop
            .notify(NotificationOptions {
                title: args.title,
                body: args.body,
                priority: NotificationPriority::default(),
            })
            .await;
        match res {
            Ok(()) => json!({"ok": true}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        }
    }
}

// ---------------- Screenshot ----------------

#[derive(Deserialize, JsonSchema, Default)]
pub struct ScreenshotArgs {}

pub struct ScreenshotSkill {
    desktop: Arc<dyn Desktop>,
}

impl ScreenshotSkill {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self { desktop }
    }
}

#[skill(
    name = "screenshot",
    description = "Take a screenshot via the XDG Portal."
)]
impl ScreenshotSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::dbus(["org.freedesktop.portal.Screenshot"])
    }
    async fn run(&self, _args: ScreenshotArgs) -> Value {
        match self.desktop.screenshot(None).await {
            Ok(p) => json!({"ok": true, "path": p.to_string_lossy()}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        }
    }
}

// Helper: percent-encode query for web search without extra dependencies
fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

// ---------------- Date & Time ----------------

#[derive(Deserialize, JsonSchema, Default)]
pub struct DateTimeArgs {}

pub struct DateTimeSkill;

#[skill(
    name = "datetime",
    description = "Get the current system date, time, day of the week, and timezone."
)]
impl DateTimeSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::none()
    }
    async fn run(&self, _args: DateTimeArgs) -> Value {
        let now = chrono::Local::now();
        let time_str = now.format("%-I:%M %p").to_string();
        let date_str = now.format("%A, %B %-d, %Y").to_string();
        let tz_str = now.format("%Z").to_string();
        let summary = format!("It is currently {time_str} on {date_str} ({tz_str}).");
        json!({
            "ok": true,
            "time": time_str,
            "date": date_str,
            "day_of_week": now.format("%A").to_string(),
            "timezone": tz_str,
            "iso": now.to_rfc3339(),
            "summary": summary
        })
    }
}

// ---------------- System Stats ----------------

#[derive(Deserialize, JsonSchema, Default)]
pub struct SystemStatsArgs {}

pub struct SystemStatsSkill;

#[skill(
    name = "system_stats",
    description = "Get real-time CPU load, RAM memory usage, GPU utilization, VRAM usage, temperature, and battery status."
)]
impl SystemStatsSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["nvidia-smi", "upower"])
    }
    async fn run(&self, _args: SystemStatsArgs) -> Value {
        // 1. CPU Load average
        let load_avg = tokio::fs::read_to_string("/proc/loadavg")
            .await
            .ok()
            .and_then(|s| {
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.len() >= 3 {
                    Some(format!("{} (1m), {} (5m), {} (15m)", parts[0], parts[1], parts[2]))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        // 2. RAM Memory
        let mut mem_total_mb: u64 = 0;
        let mut mem_avail_mb: u64 = 0;
        if let Ok(meminfo) = tokio::fs::read_to_string("/proc/meminfo").await {
            for line in meminfo.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(val) = rest.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()) {
                        mem_total_mb = val / 1024;
                    }
                } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
                    if let Some(val) = rest.split_whitespace().next().and_then(|v| v.parse::<u64>().ok()) {
                        mem_avail_mb = val / 1024;
                    }
                }
            }
        }
        let mem_used_mb = mem_total_mb.saturating_sub(mem_avail_mb);
        let mem_pct = if mem_total_mb > 0 {
            (mem_used_mb as f64 / mem_total_mb as f64 * 100.0).round() as u64
        } else {
            0
        };

        // 3. GPU Stats via nvidia-smi
        let gpu_info = match Command::new("nvidia-smi")
            .args(["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"])
            .output()
            .await
        {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let parts: Vec<&str> = stdout.trim().split(',').map(|s| s.trim()).collect();
                if parts.len() >= 5 {
                    Some(json!({
                        "name": parts[0],
                        "utilization_pct": parts[1],
                        "memory_used_mb": parts[2],
                        "memory_total_mb": parts[3],
                        "temperature_c": parts[4],
                    }))
                } else {
                    None
                }
            }
            _ => None,
        };

        // 4. Battery status
        let mut battery_pct: Option<String> = None;
        let mut battery_state: Option<String> = None;
        if let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("BAT") {
                    let cap_path = entry.path().join("capacity");
                    let status_path = entry.path().join("status");
                    if let Ok(cap) = std::fs::read_to_string(cap_path) {
                        battery_pct = Some(format!("{}%", cap.trim()));
                    }
                    if let Ok(st) = std::fs::read_to_string(status_path) {
                        battery_state = Some(st.trim().to_lowercase());
                    }
                    break;
                }
            }
        }

        // Human readable summary
        let mut summary = format!("CPU load is {load_avg}. Memory: {:.1} GB used of {:.1} GB ({mem_pct}%).",
            mem_used_mb as f64 / 1024.0, mem_total_mb as f64 / 1024.0);
        if let Some(ref g) = gpu_info {
            summary.push_str(&format!(
                " GPU: {} at {}% load, {} degrees Celsius, using {} MB VRAM.",
                g["name"].as_str().unwrap_or("NVIDIA GPU"),
                g["utilization_pct"].as_str().unwrap_or("0"),
                g["temperature_c"].as_str().unwrap_or("0"),
                g["memory_used_mb"].as_str().unwrap_or("0"),
            ));
        }
        if let (Some(ref bp), Some(ref bs)) = (&battery_pct, &battery_state) {
            summary.push_str(&format!(" Battery is at {bp} ({bs})."));
        }

        json!({
            "ok": true,
            "cpu_load": load_avg,
            "memory": {
                "used_mb": mem_used_mb,
                "total_mb": mem_total_mb,
                "percentage": mem_pct
            },
            "gpu": gpu_info,
            "battery": {
                "percentage": battery_pct,
                "state": battery_state
            },
            "summary": summary
        })
    }
}

// ---------------- List Windows ----------------

#[derive(Deserialize, JsonSchema, Default)]
pub struct ListWindowsArgs {}

pub struct ListWindowsSkill;

#[skill(
    name = "list_windows",
    description = "List currently open desktop application windows."
)]
impl ListWindowsSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["wmctrl"])
    }
    async fn run(&self, _args: ListWindowsArgs) -> Value {
        match Command::new("wmctrl").arg("-l").output().await {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let windows: Vec<String> = stdout
                    .lines()
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 4 {
                            let title = parts[3..].join(" ");
                            if !title.is_empty() && title != "Desktop" && title != "plank" {
                                Some(title)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect();
                let summary = if windows.is_empty() {
                    "No application windows currently open.".to_string()
                } else {
                    format!("Open windows: {}.", windows.join(", "))
                };
                json!({
                    "ok": true,
                    "count": windows.len(),
                    "windows": windows,
                    "summary": summary
                })
            }
            Ok(o) => json!({
                "ok": false,
                "error": format!("wmctrl exit {:?}", o.status.code()),
                "stderr": String::from_utf8_lossy(&o.stderr).into_owned()
            }),
            Err(e) => json!({"ok": false, "error": format!("spawn wmctrl: {e}")}),
        }
    }
}

// ---------------- Close App ----------------

#[derive(Deserialize, JsonSchema)]
pub struct CloseAppArgs {
    /// Window title substring or application name to close.
    pub pattern: String,
    /// Must be set to true to confirm closing the application.
    #[serde(default)]
    pub confirmed: bool,
}

pub struct CloseAppSkill;

#[skill(
    name = "close_app",
    description = "Close an application window matching pattern. Set confirmed=false to request confirmation, or confirmed=true when user confirmed."
)]
impl CloseAppSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["wmctrl"])
    }
    async fn run(&self, args: CloseAppArgs) -> Value {
        if !args.confirmed {
            return json!({
                "ok": false,
                "requires_confirmation": true,
                "action": format!("close window matching '{}'", args.pattern),
                "message": format!("Closing '{}' may cause unsaved work to be lost. Please confirm if you wish me to proceed, Sir.", args.pattern)
            });
        }
        // First try direct wmctrl -c
        if let Ok(s) = Command::new("wmctrl").args(["-c", &args.pattern]).status().await {
            if s.success() {
                return json!({
                    "ok": true,
                    "message": format!("Closed window matching '{}'.", args.pattern)
                });
            }
        }
        // Fallback: match window title case-insensitively and close by ID (-i -c)
        let pattern_lower = args.pattern.to_lowercase();
        if let Ok(output) = Command::new("wmctrl").arg("-l").output().await {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let win_id = parts[0];
                    let title = parts[3..].join(" ");
                    if title.to_lowercase().contains(&pattern_lower) {
                        if let Ok(s) = Command::new("wmctrl").args(["-i", "-c", win_id]).status().await {
                            if s.success() {
                                return json!({
                                    "ok": true,
                                    "message": format!("Closed window '{}'.", title)
                                });
                            }
                        }
                    }
                }
            }
        }
        json!({"ok": false, "error": format!("No active window matching '{}' found.", args.pattern)})
    }
}

// ---------------- Lock Screen ----------------

#[derive(Deserialize, JsonSchema, Default)]
pub struct LockScreenArgs {}

pub struct LockScreenSkill;

#[skill(
    name = "lock_screen",
    description = "Lock the desktop session immediately."
)]
impl LockScreenSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["cinnamon-screensaver-command", "xdg-screensaver"])
    }
    async fn run(&self, _args: LockScreenArgs) -> Value {
        if let Ok(s) = Command::new("cinnamon-screensaver-command").arg("-l").status().await {
            if s.success() {
                return json!({"ok": true, "via": "cinnamon-screensaver-command"});
            }
        }
        match Command::new("xdg-screensaver").arg("lock").status().await {
            Ok(s) if s.success() => json!({"ok": true, "via": "xdg-screensaver"}),
            Ok(s) => json!({"ok": false, "error": format!("xdg-screensaver exit {:?}", s.code())}),
            Err(e) => json!({"ok": false, "error": format!("spawn screensaver: {e}")}),
        }
    }
}

// ---------------- Web Search ----------------

#[derive(Deserialize, JsonSchema)]
pub struct WebSearchArgs {
    /// Search terms or question to look up.
    pub query: String,
}

pub struct WebSearchSkill;

#[skill(
    name = "web_search",
    description = "Open the default web browser with search results for the given query."
)]
impl WebSearchSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::exec(["xdg-open"])
    }
    async fn run(&self, args: WebSearchArgs) -> Value {
        let encoded = url_encode(&args.query);
        let url = format!("https://duckduckgo.com/?q={encoded}");
        match Command::new("xdg-open").arg(&url).status().await {
            Ok(s) if s.success() => json!({
                "ok": true,
                "url": url,
                "summary": format!("Searching for '{}' in default browser.", args.query)
            }),
            Ok(s) => json!({"ok": false, "error": format!("xdg-open exit {:?}", s.code())}),
            Err(e) => json!({"ok": false, "error": format!("spawn xdg-open: {e}")}),
        }
    }
}

// ---------------- File Search ----------------

#[derive(Deserialize, JsonSchema)]
pub struct FileSearchArgs {
    /// File name or substring to search for.
    pub query: String,
    /// Subfolder within home (e.g. "Documents", "Downloads", "Projects"). Defaults to personal folders.
    #[serde(default)]
    pub folder: Option<String>,
}

pub struct FileSearchSkill;

#[skill(
    name = "file_search",
    description = "Search for files by name in safe user directories (Documents, Downloads, Desktop, Projects). Sensitive directories (.ssh, .gnupg, /etc) are forbidden."
)]
impl FileSearchSkill {
    fn capabilities(&self) -> Capabilities {
        Capabilities::none()
    }
    async fn run(&self, args: FileSearchArgs) -> Value {
        let query_lower = args.query.to_lowercase();
        if args.query.contains("..") || args.query.contains('/') || args.query.contains('\\') {
            return json!({"ok": false, "error": "Query contains forbidden path navigation sequences."});
        }
        let home = match std::env::var("HOME") {
            Ok(h) => std::path::PathBuf::from(h),
            Err(_) => return json!({"ok": false, "error": "HOME directory not found."}),
        };

        let start_dir = if let Some(ref sub) = args.folder {
            if sub.contains("..") || sub.starts_with('/') {
                return json!({"ok": false, "error": "Invalid search folder specified."});
            }
            home.join(sub)
        } else {
            home.clone()
        };

        let start_str = start_dir.to_string_lossy();
        if start_str.contains(".ssh") || start_str.contains(".gnupg") || start_str.contains(".config") {
            return json!({"ok": false, "error": "Access to sensitive directory is forbidden."});
        }

        let mut matches = Vec::new();
        let mut dirs_to_visit = vec![(start_dir, 0)];

        while let Some((dir, depth)) = dirs_to_visit.pop() {
            if depth > 4 || matches.len() >= 15 {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue; };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }
                let path = entry.path();
                if name.to_lowercase().contains(&query_lower) {
                    matches.push(path.to_string_lossy().into_owned());
                    if matches.len() >= 15 {
                        break;
                    }
                }
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() && depth < 3 {
                        dirs_to_visit.push((path, depth + 1));
                    }
                }
            }
        }

        let summary = if matches.is_empty() {
            format!("No files found matching '{}'.", args.query)
        } else {
            format!("Found {} matching file(s): {}.", matches.len(), matches.join(", "))
        };

        json!({
            "ok": true,
            "query": args.query,
            "count": matches.len(),
            "matches": matches,
            "summary": summary
        })
    }
}

impl SystemSkills {
    /// Hand out an `Arc`'d list of every built-in system skill.
    pub fn skills(self) -> Vec<Arc<dyn Skill>> {
        let desktop = self.desktop;
        vec![
            Arc::new(VolumeSkill),
            Arc::new(BrightnessSkill),
            Arc::new(LaunchAppSkill),
            Arc::new(FocusWindowSkill::new(Arc::clone(&desktop))),
            Arc::new(NotifySkill::new(Arc::clone(&desktop))),
            Arc::new(ScreenshotSkill::new(desktop)),
            Arc::new(DateTimeSkill),
            Arc::new(SystemStatsSkill),
            Arc::new(ListWindowsSkill),
            Arc::new(CloseAppSkill),
            Arc::new(LockScreenSkill),
            Arc::new(WebSearchSkill),
            Arc::new(FileSearchSkill),
        ]
    }
}
