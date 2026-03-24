use std::fs;
use std::io;
use std::path::{Path};
use std::sync::OnceLock;

#[cfg(not(target_os = "windows"))]
use std::path::{PathBuf};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;



// The name of the user data directory we deliberately leave behind.
const GROVE_ROOT_DIR: &str = "Grove Root";

// Spawned cmd windows should be invisible.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ============================================================
//  Emoji support
// ============================================================

static EMOJI_OK: OnceLock<bool> = OnceLock::new();

fn emoji_supported() -> bool {
    #[cfg(target_os = "windows")]
    {
        if std::env::var("WT_SESSION").is_ok() { return true; }
        if let Ok(tp) = std::env::var("TERM_PROGRAM") {
            let tp = tp.to_lowercase();
            if tp.contains("vscode") || tp.contains("hyper") || tp.contains("wezterm") {
                return true;
            }
        }
        if std::env::var("ConEmuANSI").map(|v| v.eq_ignore_ascii_case("on")).unwrap_or(false) {
            return true;
        }
        false
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(tp) = std::env::var("TERM_PROGRAM") {
            let tp = tp.to_lowercase();
            if tp.contains("vscode") || tp.contains("iterm")
               || tp.contains("hyper") || tp.contains("wezterm") {
                return true;
            }
        }
        if let Ok(term) = std::env::var("TERM") {
            if term == "xterm-kitty" || term.contains("alacritty") {
                return true;
            }
        }
        if let Ok(ct) = std::env::var("COLORTERM") {
            let ct = ct.to_lowercase();
            if ct == "truecolor" || ct == "24bit" {
                return true;
            }
        }
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

// ============================================================
//  Shell config
// ============================================================

#[cfg(not(target_os = "windows"))]
fn shell_config_path(home_dir: &Path) -> Option<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.contains("zsh") {
        // Respect $ZDOTDIR if set; fall back to home.
        let zsh_home = std::env::var("ZDOTDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir.to_path_buf());
        Some(zsh_home.join(".zshrc"))
    } else if shell.contains("fish") {
        None
    } else {
        #[cfg(target_os = "macos")]
        { Some(home_dir.join(".bash_profile")) }
        // .profile is sourced by login shells (SSH, display managers) and sh — wider coverage than .bashrc.
        #[cfg(not(target_os = "macos"))]
        { Some(home_dir.join(".profile")) }
    }
}

// ============================================================
//  Entry point
// ============================================================

fn main() {
    if let Err(e) = run() {
        eprintln!("\n{} Uninstallation failed: {}", em("❌", "!!"), e);
        println!("\nPress Enter to exit...");
        let mut input = String::new();
        io::stdin().read_line(&mut input).ok();
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", em("=== 🌱 Grove Uninstaller ===", "=== Grove Uninstaller ==="));

    // On Windows, detect whether we need to relocate out of ~/.grove before
    // we can delete it. The relocated copy is launched with --relocated so it
    // knows to skip this check and to clean itself up at the end.
    #[cfg(target_os = "windows")]
    let is_relocated = std::env::args().any(|a| a == "--relocated");

    #[cfg(not(target_os = "windows"))]
    let is_relocated = false;

    let home_dir = dirs::home_dir().ok_or("Could not find user home directory")?;
    let grove_dir = home_dir.join(".grove");
    let bin_dir = grove_dir.join("bin");

    // If we are currently running from inside ~/.grove, copy ourselves to
    // %TEMP% and relaunch from there. Windows locks running executables, so
    // we must be outside the tree before we can delete it.
    #[cfg(target_os = "windows")]
    if !is_relocated && running_inside_grove_dir(&grove_dir) {
        relocate_and_relaunch()?;
        unreachable!(); // relocate_and_relaunch always exits
    }

    // 1. Remove launcher integration (before touching the binary)
    #[cfg(target_os = "linux")]
    remove_linux_desktop_entry(&home_dir)?;

    #[cfg(target_os = "macos")]
    remove_mac_app_bundle(&home_dir)?;

    #[cfg(target_os = "windows")]
    remove_windows_start_menu_shortcut()?;

    // 2. Strip the PATH entry
    remove_from_user_path(&home_dir, &bin_dir)?;

    // 3. Remove ~/.grove, skipping Grove Root which belongs to the user.
    remove_grove_dir(&grove_dir)?;

    println!("\n{}", em("👋 Grove has been uninstalled. Goodbye!", "Grove has been uninstalled. Goodbye!"));
    println!("\nPress Enter to exit...");
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();

    // If we're the relocated temp copy, schedule our own deletion now that
    // the user has pressed Enter and we're about to exit.
    #[cfg(target_os = "windows")]
    if is_relocated {
        schedule_self_deletion();
    }

    Ok(())
}

// ============================================================
//  Directory / file removal
// ============================================================

fn remove_file_if_exists(
    path: &Path,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|e| format!("Failed to remove {}: {}", label, e))?;
        println!("{} Removed {}", em("✅", "OK"), label);
    }
    Ok(())
}

/// Removes everything inside ~/.grove except the "Grove Root" user data
/// directory. Then attempts to remove ~/.grove itself — this will succeed
/// if Grove Root is absent, and silently fail if it is present (by design).
fn remove_grove_dir(grove_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !grove_dir.exists() {
        return Ok(());
    }

    let grove_root = grove_dir.join(GROVE_ROOT_DIR);
    let preserve = grove_root.exists();

    // Remove every entry inside ~/.grove except Grove Root.
    let entries = fs::read_dir(grove_dir)
        .map_err(|e| format!("Failed to read {}: {}", grove_dir.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();

        // Leave Grove Root exactly where it is.
        if path == grove_root {
            continue;
        }

        if path.is_dir() {
            fs::remove_dir_all(&path)
                .map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
        } else {
            fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
        }
    }

    // If Grove Root is gone (or never existed), remove the now-empty ~/.grove.
    // If it's still there this will fail, which is exactly what we want.
    if !preserve {
        match fs::remove_dir(grove_dir) {
            Ok(()) => println!("{} Removed ~/.grove", em("✅", "OK")),
            #[cfg(target_os = "windows")]
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                println!(
                    "{}  Could not remove {} automatically (the uninstaller is still running).",
                    em("⚠️", "!!"), grove_dir.display()
                );
                println!("   Please delete that folder manually once this window closes.");
            }
            Err(e) => return Err(format!("Failed to remove {}: {}", grove_dir.display(), e).into()),
        }
    } else {
        println!("{} Removed Grove program files", em("✅", "OK"));
        println!();
        println!(
            "{}  Your Grove Root has been left untouched at:",
            em("📦", "**")
        );
        println!("   {}", grove_root.display());
        println!("   It contains your personal Grove data. Delete it manually whenever");
        println!("   you're ready — Grove will never touch it again.");
    }

    Ok(())
}

// ============================================================
//  PATH removal
// ============================================================

#[cfg(target_os = "windows")]
fn remove_from_user_path(
    _home_dir: &Path,
    bin_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use winreg::enums::*;
    use winreg::RegKey;

    let bin_dir_str = bin_dir
        .to_str()
        .ok_or("Installation path contains non-UTF-8 characters")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let env_key = hkcu
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|e| format!("Failed to open Registry Environment key: {}", e))?;

    let current_path: String = env_key.get_value("Path").unwrap_or_default();

    let filtered: Vec<&str> = current_path
        .split(';')
        .filter(|entry| !entry.to_lowercase().eq(&bin_dir_str.to_lowercase()))
        .collect();

    let updated_path = filtered.join(";");

    if updated_path == current_path {
        println!("{}  Grove was not found in your PATH — nothing to remove", em("ℹ️", "--"));
        poke_windows_to_refresh();
        return Ok(());
    }

    env_key
        .set_value("Path", &updated_path)
        .map_err(|e| format!("Failed to update PATH in registry: {}", e))?;

    println!("{} Removed Grove from your PATH", em("✅", "OK"));
    poke_windows_to_refresh();
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn remove_from_user_path(
    home_dir: &Path,
    bin_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let bin_dir_str = bin_dir
        .to_str()
        .ok_or("Installation path contains non-UTF-8 characters")?;

    let Some(config_path) = shell_config_path(home_dir) else {
        eprintln!("{}  Fish shell detected. Please remove Grove from your fish config manually:", em("⚠️", "!!"));
        eprintln!("   fish_add_path --erase --universal {}", bin_dir_str);
        return Ok(());
    };

    if !config_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read {}: {}", config_path.display(), e))?;

    let had_entry = content.lines().any(|l| l.contains(bin_dir_str));

    if !had_entry {
        println!("{}  Grove PATH entry not found in {} — nothing to remove", em("ℹ️", "--"), config_path.display());
        return Ok(());
    }

    let updated = content
        .lines()
        .filter(|l| !l.contains(bin_dir_str))
        .collect::<Vec<_>>()
        .join("\n")
        + if content.ends_with('\n') { "\n" } else { "" };

    fs::write(&config_path, updated)
        .map_err(|e| format!("Failed to write {}: {}", config_path.display(), e))?;

    println!("{} Removed Grove from your PATH in {}", em("✅", "OK"), config_path.display());
    Ok(())
}

// ============================================================
//  Platform-specific launcher removal
// ============================================================

#[cfg(target_os = "linux")]
fn remove_linux_desktop_entry(home_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let desktop = home_dir.join(".local/share/applications/grove.desktop");
    let icon = home_dir.join(".local/share/icons/hicolor/256x256/apps/grove.png");
    remove_file_if_exists(&desktop, ".desktop entry")?;
    remove_file_if_exists(&icon, "application icon")?;

    let apps_dir = home_dir.join(".local/share/applications");
    let icon_dir = home_dir.join(".local/share/icons/hicolor");
    let _ = std::process::Command::new("update-desktop-database").arg(&apps_dir).status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .arg("--force").arg("--quiet").arg(&icon_dir).status();

    Ok(())
}

#[cfg(target_os = "macos")]
fn remove_mac_app_bundle(home_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let app = home_dir.join("Applications/Grove.app");
    if app.exists() {
        fs::remove_dir_all(&app)
            .map_err(|e| format!("Failed to remove Grove.app: {}", e))?;
        println!("{} Removed Grove.app from ~/Applications", em("✅", "OK"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn remove_windows_start_menu_shortcut() -> Result<(), Box<dyn std::error::Error>> {
    let shortcut = dirs::data_dir()
        .ok_or("Could not locate APPDATA directory")?
        .join("Microsoft\\Windows\\Start Menu\\Programs\\Grove.lnk");

    remove_file_if_exists(&shortcut, "Start Menu shortcut")?;
    Ok(())
}

// ============================================================
//  Windows self-relocation
// ============================================================

/// Returns true if the running exe lives somewhere inside `grove_dir`.
#[cfg(target_os = "windows")]
fn running_inside_grove_dir(grove_dir: &Path) -> bool {
    let Ok(exe)         = std::env::current_exe()   else { return false };
    let Ok(canon_exe)   = exe.canonicalize()         else { return false };
    let Ok(canon_grove) = grove_dir.canonicalize()   else { return false };
    canon_exe.starts_with(canon_grove)
}

/// Copies the uninstaller to %TEMP% and relaunches it with `--relocated`,
/// then exits. The copy does all the real work and cleans itself up.
#[cfg(target_os = "windows")]
fn relocate_and_relaunch() -> Result<(), Box<dyn std::error::Error>> {
    let exe     = std::env::current_exe()?;
    let tmp_exe = std::env::temp_dir().join("grove-uninstaller-tmp.exe");

    fs::copy(&exe, &tmp_exe)
        .map_err(|e| format!("Failed to copy uninstaller to temp dir: {}", e))?;

    println!("{} Relaunching from a temporary location to complete uninstall...", em("🔄", ".."));

    // Inherit the parent console so the user sees a continuous window.
    std::process::Command::new(&tmp_exe)
        .arg("--relocated")
        .spawn()
        .map_err(|e| format!("Failed to relaunch uninstaller: {}", e))?;

    std::process::exit(0);
}

/// Spawns a detached `cmd` that waits ~2 s then deletes this exe.
/// Call as the very last thing before exiting in the relocated copy.
#[cfg(target_os = "windows")]
fn schedule_self_deletion() {
    let Ok(exe) = std::env::current_exe() else { return };
    let exe_str = exe.to_string_lossy();

    // `ping` is used purely as a cross-platform sleep; the 3 pings give
    // the process ~2 seconds to fully exit before the delete fires.
    let _ = std::process::Command::new("cmd")
        .args([
            "/c",
            &format!("ping 127.0.0.1 -n 3 >nul & del /f /q \"{}\"", exe_str),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

// ============================================================
//  Windows environment broadcast — no windows-sys crate needed
// ============================================================

#[cfg(target_os = "windows")]
fn poke_windows_to_refresh() {
    use std::ptr;

    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;

    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    println!("{} Telling Windows to refresh its environment variables...", em("🔄", ".."));
    let env_str: Vec<u16> = "Environment\0".encode_utf16().collect();

    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            env_str.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            ptr::null_mut(),
        );
    }
}