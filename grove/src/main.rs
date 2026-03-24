// Grove requires a Unix or Windows target.
// rfd (file dialogs) does not support WASM, embedded, or UEFI targets anyway,
// but this gives a clear error message instead of a confusing compile failure.
#[cfg(not(any(unix, windows)))]
compile_error!("Grove requires a Unix or Windows target.");

// DEVELOPER NOTE:
// chrono is used instead of std::time for local wall-clock display.
// std::time::SystemTime alone is unsound for local time on Linux
// (no timezone awareness). Do not replace with std::time.
use chrono::{DateTime, Local};
use open;

//rfd is not reliably on more barebones linux installations, including even
//steamdeck desktop mode. so:
#[cfg(not(target_os = "linux"))]
use rfd::FileDialog;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
// ---------------------------------------------------------------------------
// !!!!!!!!!! CHANGE BOTH TO 99 BEFORE RELEASE !!!!!!!!!!
// If she's telling you it's an optimization pass,
// don't bother bringing this up lmfao she's aware.
// ---------------------------------------------------------------------------
const SNAPS_PER_CHAPTER: usize = 99;
const MAX_CHAPTERS: usize = 99;
// ---------------------------------------------------------------------------
// EMOJI SUPPORT
// On non-Windows terminals emoji renders fine. On Windows we only enable it
// when running inside Windows Terminal (WT_SESSION is set by wt.exe).
// Call em(with, without) anywhere — the check is cached after the first call.
// ---------------------------------------------------------------------------
use std::sync::OnceLock;
static EMOJI_OK: OnceLock<bool> = OnceLock::new();
fn emoji_supported() -> bool {
    #[cfg(target_os = "windows")]
    {
        // Windows Terminal
        if std::env::var("WT_SESSION").is_ok() { return true; }
        // VS Code, Hyper, WezTerm (set TERM_PROGRAM on Windows too)
        if let Ok(tp) = std::env::var("TERM_PROGRAM") {
            let tp = tp.to_lowercase();
            if tp.contains("vscode") || tp.contains("hyper") || tp.contains("wezterm") {
                return true;
            }
        }
        // ConEmu / Cmder
        if std::env::var("ConEmuANSI").map(|v| v.eq_ignore_ascii_case("on")).unwrap_or(false) {
            return true;
        }
        false
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Layer 1: explicit terminal program identifiers (VS Code, iTerm2, Hyper, WezTerm, etc.)
        if let Ok(tp) = std::env::var("TERM_PROGRAM") {
            let tp = tp.to_lowercase();
            if tp.contains("vscode") || tp.contains("iterm")
               || tp.contains("hyper") || tp.contains("wezterm") {
                return true;
            }
        }
        // Layer 2: TERM contains a known emoji-capable terminal name
        if let Ok(term) = std::env::var("TERM") {
            if term == "xterm-kitty" || term.contains("alacritty") {
                return true;
            }
        }
        // Layer 3: COLORTERM — set by most modern terminals (Alacritty, Kitty, WezTerm, etc.)
        if let Ok(ct) = std::env::var("COLORTERM") {
            let ct = ct.to_lowercase();
            if ct == "truecolor" || ct == "24bit" {
                return true;
            }
        }
        // Layer 4: UTF-8 locale — catches Gnome Terminal, Konsole, XFCE Terminal, and most
        // Linux desktop terminals that don't set the vars above. Any desktop running a UTF-8
        // locale is overwhelmingly likely to be in an emoji-capable terminal.
        for var in &["LC_ALL", "LC_CTYPE", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                let upper = val.to_uppercase();
                if upper.contains("UTF-8") || upper.contains("UTF8") {
                    return true;
                }
            }
        }
        false
    }
}
fn em<'a>(with: &'a str, without: &'a str) -> &'a str {
    if *EMOJI_OK.get_or_init(emoji_supported) { with } else { without }
}
#[derive(Serialize, Deserialize, Default, Clone)]
struct GroveConfig {
    last_opened_project: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Flat,
    Labyrinth,
}
impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Flat => write!(f, "Flat"),
            Mode::Labyrinth => write!(f, "Labyrinth"),
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Default)]
struct ProjectConfig {
    mode: Mode,
    paths: Vec<String>,
    /// 0 = pre-chapter mode (all snapshots live at the project root).
    /// >=1 = active chapter number; new snapshots go into Chapter {n:02}.
    /// NOTE: current_chapter jumps 0 → 2 on first migration (never lands on 1).
    /// So any check "> 0" is effectively ">= 2" in practice, which satisfies
    /// the spec's "only show chapter if latest chapter is greater than 1"
    /// requirement without needing a special case.
    current_chapter: usize,
}
#[derive(Serialize, Deserialize, Clone)]
struct SnapshotMapping {
    mode: Mode,
    entries: BTreeMap<String, String>,
}
#[derive(Clone)]
struct SnapshotInfo {
    path: PathBuf,
    era: String,
    mode: Mode,
}
// --- NAME SANITIZATION ---
fn sanitize_name(name: &str) -> String {
    let sanitized: String = name.trim().chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    // Windows reserved device names — appending _ makes them safe on all platforms.
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = sanitized.split('.').next().unwrap(); // split always yields ≥1 element
    let upper = base.to_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        format!("{}_", sanitized)
    } else {
        sanitized
    }
}
// --- PATH UTILS ---
fn home_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| {
            eprintln!("Grove: $HOME is not set. Cannot locate the grove directory.");
            std::process::exit(1);
        }))
    }
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| {
            eprintln!("Grove: %USERPROFILE% is not set. Cannot locate the grove directory.");
            std::process::exit(1);
        }))
    }
}
static GROVE_DIR: OnceLock<PathBuf> = OnceLock::new();
fn compute_grove_dir() -> &'static PathBuf {
    GROVE_DIR.get_or_init(|| {
        let mut path = home_dir();
        path.push(".grove");
        path
    })
}
fn ensure_grove_dir() {
    let path = compute_grove_dir();
    if !path.exists() {
        fs::create_dir_all(path).unwrap_or_else(|e| {
            eprintln!("Grove: Could not create grove directory at '{}': {}", path.display(), e);
            std::process::exit(1);
        });
    }
}
fn get_config_path() -> PathBuf {
    compute_grove_dir().join(".grove_config.json")
}
static PROJECTS_ROOT: OnceLock<PathBuf> = OnceLock::new();
fn compute_projects_root() -> &'static PathBuf {
    PROJECTS_ROOT.get_or_init(|| compute_grove_dir().join("Grove Root"))
}
fn ensure_projects_root() -> &'static Path {
    let path = compute_projects_root();
    if !path.exists() {
        fs::create_dir_all(path).unwrap_or_else(|e| {
            eprintln!("Grove: Could not create projects directory at '{}': {}", path.display(), e);
            std::process::exit(1);
        });
    }
    path
}
fn compute_project_dir(name: &str) -> PathBuf {
    compute_projects_root().join(name)
}
fn ensure_project_dir(name: &str) -> PathBuf {
    let path = compute_project_dir(name);
    if !path.exists() {
        fs::create_dir_all(&path).unwrap_or_else(|e| {
            eprintln!("Grove: Could not create project directory at '{}': {}", path.display(), e);
            std::process::exit(1);
        });
    }
    path
}
/// Pre-chapter location_data at the project root.
/// After chapter migration this is only used as the *source* during the move.
fn compute_location_data_dir(project_name: &str) -> PathBuf {
    compute_project_dir(project_name).join(".location_data")
}
fn compute_chapter_dir(project_name: &str, chapter: usize) -> PathBuf {
    compute_project_dir(project_name).join(format!("Chapter {:02}", chapter))
}
fn get_project_config_path(project_name: &str) -> PathBuf {
    compute_project_dir(project_name).join(".grove_project.json")
}
// --- SNAPSHOT META ---
// Lightweight counterpart to SnapshotMapping used during enumeration.
// Serde reads the same JSON file but skips deserializing the `entries` BTreeMap entirely,
// avoiding a potentially large allocation just to read one field.
#[derive(Deserialize)]
struct SnapshotMeta {
    mode: Mode,
}
fn read_snapshot_mode_at(location_data: &Path, snap_folder_name: &str) -> Option<Mode> {
    let json_path = location_data.join(snap_meta_filename(&snap_folder_name));
    if let Ok(data) = fs::read_to_string(json_path) {
        if let Ok(m) = serde_json::from_str::<SnapshotMeta>(&data) {
            return Some(m.mode);
        }
    }
    None
}
// --- CONFIG I/O ---
fn load_config() -> GroveConfig {
    if let Ok(data) = fs::read_to_string(get_config_path()) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        GroveConfig::default()
    }
}
fn save_config(config: &GroveConfig) {
    ensure_grove_dir();
    let data = serde_json::to_string_pretty(config).expect("Failed to serialize grove config");
    fs::write(get_config_path(), data).expect("Failed to write grove config");
}
fn load_project_config(project_name: &str) -> ProjectConfig {
    let path = get_project_config_path(project_name);
    if let Ok(data) = fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str(&data) {
            return cfg;
        }
    }
    ProjectConfig::default()
}
fn save_project_config(project_name: &str, config: &ProjectConfig) {
    let data = serde_json::to_string_pretty(config).expect("Failed to serialize project config");
    fs::write(get_project_config_path(project_name), data).expect("Failed to write project config");
}
// --- CORE UTILS ---
fn read_input(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().unwrap_or(());
    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(0) | Err(_) => String::new(), // EOF or read error → empty string (acts as cancel)
        Ok(_) => input.trim().to_string(),
    }
}
fn is_cancel(input: &str) -> bool {
    let lower = input.trim().to_lowercase();
    lower.is_empty() || lower == "cancel" || lower == "back"
}
fn is_yes(s: &str) -> bool {
    s == "y" || s == "yes"
}
fn say_what() {
    println!("what?");
}
// Note: takes Fn rather than FnMut. If a future retry closure needs
// to mutate captured state, change this to FnMut — zero other changes needed.
fn with_retry<F>(description: &str, f: F)
where
    F: Fn() -> io::Result<()>,
{
    loop {
        match f() {
            Ok(_) => break,
            Err(e) => {
                println!("\n{} Could not {}: {}", em("⚠️", "[!]"), description, e);
                println!("1. Try again");
                println!("2. Skip");
                if read_input("> ").trim() == "2" { break; }
            }
        }
    }
}
fn safe_copy<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dest: Q) {
    if fs::copy(src.as_ref(), dest.as_ref()).is_ok() { return; }
    let src = src.as_ref().to_path_buf();
    let dest = dest.as_ref().to_path_buf();
    with_retry(&format!("copy '{}'", src.display()), || {
        fs::copy(&src, &dest).map(|_| ())
    });
}
fn safe_create_dir_all<P: AsRef<Path>>(path: P) {
    if fs::create_dir_all(path.as_ref()).is_ok() { return; }
    let path = path.as_ref().to_path_buf();
    with_retry(&format!("create directory '{}'", path.display()), || {
        fs::create_dir_all(&path)
    });
}
fn safe_write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) {
    if fs::write(path.as_ref(), contents.as_ref()).is_ok() { return; }
    let path = path.as_ref().to_path_buf();
    let bytes = contents.as_ref().to_vec();
    with_retry(&format!("write to '{}'", path.display()), || {
        fs::write(&path, &bytes)
    });
}
fn safe_remove_dir_all<P: AsRef<Path>>(path: P) {
    if fs::remove_dir_all(path.as_ref()).is_ok() { return; }
    let path = path.as_ref().to_path_buf();
    with_retry(&format!("remove directory '{}'", path.display()), || {
        fs::remove_dir_all(&path)
    });
}
// DEVELOPER NOTE:
// Returns Err(()) — a unit error — when number and name conflict.
// Surfacing which values conflicted would complicate the signature for no real user benefit.
fn handle_menu_choice(input: &str, options: &[(&str, &str)]) -> Result<Option<String>, ()> {
    let lower = input.trim().to_lowercase();
    let parts: Vec<&str> = lower.splitn(2, ' ').collect();
    // Numbers take priority over names per spec (e.g. a project named "1"
    // loses to menu option numbered "1").
    let find = |s: &str| -> Option<&str> {
        options.iter().find(|(num, _)| *num == s).map(|(num, _)| *num)
            .or_else(|| options.iter().find(|(_, name)| *name == s).map(|(num, _)| *num))
    };
    match parts.as_slice() {
        [single] => Ok(find(single).map(str::to_string)),
        [a, b] => match (find(a), find(b)) {
            (Some(x), Some(y)) if x != y => {
                println!("{} Flag: Number and name don't match!", em("⚠️", "[!]"));
                Err(())
            }
            (Some(x), _) | (_, Some(x)) => Ok(Some(x.to_string())),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}
fn parse_del_arg(lower: &str) -> Option<String> {
    if let Some(rest) = lower.strip_prefix("delete ")
        .or_else(|| lower.strip_prefix("del "))
    {
        Some(rest.trim().to_string())
    } else if lower == "delete" || lower == "del" {
        Some(String::new())
    } else {
        None
    }
}
fn get_common_prefix(paths: &[String]) -> PathBuf {
    if paths.is_empty() { return PathBuf::new(); }
    let parents: Vec<PathBuf> = paths.iter().map(|p| {
        let path = Path::new(p);
        match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => path.to_path_buf(),
        }
    }).collect();
    let mut iter = parents.into_iter();
    let first = iter.next().unwrap();
    let mut common: Vec<_> = first.components().collect();
    for path in iter {
        let comps: Vec<_> = path.components().collect();
        let match_len = common.iter().zip(comps.iter()).take_while(|(a, b)| a == b).count();
        common.truncate(match_len);
        if common.is_empty() { break; }
    }
    common.iter().collect()
}
fn strip_absolute_prefix(path: &Path) -> PathBuf {
    path.components()
        .filter(|c| !matches!(c,
            std::path::Component::Prefix(_) | std::path::Component::RootDir))
        .collect()
}
/// Returns `path` relative to `prefix`. If `prefix` is empty or the strip fails,
/// falls back to stripping only the absolute root (drive letter / leading slash).
fn relative_to_prefix(path: &Path, prefix: &Path) -> PathBuf {
    if prefix.as_os_str().is_empty() {
        strip_absolute_prefix(path)
    } else {
        path.strip_prefix(prefix)
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|_| strip_absolute_prefix(path))
    }
}
// --- CHAPTER ENUMERATION ---
fn get_all_chapter_numbers(project_name: &str) -> Vec<usize> {
    let proj_dir = compute_project_dir(project_name);
    let mut chapters = Vec::new();
    let Ok(entries) = fs::read_dir(&proj_dir) else { return chapters; };
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(suffix) = name.strip_prefix("Chapter ") {
            if let Ok(n) = suffix.trim().parse::<usize>() {
                chapters.push(n);
            }
        }
    }
    chapters.sort();
    chapters
}
// --- SNAPSHOT ENUMERATION ---
fn get_chapter_snapshots(project_name: &str, chapter: usize) -> Vec<SnapshotInfo> {
    let search_dir = if chapter == 0 {
        compute_project_dir(project_name)
    } else {
        compute_chapter_dir(project_name, chapter)
    };
    if !search_dir.exists() { return Vec::new(); }
    let location_data = search_dir.join(".location_data");
    let mut snapshots = Vec::new();
    let Ok(entries) = fs::read_dir(&search_dir) else { return snapshots; };
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) { continue; }
        if path == location_data { continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        if chapter == 0 && name.starts_with("Chapter ") { continue; }
        if name.starts_with("__grove_") { continue; }
        let (id_str, era) = if let Some(pos) = name.find('_') {
            (&name[..pos], name[pos + 1..].to_string())
        } else {
            (name.as_str(), String::new())
        };
        if id_str.parse::<usize>().is_ok() {
            let Some(mode) = read_snapshot_mode_at(&location_data, &name) else {
                println!("{} Could not read mode for snapshot '{}', skipping.", em("⚠️", "[!]"), name);
                continue;
            };
            snapshots.push(SnapshotInfo { path, era, mode });
        }
    }
    snapshots.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    snapshots
}
fn format_snapshot_time(path: &Path) -> String {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return "--".to_string(),
    };
    // created() is unsupported by the Linux kernel in most configurations.
    // On Linux/other Unix we go straight to modified() to avoid a guaranteed
    // failed syscall on every call in the hot restore-screen loop.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let system_time = meta.created().or_else(|_| meta.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let system_time = meta.modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let dt: DateTime<Local> = system_time.into();
    dt.format("%b %d %Y  %l:%M%p").to_string()
}
// --- SNAPSHOT META FILENAME ---
// Could be inlined as format!("{}.json", name) at each call site,
// but the named function keeps intent clearer at a glance.
fn snap_meta_filename(snap_folder_name: &str) -> String {
    format!("{}.json", snap_folder_name)
}
// --- SNAPSHOT COPY ENGINE ---
fn copy_dir_recursive(src: &Path, dest: &Path) {
    safe_create_dir_all(dest);
    let Ok(entries) = fs::read_dir(src) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip symlinks — avoids infinite recursion on circular links.
        // Grove backs up content, not link graphs.
        if path.is_symlink() { continue; }
        let target = dest.join(entry.file_name());
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            copy_dir_recursive(&path, &target);
        } else {
            safe_copy(&path, &target);
        }
    }
}
fn flat_stored_name(original_name: &str, src: &Path, counts: &mut HashMap<String, usize>) -> String {
    if let Some(c) = counts.get_mut(original_name) {
        *c += 1;
        let n = *c + 1;
        let stem = src.file_stem().unwrap_or_default().to_string_lossy();
        let ext = src.extension().unwrap_or_default().to_string_lossy();
        if ext.is_empty() {
            format!("{}_{}", stem, n)
        } else {
            format!("{}_{}.{}", stem, n, ext)
        }
    } else {
        counts.insert(original_name.to_string(), 0);
        original_name.to_string()
    }
}
fn copy_into_snapshot(paths: &[String], backup_dir: &Path, mode: &Mode) -> SnapshotMapping {
    if *mode == Mode::Flat {
        let mut entries: BTreeMap<String, String> = BTreeMap::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for path_str in paths {
            let src = Path::new(path_str);
            if !src.exists() { println!("{} '{}' not found, skipping.", em("⚠️", "[!]"), path_str); continue; }
            let original_name = src.file_name().unwrap().to_string_lossy().to_string();
            let stored_name = flat_stored_name(&original_name, src, &mut counts);
            entries.insert(stored_name.clone(), path_str.clone());
            let dest = backup_dir.join(&stored_name);
            if src.is_file() { safe_copy(src, dest); }
            else if src.is_dir() { copy_dir_recursive(src, &dest); }
        }
        SnapshotMapping { mode: Mode::Flat, entries }
    } else {
        let prefix = get_common_prefix(paths);
        let mut entries: BTreeMap<String, String> = BTreeMap::new();
        for path_str in paths {
            let src = Path::new(path_str);
            if !src.exists() { println!("{} '{}' not found, skipping.", em("⚠️", "[!]"), path_str); continue; }
            let relative = relative_to_prefix(src, &prefix);
            let rel_str = relative.to_string_lossy().to_string();
            entries.insert(rel_str.clone(), path_str.clone());
            let final_dest = backup_dir.join(&relative);
            if src.is_file() {
                if let Some(parent) = final_dest.parent() { safe_create_dir_all(parent); }
                safe_copy(src, &final_dest);
            } else if src.is_dir() {
                copy_dir_recursive(src, &final_dest);
            }
        }
        SnapshotMapping { mode: Mode::Labyrinth, entries }
    }
}
// --- NEXT SNAPSHOT ID ---
/// Returns the next available snapshot ID for a chapter by scanning the max
/// existing numeric prefix on disk, then adding 1. Count-based approaches
/// collide after deletion; this is immune to that.
fn next_snapshot_id(project_name: &str, chapter: usize) -> usize {
    let dir = if chapter == 0 {
        compute_project_dir(project_name)
    } else {
        compute_chapter_dir(project_name, chapter)
    };
    let Ok(entries) = fs::read_dir(&dir) else { return 1; };
    let max = entries.flatten()
        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.split('_').next()?.parse::<usize>().ok()
        })
        .max()
        .unwrap_or(0);
    max + 1
}
// --- CHAPTER MANAGEMENT ---
/// One-time migration: moves all root-level snapshots into Chapter 01 and
/// creates an empty Chapter 02 ready for the next save.
///
/// current_chapter jumps directly from 0 to 2 (never 1). This means any
/// check like "current_chapter > 0" is effectively ">= 2" in practice,
/// satisfying the spec's "only show chapter if latest chapter is greater
/// than 1" requirement without a special case.
///
/// current_chapter is persisted BEFORE any files move. If a crash occurs
/// mid-migration, the next run sees current_chapter already set, skips
/// re-migration, and the partially-moved snapshots remain accessible at
/// whichever location they ended up in.
fn migrate_to_chapters(project_name: &str, proj_cfg: &mut ProjectConfig) {
    println!("Organizing into chapters...");
    let proj_dir = compute_project_dir(project_name);
    let ch1_dir = proj_dir.join("Chapter 01");
    let ch1_loc = ch1_dir.join(".location_data");
    safe_create_dir_all(&ch1_dir);
    safe_create_dir_all(&ch1_loc);
    // Persist before moving anything — crash safety.
    proj_cfg.current_chapter = 2;
    save_project_config(project_name, proj_cfg);
    let old_loc = compute_location_data_dir(project_name);
    let existing = get_chapter_snapshots(project_name, 0);
    for snap in &existing {
        let snap_folder_name = snap.path.file_name().unwrap().to_string_lossy().to_string();
        if let Err(e) = fs::rename(&snap.path, ch1_dir.join(&snap_folder_name)) {
            println!("{} Could not move '{}': {}", em("⚠️", "[!]"), snap_folder_name, e);
        }
        let src = old_loc.join(snap_meta_filename(&snap_folder_name));
        if src.exists() {
            let _ = fs::rename(&src, ch1_loc.join(snap_meta_filename(&snap_folder_name)));
        }
    }
    // Clean up the now-empty root-level .location_data folder.
    if old_loc.exists() {
        safe_remove_dir_all(&old_loc);
    }
    safe_create_dir_all(proj_dir.join("Chapter 02"));
    safe_create_dir_all(proj_dir.join("Chapter 02").join(".location_data"));
    println!("{} Chapters created.", em("✨", "*"));
}
fn open_next_chapter(project_name: &str, proj_cfg: &mut ProjectConfig) {
    let new_chapter = proj_cfg.current_chapter + 1;
    let ch_dir = compute_chapter_dir(project_name, new_chapter);
    safe_create_dir_all(&ch_dir);
    safe_create_dir_all(ch_dir.join(".location_data"));
    proj_cfg.current_chapter = new_chapter;
    save_project_config(project_name, proj_cfg);
    println!("{} Chapter {:02} started.", em("✨", "*"), new_chapter);
}
/// Returns true if a chapter contains at least one snapshot folder.
/// Orphaned metadata alone does not veto deletion — only snapshot folders count.
fn chapter_has_content(project_name: &str, chapter: usize) -> bool {
    let ch_dir = compute_chapter_dir(project_name, chapter);
    fs::read_dir(&ch_dir).map(|entries| {
        entries.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            e.file_type().map(|ft| ft.is_dir()).unwrap_or(false) && !n.starts_with('.') && !n.starts_with("__grove_")
        })
    }).unwrap_or(false)
}
/// Removes the latest chapter if empty, then cascades: keeps checking the new
/// latest and removes that too if also empty, until one with content is found.
/// Chapters are never renumbered — gaps are intentional and preserve structural
/// memory ("that save in chapter 3" always stays in chapter 3).
/// Returns true if at least one chapter was removed.
fn maybe_cleanup_empty_latest_chapter(project_name: &str, proj_cfg: &mut ProjectConfig) -> bool {
    let mut removed_any = false;
    loop {
        if proj_cfg.current_chapter == 0 { break; }
        if chapter_has_content(project_name, proj_cfg.current_chapter) { break; }
        let ch_dir = compute_chapter_dir(project_name, proj_cfg.current_chapter);
        safe_remove_dir_all(&ch_dir);
        println!("Chapter {:02} was empty and has been removed.", proj_cfg.current_chapter);
        removed_any = true;
        let remaining = get_all_chapter_numbers(project_name);
        proj_cfg.current_chapter = remaining.last().copied().unwrap_or(0);
        save_project_config(project_name, proj_cfg);
    }
    removed_any
}
// --- DRIFT RECOVERY ---
/// Reconciles proj_cfg.current_chapter with what actually exists on disk.
/// Called after load_project_config so the rest of the program can trust
/// current_chapter regardless of whether the user moved folders externally.
fn reconcile_current_chapter(project_name: &str, proj_cfg: &mut ProjectConfig) {
    if proj_cfg.current_chapter == 0 { return; }
    // If the stored chapter folder is gone, re-derive from disk and persist.
    if !compute_chapter_dir(project_name, proj_cfg.current_chapter).exists() {
        let chapters = get_all_chapter_numbers(project_name);
        let derived = chapters.last().copied().unwrap_or(0);
        if derived != proj_cfg.current_chapter {
            proj_cfg.current_chapter = derived;
            save_project_config(project_name, proj_cfg);
        }
    }
}
/// Scans all snapshot directories for orphaned conversion artifacts left behind
/// by a crash mid-convert. Three cases, each resolved silently:
///
/// - `__grove_converting_X` + `X.bak` both exist:
///     rename of temp→original never completed; original is safe in .bak.
///     Restore .bak → original, delete temp.
/// - `__grove_converting_X` exists but no `.bak`:
///     Original was already replaced before crash; temp is stale. Delete temp.
/// - `X.bak` exists but no `__grove_converting_X`:
///     Conversion succeeded; only the final `remove_dir_all` on .bak didn't finish.
///     Delete .bak.
fn heal_conversion_orphans(project_name: &str) {
    let mut search_dirs: Vec<PathBuf> = vec![compute_project_dir(project_name)];
    for ch in get_all_chapter_numbers(project_name) {
        search_dirs.push(compute_chapter_dir(project_name, ch));
    }
    for dir in &search_dirs {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) { continue; }
            if let Some(snap_name) = name.strip_prefix("__grove_converting_") {
                let bak_path = dir.join(format!("{}.bak", snap_name));
                let original_path = dir.join(snap_name);
                if bak_path.exists() {
                    // Original is safe in .bak — restore it, then clean up temp.
                    if let Err(e) = fs::rename(&bak_path, &original_path) {
                        println!("{} Could not recover '{}' from backup: {}", em("⚠️", "[!]"), snap_name, e);
                    } else {
                        println!("Recovered interrupted conversion for '{}'.", snap_name);
                    }
                } else {
                    println!("Cleaned up leftover conversion temp for '{}'.", snap_name);
                }
                safe_remove_dir_all(&path);
            } else if let Some(snap_name) = name.strip_suffix(".bak") {
                // Only treat as a conversion orphan if no temp dir exists for it.
                let temp_path = dir.join(format!("__grove_converting_{}", snap_name));
                if !temp_path.exists() {
                    safe_remove_dir_all(&path);
                    println!("Cleaned up leftover conversion backup for '{}'.", snap_name);
                }
                // If the temp also exists, the __grove_converting_ branch above handles it.
            }
        }
    }
}
// --- CORE ACTIONS ---
fn do_save(project_name: &str, proj_cfg: &mut ProjectConfig, is_pre_restore: bool) {
    // Called here to cover the CLI path (grove save <project>), which bypasses
    // run_project entirely. In the interactive path, run_project already called
    // this — the second call is a no-op because reconcile is idempotent (one
    // Path::exists() check that returns true, then exits). Do not remove: the
    // CLI path has no other reconcile call.
    reconcile_current_chapter(project_name, proj_cfg);
    if proj_cfg.paths.is_empty() {
        println!("No files listed in this project to save!");
        return;
    }
    // Derive position from highest existing snapshot ID — immune to deletion drift.
    // Uses max-ID, not count: deleting old snapshots never de-increments the gate,
    // only deleting the latest one does. Matches Grove's Soul re: external deletions.
    let highest_id = next_snapshot_id(project_name, proj_cfg.current_chapter).saturating_sub(1);
    let chapter_is_full = highest_id >= SNAPS_PER_CHAPTER;
    // Hard limit: would opening the next chapter exceed the cap?
    if chapter_is_full {
        let next_chapter = if proj_cfg.current_chapter == 0 { 2 } else { proj_cfg.current_chapter + 1 };
        if next_chapter > MAX_CHAPTERS {
            println!(
                "{} This project has reached its snapshot limit ({} chapters x {} saves = {} total).",
                em("⚠️", "[!]"), MAX_CHAPTERS, SNAPS_PER_CHAPTER, MAX_CHAPTERS * SNAPS_PER_CHAPTER
            );
            println!("    Use 'dup' to continue in a fresh project — your files carry over, snapshots don't.");
            return;
        }
    }
    // Pre-restore saves always use a blank era name, per spec.
    let raw_label = if is_pre_restore {
        String::new()
    } else {
        loop {
            let input = read_input("Name? ");
            if input.trim().eq_ignore_ascii_case("cancel") {
                println!("Save cancelled.");
                return;
            }
            if input.chars().count() > 26 {
                println!("Era name must be 26 characters or fewer.");
                continue;
            }
            break input;
        }
    };
    let final_label = if raw_label.is_empty() { String::new() } else { sanitize_name(&raw_label) };
    if !final_label.is_empty() && final_label != raw_label {
        println!("Saved as: {}", final_label);
    }
    if chapter_is_full {
        if proj_cfg.current_chapter == 0 {
            migrate_to_chapters(project_name, proj_cfg);
        } else {
            open_next_chapter(project_name, proj_cfg);
        }
    }
    // When the chapter just advanced, the new chapter is always empty so the first ID is 1.
    // When it didn't, highest_id + 1 is identical to what next_snapshot_id would return.
    let snap_id_within = if chapter_is_full { 1 } else { highest_id + 1 };
    let snap_name = if final_label.is_empty() {
        format!("{:02}", snap_id_within)
    } else {
        format!("{:02}_{}", snap_id_within, final_label)
    };
    let backup_dir = if proj_cfg.current_chapter == 0 {
        ensure_project_dir(project_name).join(&snap_name)
    } else {
        compute_chapter_dir(project_name, proj_cfg.current_chapter).join(&snap_name)
    };
    safe_create_dir_all(&backup_dir);
    println!("Saving snapshot '{}'...", snap_name);
    let mapping = copy_into_snapshot(&proj_cfg.paths, &backup_dir, &proj_cfg.mode);
    let loc_data = backup_dir.parent().unwrap().join(".location_data");
    safe_create_dir_all(&loc_data);
    safe_write(
        loc_data.join(snap_meta_filename(&snap_name)),
        serde_json::to_string_pretty(&mapping).expect("Failed to serialize snapshot mapping"),
    );
    save_project_config(project_name, proj_cfg);
    println!("{} Save complete!", em("✨", "*"));
}
fn do_restore_snapshot(snap: &SnapshotInfo) {
    let location_data = snap.path.parent().unwrap().join(".location_data");
    let snap_folder_name = snap.path.file_name().unwrap().to_string_lossy().to_string();
    let json_path = location_data.join(snap_meta_filename(&snap_folder_name));
    let data = match fs::read_to_string(&json_path) {
        Ok(d) => d,
        Err(_) => { println!("{} Could not find mapping file for this snapshot.", em("⚠️", "[!]")); return; }
    };
    let mapping: SnapshotMapping = match serde_json::from_str(&data) {
        Ok(m) => m,
        Err(_) => { println!("{} Could not parse mapping file.", em("⚠️", "[!]")); return; }
    };
    for (key, original_path_str) in &mapping.entries {
        let src = snap.path.join(key);
        let dest = PathBuf::from(original_path_str);
        if !src.exists() { continue; }
        if src.is_file() {
            if let Some(parent) = dest.parent() { safe_create_dir_all(parent); }
            safe_copy(&src, &dest);
        } else if src.is_dir() {
            copy_dir_recursive(&src, &dest);
        }
    }
    println!("{} Restore complete!", em("✨", "*"));
}
fn do_convert_snapshot(snap: &SnapshotInfo) {
    let location_data = snap.path.parent().unwrap().join(".location_data");
    let snap_folder_name = snap.path.file_name().unwrap().to_string_lossy().to_string();
    let json_path = location_data.join(snap_meta_filename(&snap_folder_name));
    let data = match fs::read_to_string(&json_path) {
        Ok(d) => d,
        Err(_) => { println!("{} Cannot read mapping file.", em("⚠️", "[!]")); return; }
    };
    let mapping: SnapshotMapping = match serde_json::from_str(&data) {
        Ok(m) => m,
        Err(_) => { println!("{} Cannot parse mapping file.", em("⚠️", "[!]")); return; }
    };
    let temp_path = snap.path.parent().unwrap()
        .join(format!("__grove_converting_{}", snap_folder_name));
    safe_create_dir_all(&temp_path);
    let new_mapping = match mapping.mode {
        Mode::Flat => {
            let original_paths: Vec<String> = mapping.entries.values().cloned().collect();
            let prefix = get_common_prefix(&original_paths);
            let mut lab_entries: BTreeMap<String, String> = BTreeMap::new();
            for (stored_name, original_path_str) in &mapping.entries {
                let src = snap.path.join(stored_name);
                if !src.exists() { continue; }
                let original = Path::new(original_path_str);
                let rel = relative_to_prefix(original, &prefix);
                let rel_str = rel.to_string_lossy().to_string();
                lab_entries.insert(rel_str.clone(), original_path_str.clone());
                let dest = temp_path.join(&rel);
                if src.is_file() {
                    if let Some(parent) = dest.parent() { safe_create_dir_all(parent); }
                    safe_copy(&src, &dest);
                } else if src.is_dir() { copy_dir_recursive(&src, &dest); }
            }
            SnapshotMapping { mode: Mode::Labyrinth, entries: lab_entries }
        }
        Mode::Labyrinth => {
            let mut flat_entries: BTreeMap<String, String> = BTreeMap::new();
            let mut counts: HashMap<String, usize> = HashMap::new();
            for (rel_str, original_path_str) in &mapping.entries {
                let src = snap.path.join(rel_str);
                if !src.exists() { continue; }
                let original_name = Path::new(original_path_str).file_name().unwrap()
                    .to_string_lossy().to_string();
                let stored_name = flat_stored_name(&original_name, &src, &mut counts);
                flat_entries.insert(stored_name.clone(), original_path_str.clone());
                let dest = temp_path.join(&stored_name);
                if src.is_file() { safe_copy(&src, &dest); }
                else if src.is_dir() { copy_dir_recursive(&src, &dest); }
            }
            SnapshotMapping { mode: Mode::Flat, entries: flat_entries }
        }
    };
    let bak_path = snap.path.with_file_name(format!("{}.bak", snap_folder_name));
    if let Err(e) = fs::rename(&snap.path, &bak_path) {
        println!("{} Could not stage original for replacement: {}", em("⚠️", "[!]"), e);
        safe_remove_dir_all(&temp_path);
        return;
    }
    if let Err(e) = fs::rename(&temp_path, &snap.path) {
        println!("{} Could not finalize conversion, restoring original: {}", em("⚠️", "[!]"), e);
        let _ = fs::rename(&bak_path, &snap.path);
        return;
    }
    safe_write(json_path,
        serde_json::to_string_pretty(&new_mapping).expect("Failed to serialize mapping"));
    safe_remove_dir_all(&bak_path);
    println!("{} Done!", em("✨", "*"));
}
// --- CHAPTER DELETE (gaps preserved — no renumbering) ---
/// Chapters screen: list chapters, handle selection and hidden delete.
/// Returns Some(chapter_number) when the user selects a chapter to view,
/// or None to cancel back to the restore screen.
///
/// The list number IS the chapter's real ID — gaps are shown honestly.
/// Typing "3" means Chapter 03 regardless of how many chapters precede it.
fn run_chapters_screen(project_name: &str, proj_cfg: &mut ProjectConfig) -> Option<usize> {
    loop {
        let chapter_nums = get_all_chapter_numbers(project_name);
        if chapter_nums.is_empty() { return None; }
        println!("\n--- Chapters ---");
        for &ch in &chapter_nums {
            // List number equals chapter ID — gaps appear naturally.
            println!("{}. Chapter {:02}", ch, ch);
        }
        let input = read_input("> ");
        let lower = input.trim().to_lowercase();
        if is_cancel(&lower) { return None; }
        // Hidden delete — "del", "delete", "del 3", "delete 1,3", etc.
        // Numbers are chapter IDs, not list positions.
        if let Some(arg) = parse_del_arg(&lower) {
            // Parse as real chapter IDs, reject any not present on disk.
            let parse_ids = |s: &str| -> Vec<usize> {
                s.split(',')
                    .filter_map(|p| p.trim().parse::<usize>().ok())
                    .filter(|n| chapter_nums.contains(n))
                    .collect()
            };
            let mut to_delete: Vec<usize> = if !arg.is_empty() {
                parse_ids(&arg)
            } else {
                let sel = read_input("Chapter(s) to delete (comma-separated, blank to cancel): ");
                if is_cancel(&sel) { continue; }
                parse_ids(&sel)
            };
            if to_delete.is_empty() { continue; }
            to_delete.sort();
            to_delete.dedup();
            if to_delete.len() >= chapter_nums.len() {
                println!("{} Cannot delete all chapters. Use the restore screen to remove individual snapshots.", em("⚠️", "[!]"));
                continue;
            }
            let names: Vec<String> = to_delete.iter()
                .map(|&ch| format!("Chapter {:02}", ch))
                .collect();
            println!("\nPermanently delete {}?", names.join(", "));
            let confirm = read_input("(y/n): ").to_lowercase();
            if is_yes(&confirm) {
                for &ch in &to_delete {
                    safe_remove_dir_all(compute_chapter_dir(project_name, ch));
                    println!("Chapter {:02} deleted.", ch);
                }
                // Chapters are never renumbered — gaps are intentional and preserve
                // structural memory ("that save in chapter 3" stays in chapter 3).
                let survivors = get_all_chapter_numbers(project_name);
                proj_cfg.current_chapter = survivors.last().copied().unwrap_or(0);
                save_project_config(project_name, proj_cfg);
                // Cascade: if the new latest chapter is also empty (e.g. user deleted
                // snapshots from it externally), keep trimming back until one has content.
                maybe_cleanup_empty_latest_chapter(project_name, proj_cfg);
                println!("{} Done.", em("✨", "*"));
            } else {
                println!("Delete cancelled.");
            }
            continue;
        }
        // Chapter selection — input is the real chapter ID, not a list position.
        if let Ok(n) = lower.parse::<usize>() {
            if chapter_nums.contains(&n) {
                return Some(n);
            }
        }
    }
}
// --- STATE MACHINES ---
fn run_files(project_name: &str, proj_cfg: &mut ProjectConfig) {
    let opts = &[("1", "add"), ("2", "delist"), ("3", "menu")];
    let print_list = |paths: &[String]| {
        if paths.is_empty() {
            println!("(No files tracked yet)\n");
        } else {
            for (i, p) in paths.iter().enumerate() { println!("{}. {}\n", i + 1, p); }
        }
    };
    let confirm_delist = |proj_cfg: &mut ProjectConfig, mut to_remove: Vec<usize>| -> bool {
        if to_remove.is_empty() { return false; }
        to_remove.sort_by(|a, b| b.cmp(a));
        to_remove.dedup();
        let names: Vec<String> = to_remove.iter().map(|&i| {
            Path::new(&proj_cfg.paths[i]).file_name().unwrap()
                .to_string_lossy().to_string()
        }).collect();
        println!("\nAre you sure you want to delist {}?", names.join(", "));
        let confirm = read_input("(y/n): ").to_lowercase();
        if is_yes(&confirm) {
            for i in to_remove { proj_cfg.paths.remove(i); }
            save_project_config(project_name, proj_cfg);
            println!("{} Delisted successfully.", em("✨", "*"));
            true
        } else {
            println!("Delist cancelled.");
            false
        }
    };
    loop {
        println!("\n=== {}FILES: {} ===", em("📁 ", ""), project_name);
        println!();
        print_list(&proj_cfg.paths);
        println!("----------------------");
        println!("1. Add");
        println!("2. Delist");
        println!("3. Menu");
        let choice = read_input("> ");
        // Inline delist: "delist 1,3" or "2 1,3" before handle_menu_choice
        {
            let lower = choice.trim().to_lowercase();
            let inline_arg = if lower.starts_with("delist ") {
                Some(lower["delist ".len()..].trim().to_string())
            } else if lower.starts_with("2 ") {
                Some(lower["2 ".len()..].trim().to_string())
            } else {
                None
            };
            if let Some(arg) = inline_arg {
                if !proj_cfg.paths.is_empty() {
                    let indices: Vec<usize> = arg.split(',')
                        .filter_map(|p| p.trim().parse::<usize>().ok())
                        .filter(|&i| i > 0 && i <= proj_cfg.paths.len())
                        .map(|i| i - 1)
                        .collect();
                    confirm_delist(proj_cfg, indices);
                }
                continue;
            }
        }
        if is_cancel(&choice) { return; }
        match handle_menu_choice(&choice, opts) {
            Ok(Some(cmd)) => match cmd.as_str() {
                "1" => {
                    let t = read_input("(1)file  or  (2)directory? ").to_lowercase();
                    if is_cancel(&t) { continue; }
                    let pick_file = matches!(t.as_str(), "1" | "f" | "file" | "files");
                    let pick_dir  = matches!(t.as_str(), "2" | "d" | "dir" | "directory" | "directories" | "folder" | "folders");
                    if pick_file {
                        #[cfg(not(target_os = "linux"))]
                        if let Some(selected) = FileDialog::new().pick_files() {
                            for file in selected {
                                let s = file.to_string_lossy().to_string();
                                if !proj_cfg.paths.contains(&s) { proj_cfg.paths.push(s); }
                            }
                            save_project_config(project_name, proj_cfg);
                            println!("{} Files added.", em("✨", "*"));
                        }
                        #[cfg(target_os = "linux")]
                        if let Some(selected) = tinyfiledialogs::open_file_dialog_multi("Select Files", "", None) {
                            for file in selected {
                                if !proj_cfg.paths.contains(&file) { proj_cfg.paths.push(file); }
                            }
                            save_project_config(project_name, proj_cfg);
                            println!("{} Files added.", em("✨", "*"));
                        }
                    } else if pick_dir {
                        #[cfg(not(target_os = "linux"))]
                        if let Some(selected) = FileDialog::new().pick_folders() {
                            for folder in selected {
                                let s = folder.to_string_lossy().to_string();
                                if !proj_cfg.paths.contains(&s) { proj_cfg.paths.push(s); }
                            }
                            save_project_config(project_name, proj_cfg);
                            println!("{} Folders added.", em("✨", "*"));
                        }
                        #[cfg(target_os = "linux")]
                        if let Some(folder) = tinyfiledialogs::select_folder_dialog("Select Folder", "") {
                            if !proj_cfg.paths.contains(&folder) { proj_cfg.paths.push(folder); }
                            save_project_config(project_name, proj_cfg);
                            println!("{} Folder added.", em("✨", "*"));
                        }
                    } else {
                        say_what();
                    }
                }
                "2" => {
                    if proj_cfg.paths.is_empty() { continue; }
                    println!("\n--- Delist Items ---");
                    print_list(&proj_cfg.paths);
                    let del_input = read_input("Enter number(s) comma-separated to delist: ");
                    if is_cancel(&del_input) { continue; }
                    let indices: Vec<usize> = del_input.split(',')
                        .filter_map(|p| p.trim().parse::<usize>().ok())
                        .filter(|&i| i > 0 && i <= proj_cfg.paths.len())
                        .map(|i| i - 1)
                        .collect();
                    confirm_delist(proj_cfg, indices);
                }
                "3" => break,
                _ => {}
            },
            Ok(None) => say_what(),
            Err(_) => {}
        }
    }
}
fn format_snap_label(idx: usize, era: &str) -> String {
    if era.is_empty() {
        idx.to_string()
    } else {
        format!("{} ({})", idx, era)
    }
}
/// Deletes the specified snapshots (by 0-based index) from `snapshots`, prompting
/// for confirmation first. Refreshes `snapshots` from disk on success.
/// Returns true if the caller should exit the restore screen (latest chapter cleaned up).
fn exec_delete(
    project_name: &str,
    viewing_chapter: usize,
    to_delete_raw: Vec<usize>,
    snapshots: &mut Vec<SnapshotInfo>,
    proj_cfg: &mut ProjectConfig,
) -> bool {
    let mut to_delete = to_delete_raw;
    to_delete.sort_by(|a, b| b.cmp(a));
    to_delete.dedup();
    if to_delete.is_empty() { return false; }
    let names: Vec<String> = to_delete.iter().map(|&i| {
        format_snap_label(i + 1, &snapshots[i].era)
    }).collect();
    println!("\nPermanently delete snapshot{}  {}?",
        if names.len() == 1 { "" } else { "s" }, names.join(", "));
    let confirm = read_input("(y/n): ").to_lowercase();
    if is_yes(&confirm) {
        for i in &to_delete {
            let snap = &snapshots[*i];
            let snap_folder_name = snap.path.file_name().unwrap().to_string_lossy().to_string();
            let loc = snap.path.parent().unwrap().join(".location_data");
            safe_remove_dir_all(&snap.path);
            let _ = fs::remove_file(loc.join(snap_meta_filename(&snap_folder_name)));
        }
        println!("{} Deleted.", em("✨", "*"));
        *snapshots = get_chapter_snapshots(project_name, viewing_chapter);
        // Only trigger empty-chapter cleanup when deleting from the current (latest) chapter.
        if proj_cfg.current_chapter > 0 && viewing_chapter == proj_cfg.current_chapter {
            return maybe_cleanup_empty_latest_chapter(project_name, proj_cfg);
        }
    } else {
        println!("Delete cancelled.");
    }
    false
}
/// Prints the restore screen header, adjusting title for sub-mode and chapter context.
fn print_restore_header(
    in_convert_mode: bool,
    in_delete_mode: bool,
    show_chapter: bool,
    viewing_chapter: usize,
) {
    let mode_str = if in_convert_mode { "Convert" } else if in_delete_mode { "Delete" } else { "Restore" };
    if show_chapter {
        println!("\n--- {} Snapshot  ·  Chapter {:02} ---", mode_str, viewing_chapter);
    } else {
        println!("\n--- {} Snapshot ---", mode_str);
    }
}
fn handle_snapshot_restore(
    project_name: &str,
    snap: &SnapshotInfo,
    snap_label: &str,
    proj_cfg: &mut ProjectConfig,
) -> bool {
    println!("\nAbout to restore save {} of {}.", snap_label, project_name);
    println!("\n1. Save current files, then restore");
    println!("2. Restore without saving");
    let choice = read_input("\n(blank to cancel): ").trim().to_lowercase();
    match choice.as_str() {
        "1" => {
            let confirm = read_input("Save current files and then restore selected save? (y/n): ")
                .trim().to_lowercase();
            if !is_yes(&confirm) {
                println!("Restoration cancelled.");
                return false;
            }
            println!("Saving current state first...");
            do_save(project_name, proj_cfg, true);
        }
        "2" => {
            let confirm = read_input("WARNING: Restore backup without saving current files? (y/n): ")
                .trim().to_lowercase();
            if !is_yes(&confirm) {
                println!("Restoration cancelled.");
                return false;
            }
            println!("Proceeding without saving...");
        }
        _ => {
            println!("Restoration cancelled.");
            return false;
        }
    }
    println!("Restoring snapshot {}...", snap_label);
    do_restore_snapshot(snap);
    true
}
fn run_restore(project_name: &str, proj_cfg: &mut ProjectConfig) {
    let mut chapter_nums = get_all_chapter_numbers(project_name);
    let mut in_chapter_mode = !chapter_nums.is_empty();
    let mut latest_chapter = if in_chapter_mode { *chapter_nums.last().unwrap() } else { 0 };
    let mut viewing_chapter = latest_chapter;
    let mut in_convert_mode = false;
    let mut in_delete_mode = false;
    let mut snapshots = get_chapter_snapshots(project_name, viewing_chapter);
    loop {
        // Chapter header context only shown when more than one chapter exists.
        let show_chapter = chapter_nums.len() > 1;
        print_restore_header(in_convert_mode, in_delete_mode, show_chapter, viewing_chapter);
        if snapshots.is_empty() {
            if !in_chapter_mode { println!("No snapshots found for this project."); return; }
            println!("(No snapshots in this chapter)");
        } else {
            for (i, snap) in snapshots.iter().enumerate() {
                let ts = format_snapshot_time(&snap.path);
                let mode_tag = if snap.mode != proj_cfg.mode {
                    if snap.mode == Mode::Labyrinth { " (Labyrinth)" } else { " (Flat)" }
                } else { "" };
                let left = if snap.era.is_empty() {
                    format!("{}. {}", i + 1, mode_tag.trim_start())
                } else {
                    format!("{}. {}{}", i + 1, snap.era, mode_tag)
                };
                println!("{:<45}{}", left, ts);
            }
        }
        let choice = read_input("> ");
        let lower = choice.trim().to_lowercase();
        // Blank exits sub-modes without leaving the restore screen.
        if (in_convert_mode || in_delete_mode) && lower.is_empty() {
            in_convert_mode = false;
            in_delete_mode = false;
            continue;
        }
        if lower.is_empty() || lower == "cancel" || lower == "back" { return; }
        // Hidden commands only available in normal (non-sub-mode) view.
        if !in_convert_mode && !in_delete_mode {
            // del / delete with optional inline numbers
            if let Some(arg) = parse_del_arg(&lower) {
                if arg.is_empty() {
                    in_delete_mode = true; continue;
                }
                let indices: Vec<usize> = arg.split(',')
                    .filter_map(|p| p.trim().parse::<usize>().ok())
                    .filter(|&i| i > 0 && i <= snapshots.len())
                    .map(|i| i - 1).collect();
                if exec_delete(project_name, viewing_chapter, indices, &mut snapshots, proj_cfg) { return; }
                continue;
            }
            // convert with optional inline number
            let conv_arg = if lower.starts_with("convert ") {
                Some(lower["convert ".len()..].trim().to_string())
            } else { None };
            if let Some(arg) = conv_arg {
                if let Ok(idx) = arg.parse::<usize>() {
                    if idx > 0 && idx <= snapshots.len() {
                        let snap = snapshots[idx - 1].clone();
                        let snap_label = format_snap_label(idx, &snap.era);
                        let target_mode = match snap.mode {
                            Mode::Flat => Mode::Labyrinth,
                            Mode::Labyrinth => Mode::Flat,
                        };
                        let confirm = read_input(&format!(
                            "Convert snapshot {} to {}? (y/n): ", snap_label, target_mode
                        )).to_lowercase();
                        if is_yes(&confirm) {
                            println!("Please wait: converting...");
                            do_convert_snapshot(&snap);
                            snapshots = get_chapter_snapshots(project_name, viewing_chapter);
                        } else {
                            println!("Conversion cancelled.");
                        }
                    }
                }
                continue;
            }
            if lower == "convert" { in_convert_mode = true; continue; }
            // Chapter navigation
            if in_chapter_mode {
                if lower == "chapters" {
                    let selected = run_chapters_screen(project_name, proj_cfg);
                    // Re-derive chapter state — deletions may have happened inside.
                    chapter_nums = get_all_chapter_numbers(project_name);
                    in_chapter_mode = !chapter_nums.is_empty();
                    latest_chapter = if in_chapter_mode { *chapter_nums.last().unwrap() } else { 0 };
                    viewing_chapter = match selected {
                        Some(n) if chapter_nums.contains(&n) => n,
                        _ => latest_chapter,
                    };
                    snapshots = get_chapter_snapshots(project_name, viewing_chapter);
                    continue;
                }
                if let Some(suffix) = lower.strip_prefix("chapter") {
                    if let Ok(n) = suffix.trim().parse::<usize>() {
                        if chapter_nums.contains(&n) {
                            viewing_chapter = n;
                            snapshots = get_chapter_snapshots(project_name, viewing_chapter);
                        } else {
                            println!("{} Chapter {:02} not found.", em("⚠️", "[!]"), n);
                        }
                        continue;
                    }
                }
            }
        }
        // Delete mode — waiting for snapshot numbers.
        if in_delete_mode {
            let indices: Vec<usize> = lower.split(',')
                .filter_map(|p| p.trim().parse::<usize>().ok())
                .filter(|&i| i > 0 && i <= snapshots.len())
                .map(|i| i - 1).collect();
            if exec_delete(project_name, viewing_chapter, indices, &mut snapshots, proj_cfg) { return; }
            continue;
        }
        // Numeric snapshot selection.
        if let Ok(idx) = lower.parse::<usize>() {
            if idx > 0 && idx <= snapshots.len() {
                let snap = snapshots[idx - 1].clone();
                let snap_label = format_snap_label(idx, &snap.era);
                if in_convert_mode {
                    let target_mode = match snap.mode {
                        Mode::Flat => Mode::Labyrinth,
                        Mode::Labyrinth => Mode::Flat,
                    };
                    let confirm = read_input(&format!(
                        "Convert snapshot {} to {}? (y/n): ", snap_label, target_mode
                    )).to_lowercase();
                    if is_yes(&confirm) {
                        println!("Please wait: converting...");
                        do_convert_snapshot(&snap);
                        snapshots = get_chapter_snapshots(project_name, viewing_chapter);
                    } else {
                        println!("Conversion cancelled.");
                    }
                    continue; // explicit continue — prevents "what?" after a valid convert action
                } else if handle_snapshot_restore(project_name, &snap, &snap_label, proj_cfg) {
                    return;
                }
                continue; // after a cancelled restore, loop back cleanly
            }
        }
        // Nothing matched — including mistyped hidden commands and out-of-range numbers.
        say_what();
    }
}
fn run_project(project_name: &str, config: &mut GroveConfig) {
    if project_name.is_empty() {
        println!("{} Attempted to open a project with no name. Returning to menu.", em("⚠️", "[!]"));
        return;
    }
    config.last_opened_project = Some(project_name.to_string());
    save_config(config);
    let opts = &[
        ("1", "save"), ("2", "files"), ("3", "restore"), ("4", "grove"), ("5", "menu"),
    ];
    let mut proj_cfg = load_project_config(project_name);
    reconcile_current_chapter(project_name, &mut proj_cfg);
    heal_conversion_orphans(project_name);
    loop {
        println!("\n=== {}PROJECT: {} ===", em("🛠️  ", ""), project_name);
        // Intentional per-render read_dir — stays current if the user deletes a snapshot
        // externally while the app is open (the Soul of this project). read_dir here only
        // reads filenames, never opens files, so it's effectively free at ≤99 entries.
        let actual_saves = next_snapshot_id(project_name, proj_cfg.current_chapter).saturating_sub(1);
        let nearly_full = actual_saves + 3 > SNAPS_PER_CHAPTER;
        if nearly_full {
            // current_chapter jumps 0 → 2 on first migration and never lands on 1
            // under normal operation. However, reconcile_current_chapter can set it
            // to 1 if the user manually deletes Chapter 02. The explicit "> 1" check
            // is therefore required and intentional — do not simplify to "> 0".
            let chapter_part = if proj_cfg.current_chapter > 1 {
                format!("Chapter {}  |  ", proj_cfg.current_chapter)
            } else {
                String::new()
            };
            let mode_part = if proj_cfg.mode == Mode::Labyrinth {
                "Mode: Labyrinth  |  ".to_string()
            } else {
                String::new()
            };
            println!("{}{}Saves: {}/{}", mode_part, chapter_part, actual_saves, SNAPS_PER_CHAPTER);
        } else if proj_cfg.mode == Mode::Labyrinth {
            println!("Mode: Labyrinth");
        }
        println!();
        println!("1. Save");
        println!("2. Files");
        println!("3. Restore");
        println!("4. Grove");
        println!("5. Menu");
        let choice = read_input("> ");
        match handle_menu_choice(&choice, opts) {
            Ok(Some(cmd)) => match cmd.as_str() {
                "1" => do_save(project_name, &mut proj_cfg, false),
                "2" => run_files(project_name, &mut proj_cfg),
                "3" => run_restore(project_name, &mut proj_cfg),
                "4" => { let _ = open::that(ensure_project_dir(project_name)); }
                "5" => break,
                _ => {}
            },
            Ok(None) => {
                let lower = choice.trim().to_lowercase();
                if lower == "labyrinth" || lower == "lab" {
                    proj_cfg.mode = match proj_cfg.mode {
                        Mode::Flat => Mode::Labyrinth,
                        Mode::Labyrinth => Mode::Flat,
                    };
                    save_project_config(project_name, &proj_cfg);
                    println!("{} Mode changed to {}.", em("✨", "*"), proj_cfg.mode);
                } else if lower == "dup" || lower == "duplicate" {
                    let raw = read_input("Enter name for duplicate project (blank to cancel): ");
                    if !is_cancel(&raw) && !raw.is_empty() {
                        let new_name = sanitize_name(&raw);
                        if new_name.is_empty() {
                            println!("{} That name contains no valid characters.", em("⚠️", "[!]"));
                        } else if compute_project_dir(&new_name).exists() {
                            println!("{} A project named '{}' already exists.", em("⚠️", "[!]"), new_name);
                        } else {
                            ensure_project_dir(&new_name);
                            let new_cfg = ProjectConfig {
                                mode: proj_cfg.mode,
                                paths: proj_cfg.paths.clone(),
                                current_chapter: 0,
                            };
                            save_project_config(&new_name, &new_cfg);
                            println!("{} Duplicate project '{}' created. Opening it now.", em("✨", "*"), new_name);
                            run_project(&new_name, config);
                            break;
                        }
                    }
                } else if lower == "delete" {
                    println!("\nPermanently delete project '{}' and all its snapshots?", project_name);
                    let confirm = read_input("(y/n): ").to_lowercase();
                    if is_yes(&confirm) {
                        safe_remove_dir_all(compute_project_dir(project_name));
                        if config.last_opened_project.as_deref() == Some(project_name) {
                            config.last_opened_project = None;
                            save_config(config);
                        }
                        println!("{} Project '{}' deleted.", em("✨", "*"), project_name);
                        break;
                    } else {
                        println!("Delete cancelled.");
                    }
                } else {
                    say_what();
                }
            }
            Err(_) => {}
        }
    }
}
fn create_project_flow(prefilled_name: Option<String>, config: &mut GroveConfig) {
    let raw = match prefilled_name {
        Some(n) if !n.is_empty() => n,
        _ => {
            let n = read_input("Enter new project name (blank to return): ");
            if is_cancel(&n) { return; }
            n
        }
    };
    if raw.is_empty() { return; }
    let name = sanitize_name(&raw);
    if name.is_empty() { println!("{} That name contains no valid characters.", em("⚠️", "[!]")); return; }
    if compute_project_dir(&name).exists() {
        println!("{} A project named '{}' already exists.", em("⚠️", "[!]"), name);
        return;
    }
    ensure_project_dir(&name);
    let mut proj_cfg = ProjectConfig::default();
    save_project_config(&name, &mut proj_cfg);
    run_files(&name, &mut proj_cfg);
    run_project(&name, config);
}
fn run_menu(config: &mut GroveConfig) {
    let opts = &[
        ("1", "create"), ("2", "grove root"), ("3", "open project"), ("4", "exit"),
    ];
    loop {
        println!("\n=== {}GROVE MENU ===", em("🌱 ", ""));
        println!();
        println!("1. Create");
        println!("2. Grove Root");
        println!("3. Open Project");
        println!("4. Exit");
        let choice = read_input("> ");
        match handle_menu_choice(&choice, opts) {
            Ok(Some(cmd)) => match cmd.as_str() {
                "4" => { println!("Goodbye! {}", em("🌲✨", "")); std::process::exit(0); }
                "1" => {
                    let trimmed = choice.trim();
                    let prefilled = if trimmed.to_lowercase().starts_with("create") {
                        let after = trimmed["create".len()..].trim().to_string();
                        if after.is_empty() { None } else { Some(after) }
                    } else { None };
                    create_project_flow(prefilled, config);
                }
                "2" => { let _ = open::that(ensure_projects_root()); }
                "3" => {
                    let Ok(entries) = fs::read_dir(ensure_projects_root()) else { continue; };
                    let mut projects: Vec<String> = entries.flatten()
                        .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .map(|e| e.file_name().to_string_lossy().to_string()).collect();
                    projects.sort();
                    if projects.is_empty() { println!("No projects found. Create one first!"); continue; }
                    println!("\n--- Projects ---");
                    println!();
                    for (i, p) in projects.iter().enumerate() { println!("{}. {}", i + 1, p); }
                    println!();
                    let proj_choice = read_input("Enter project number or name (blank to return): ");
                    if is_cancel(&proj_choice) { continue; }
                    let opts_owned: Vec<(String, String)> = projects.iter().enumerate()
                        .map(|(i, p)| ((i + 1).to_string(), p.to_lowercase())).collect();
                    let opts_ref: Vec<(&str, &str)> = opts_owned.iter()
                        .map(|(n, p)| (n.as_str(), p.as_str())).collect();
                    match handle_menu_choice(&proj_choice, &opts_ref) {
                        Ok(Some(num)) => {
                            let idx: usize = num.parse().expect("Matched option must be a number");
                            run_project(&projects[idx - 1], config);
                        }
                        Ok(None) => say_what(),
                        Err(_) => {}
                    }
                }
                _ => {}
            },
            Ok(None) => {
                let trimmed = choice.trim();
                let lower = trimmed.to_lowercase();
                if lower == "grove" { let _ = open::that(ensure_projects_root()); continue; }
                let potential_name = if lower.starts_with("open ") {
                    trimmed["open ".len()..].trim().to_string()
                } else {
                    trimmed.to_string()
                };
                // Case-insensitive lookup: scan disk for the real folder name so
                // "myproject" finds "MyProject" on Linux (case-sensitive filesystem).
                // The project always opens under its actual on-disk name.
                let resolved_name = if !potential_name.is_empty() {
                    let needle = potential_name.to_lowercase();
                    fs::read_dir(ensure_projects_root()).ok()
                        .and_then(|entries| {
                            entries.flatten()
                                .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                                .find(|e| e.file_name().to_string_lossy().to_lowercase() == needle)
                                .map(|e| e.file_name().to_string_lossy().to_string())
                        })
                } else {
                    None
                };
                if let Some(name) = resolved_name {
                    run_project(&name, config);
                } else if !trimmed.is_empty() {
                    say_what();
                }
            }
            Err(_) => {}
        }
    }
}
fn main() {
    let mut config = load_config();
    let mut args = env::args().skip(1);
    let mut started_via_arg = false;
    if let Some(raw_cmd) = args.next() {
        started_via_arg = true;
        let cmd = raw_cmd.to_lowercase();
        let arg2 = args.next();
        match cmd.as_str() {
            "create" => {
                let prefilled = arg2.filter(|n| !n.is_empty());
                create_project_flow(prefilled, &mut config);
            }
            "save" | "grove" | "restore" => {
                let is_grove_cmd = cmd == "grove";
                let target_project = if let Some(n) = arg2 {
                    let n = sanitize_name(&n);
                    let proj_path = compute_projects_root().join(&n);
                    if !n.is_empty() && proj_path.exists() && proj_path != *compute_projects_root() { Some(n) }
                    else { println!("{} Project '{}' does not exist.", em("⚠️", "[!]"), n); run_menu(&mut config); return; }
                } else if is_grove_cmd { None }
                else { config.last_opened_project.clone() };
                if is_grove_cmd && target_project.is_none() {
                    let _ = open::that(ensure_projects_root());
                    run_menu(&mut config);
                    return;
                }
                if let Some(proj) = target_project {
                    let mut proj_cfg = load_project_config(&proj);
                    if cmd == "save" {
                        do_save(&proj, &mut proj_cfg, false);
                        run_project(&proj, &mut config);
                    } else if is_grove_cmd {
                        let _ = open::that(ensure_project_dir(&proj));
                        run_project(&proj, &mut config);
                    } else if cmd == "restore" {
                        run_restore(&proj, &mut proj_cfg);
                        run_project(&proj, &mut config);
                    }
                } else {
                    println!("{} No project is currently open. Please select one from the menu.", em("⚠️", "[!]"));
                    run_menu(&mut config);
                    return;
                }
            }
            _ => {
                say_what();
                std::process::exit(0);
            }
        }
    }
    if !started_via_arg {
        if let Some(last) = config.last_opened_project.clone() {
            let proj_path = compute_projects_root().join(&last);
            if !last.is_empty() && proj_path.exists() && proj_path != *compute_projects_root() {
                println!("Welcome back! Opening last project: {}", last);
                run_project(&last, &mut config);
            } else {
                if last.is_empty() {
                    println!("{} No valid project was last open. Starting fresh.", em("⚠️", "[!]"));
                } else {
                    println!("{} Last project '{}' no longer exists. Starting fresh.", em("⚠️", "[!]"), last);
                }
                config.last_opened_project = None;
                save_config(&config);
            }
        }
    }
    run_menu(&mut config);
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // sanitize_name
    // -----------------------------------------------------------------------
    #[test]
    fn sanitize_normal_name_unchanged() {
        assert_eq!(sanitize_name("my_project"), "my_project");
    }

    #[test]
    fn sanitize_reserved_name_bare() {
        assert!(sanitize_name("NUL").ends_with('_'));
    }

    #[test]
    fn sanitize_reserved_name_with_extension() {
        assert!(sanitize_name("CON.txt").ends_with('_'));
    }

    #[test]
    fn sanitize_reserved_case_insensitive() {
        assert!(sanitize_name("nul").ends_with('_'));
        assert!(sanitize_name("Con").ends_with('_'));
        assert!(sanitize_name("cOm1").ends_with('_'));
    }

    #[test]
    fn sanitize_all_reserved_names() {
        let reserved = ["CON","PRN","AUX","NUL",
                        "COM1","COM2","COM3","COM4","COM5",
                        "COM6","COM7","COM8","COM9",
                        "LPT1","LPT2","LPT3","LPT4","LPT5",
                        "LPT6","LPT7","LPT8","LPT9"];
        for name in &reserved {
            assert!(sanitize_name(name).ends_with('_'), "{} should be suffixed", name);
        }
    }

    #[test]
    fn sanitize_replaces_illegal_chars() {
        let result = sanitize_name("my/file:name<>|*?\"");
        for ch in ['/', '\\', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!result.contains(ch), "should not contain '{}'", ch);
        }
    }

    #[test]
    fn sanitize_trims_whitespace() {
        assert_eq!(sanitize_name("  project  "), "project");
    }

    #[test]
    fn sanitize_non_reserved_name_with_numbers_unchanged() {
        assert_eq!(sanitize_name("save_01"), "save_01");
    }

    // -----------------------------------------------------------------------
    // flat_stored_name
    // -----------------------------------------------------------------------
    #[test]
    fn flat_name_no_collision() {
        let mut counts = HashMap::new();
        assert_eq!(flat_stored_name("notes.txt", Path::new("notes.txt"), &mut counts), "notes.txt");
    }

    #[test]
    fn flat_name_collision_increments() {
        let mut counts = HashMap::new();
        let src = Path::new("notes.txt");
        flat_stored_name("notes.txt", src, &mut counts);
        assert_eq!(flat_stored_name("notes.txt", src, &mut counts), "notes_2.txt");
        assert_eq!(flat_stored_name("notes.txt", src, &mut counts), "notes_3.txt");
    }

    #[test]
    fn flat_name_no_extension() {
        let mut counts = HashMap::new();
        let src = Path::new("Makefile");
        flat_stored_name("Makefile", src, &mut counts);
        assert_eq!(flat_stored_name("Makefile", src, &mut counts), "Makefile_2");
    }

    #[test]
    fn flat_name_different_files_dont_collide() {
        let mut counts = HashMap::new();
        assert_eq!(flat_stored_name("a.txt", Path::new("a.txt"), &mut counts), "a.txt");
        assert_eq!(flat_stored_name("b.txt", Path::new("b.txt"), &mut counts), "b.txt");
    }

    #[test]
    fn flat_name_collision_many() {
        let mut counts = HashMap::new();
        let src = Path::new("file.rs");
        for _ in 0..5 { flat_stored_name("file.rs", src, &mut counts); }
        assert_eq!(flat_stored_name("file.rs", src, &mut counts), "file_6.rs");
    }

    // -----------------------------------------------------------------------
    // get_common_prefix
    // -----------------------------------------------------------------------
    #[test]
    fn common_prefix_shared_parent() {
        let paths = vec![
            "/home/user/project/src/main.rs".to_string(),
            "/home/user/project/src/lib.rs".to_string(),
        ];
        assert_eq!(get_common_prefix(&paths), PathBuf::from("/home/user/project/src"));
    }

    #[test]
    fn common_prefix_no_common() {
        let paths = vec![
            "/home/user/project/file.rs".to_string(),
            "/other/thing/file.rs".to_string(),
        ];
        assert!(get_common_prefix(&paths).components().count() <= 1);
    }

    #[test]
    fn common_prefix_single_path() {
        let paths = vec!["/home/user/file.txt".to_string()];
        assert_eq!(get_common_prefix(&paths), PathBuf::from("/home/user"));
    }

    #[test]
    fn common_prefix_empty_input() {
        assert_eq!(get_common_prefix(&[]), PathBuf::new());
    }

    #[test]
    fn common_prefix_diverges_at_second_component() {
        let paths = vec![
            "/home/alice/file.txt".to_string(),
            "/home/bob/file.txt".to_string(),
        ];
        assert_eq!(get_common_prefix(&paths), PathBuf::from("/home"));
    }

    #[test]
    fn common_prefix_identical_paths() {
        let paths = vec![
            "/home/user/file.txt".to_string(),
            "/home/user/file.txt".to_string(),
        ];
        assert_eq!(get_common_prefix(&paths), PathBuf::from("/home/user"));
    }

    // -----------------------------------------------------------------------
    // handle_menu_choice
    // -----------------------------------------------------------------------
    #[test]
    fn menu_by_number() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("1", opts), Ok(Some("1".to_string())));
    }

    #[test]
    fn menu_by_name() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("save", opts), Ok(Some("1".to_string())));
    }

    #[test]
    fn menu_number_and_name_agree() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("1 save", opts), Ok(Some("1".to_string())));
    }

    #[test]
    fn menu_name_and_number_agree_reversed() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("save 1", opts), Ok(Some("1".to_string())));
    }

    #[test]
    fn menu_number_and_name_conflict_errors() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("1 files", opts), Err(()));
    }

    #[test]
    fn menu_no_match_returns_none() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("grove", opts), Ok(None));
    }

    #[test]
    fn menu_number_priority_over_name() {
        // A project named "1" must lose to menu item numbered "1" per spec.
        let opts = &[("1", "save"), ("2", "1")];
        assert_eq!(handle_menu_choice("1", opts), Ok(Some("1".to_string())));
    }

    #[test]
    fn menu_case_insensitive_name() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("SAVE", opts), Ok(Some("1".to_string())));
        assert_eq!(handle_menu_choice("Files", opts), Ok(Some("2".to_string())));
    }

    #[test]
    fn menu_empty_input_no_match() {
        let opts = &[("1", "save"), ("2", "files")];
        assert_eq!(handle_menu_choice("", opts), Ok(None));
    }

    // -----------------------------------------------------------------------
    // is_cancel
    // -----------------------------------------------------------------------
    #[test]
    fn is_cancel_empty_string() { assert!(is_cancel("")); }

    #[test]
    fn is_cancel_whitespace_only() { assert!(is_cancel("   ")); }

    #[test]
    fn is_cancel_cancel_word() {
        assert!(is_cancel("cancel"));
        assert!(is_cancel("CANCEL"));
        assert!(is_cancel("Cancel"));
    }

    #[test]
    fn is_cancel_back_word() {
        assert!(is_cancel("back"));
        assert!(is_cancel("BACK"));
    }

    #[test]
    fn is_cancel_normal_inputs_not_cancel() {
        assert!(!is_cancel("save"));
        assert!(!is_cancel("1"));
        assert!(!is_cancel("grove"));
    }

    // -----------------------------------------------------------------------
    // is_yes
    // -----------------------------------------------------------------------
    #[test]
    fn is_yes_accepts_y_and_yes() {
        assert!(is_yes("y"));
        assert!(is_yes("yes"));
    }

    #[test]
    fn is_yes_rejects_other() {
        assert!(!is_yes("n"));
        assert!(!is_yes("no"));
        assert!(!is_yes("Y"));  // caller is responsible for lowercasing before is_yes
        assert!(!is_yes(""));
    }

    // -----------------------------------------------------------------------
    // format_snap_label
    // -----------------------------------------------------------------------
    #[test]
    fn snap_label_with_era() {
        assert_eq!(format_snap_label(3, "flibberty"), "3 (flibberty)");
    }

    #[test]
    fn snap_label_without_era() {
        assert_eq!(format_snap_label(5, ""), "5");
    }

    #[test]
    fn snap_label_idx_one() {
        assert_eq!(format_snap_label(1, "alpha"), "1 (alpha)");
    }

    // -----------------------------------------------------------------------
    // strip_absolute_prefix / relative_to_prefix
    // -----------------------------------------------------------------------
    #[test]
    fn strip_absolute_removes_root_slash() {
        let stripped = strip_absolute_prefix(Path::new("/home/user/file.txt"));
        assert!(!stripped.to_string_lossy().starts_with('/'));
        assert!(stripped.to_string_lossy().contains("home"));
    }

    #[test]
    fn relative_to_prefix_basic() {
        let path = Path::new("/home/user/project/main.rs");
        assert_eq!(
            relative_to_prefix(path, &PathBuf::from("/home/user/project")),
            PathBuf::from("main.rs")
        );
    }

    #[test]
    fn relative_to_prefix_empty_prefix_falls_back_to_strip() {
        let path = Path::new("/home/user/file.txt");
        let result = relative_to_prefix(path, &PathBuf::new());
        assert!(!result.to_string_lossy().starts_with('/'));
    }

    #[test]
    fn relative_to_prefix_mismatch_falls_back() {
        let path = Path::new("/other/path/file.txt");
        let result = relative_to_prefix(path, &PathBuf::from("/home/user"));
        assert!(!result.to_string_lossy().starts_with('/'));
    }

    // -----------------------------------------------------------------------
    // parse_del_arg
    // -----------------------------------------------------------------------
    #[test]
    fn parse_del_arg_with_space_delete() {
        assert_eq!(parse_del_arg("delete 1,3"), Some("1,3".to_string()));
    }

    #[test]
    fn parse_del_arg_with_space_del() {
        assert_eq!(parse_del_arg("del 2"), Some("2".to_string()));
    }

    #[test]
    fn parse_del_arg_bare_delete() {
        assert_eq!(parse_del_arg("delete"), Some(String::new()));
    }

    #[test]
    fn parse_del_arg_bare_del() {
        assert_eq!(parse_del_arg("del"), Some(String::new()));
    }

    #[test]
    fn parse_del_arg_no_match() {
        assert_eq!(parse_del_arg("restore"), None);
        assert_eq!(parse_del_arg(""), None);
    }

    // -----------------------------------------------------------------------
    // snap_meta_filename
    // -----------------------------------------------------------------------
    #[test]
    fn snap_meta_filename_basic() {
        assert_eq!(snap_meta_filename("02_flibberty"), "02_flibberty.json");
    }

    #[test]
    fn snap_meta_filename_no_era() {
        assert_eq!(snap_meta_filename("06"), "06.json");
    }
}
