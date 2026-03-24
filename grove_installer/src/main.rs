use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(not(target_os = "windows"))]
use std::io::Write;

// Embeds the correct binary at compile time depending on target platform.
#[cfg(target_os = "windows")]
const GROVE_BINARY: &[u8] = include_bytes!("../grove.exe");

#[cfg(not(target_os = "windows"))]
const GROVE_BINARY: &[u8] = include_bytes!("../grove");

#[cfg(target_os = "linux")]
const GROVE_ICON_PNG: &[u8] = include_bytes!("../grove.png");

#[cfg(target_os = "macos")]
const GROVE_ICON_ICNS: &[u8] = include_bytes!("../grove.icns");

// Embeds the uninstaller binary at compile time.
#[cfg(target_os = "windows")]
const GROVE_UNINSTALLER_BINARY: &[u8] = include_bytes!("../grove_uninstaller.exe");

#[cfg(not(target_os = "windows"))]
const GROVE_UNINSTALLER_BINARY: &[u8] = include_bytes!("../grove_uninstaller");

// Appends ".exe" on Windows, leaves the name bare everywhere else.
#[cfg(target_os = "windows")]
macro_rules! exe { ($n:literal) => { concat!($n, ".exe") } }
#[cfg(not(target_os = "windows"))]
macro_rules! exe { ($n:literal) => { $n } }

const GROVE_BIN_NAME: &str = exe!("grove");
const GROVE_UNINSTALLER_BIN_NAME: &str = exe!("grove_uninstaller");

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
        eprintln!("\n{} Installation failed: {}", em("❌", "!!"), e);
        println!("\nPress Enter to exit...");
        let mut input = String::new();
        io::stdin().read_line(&mut input).ok();
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", em("=== 🌱 Grove Installer ===", "=== Grove Installer ==="));

    let home_dir = dirs::home_dir().ok_or("Could not find user home directory")?;
    let grove_dir = home_dir.join(".grove");
    let target_dir = grove_dir.join("bin");

    // Track whether .grove is new so we know whether to clean up on failure.
    let fresh_install = !grove_dir.exists();

    let result = install(&home_dir, &target_dir);

    if result.is_err() && fresh_install {
        let _ = fs::remove_dir_all(&grove_dir);
    }
    result?;

    println!("\n{}", em("🎉 Grove is installed! 🎉", "Grove is installed!"));

    #[cfg(target_os = "windows")]
    println!("You can now open any NEW command prompt or terminal and simply type: grove");

    #[cfg(not(target_os = "windows"))]
    {
        let config_hint = shell_config_path(&home_dir)
            .map(|p| format!("source {}", p.display()))
            .unwrap_or_else(|| "reload your fish config".to_string());
        println!("Open a new terminal (or run: {}) and simply type: grove", config_hint);
    }

    println!("\nPress Enter to exit...");
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();

    Ok(())
}

fn install(home_dir: &Path, target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // create_dir_all is already a no-op if the directory exists.
    fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create installation directory: {}", e))?;

    let target_bin = install_binary(target_dir, GROVE_BIN_NAME, GROVE_BINARY)?;
    println!("{} Copied Grove to {}", em("✅", "OK"), target_bin.display());

    let target_uninstaller = install_binary(target_dir, GROVE_UNINSTALLER_BIN_NAME, GROVE_UNINSTALLER_BINARY)?;
    println!("{} Copied Grove Uninstaller to {}", em("✅", "OK"), target_uninstaller.display());

    add_to_user_path(home_dir, target_dir)?;

    #[cfg(target_os = "linux")]
    install_linux_desktop_entry(home_dir, &target_bin)?;

    #[cfg(target_os = "macos")]
    install_mac_app_bundle(home_dir, &target_bin)?;

    #[cfg(target_os = "windows")]
    install_windows_start_menu_shortcut(&target_bin)?;

    verify_binary(&target_bin)?;
    Ok(())
}

// ============================================================
//  Binary installation
// ============================================================

/// Writes `data` to `dir/name` atomically and marks it executable. Returns the final path.
fn install_binary(
    dir: &Path,
    name: &str,
    data: &[u8],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = dir.join(name);
    write_binary_atomically(&path, data)?;
    set_executable_bit(&path)?;
    #[cfg(target_os = "macos")]
    remove_quarantine(&path);
    Ok(path)
}

fn write_binary_atomically(
    target_bin: &Path,
    data: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let tmp_bin = target_bin.with_extension("tmp");

    fs::write(&tmp_bin, data).map_err(|e| {
        #[cfg(target_os = "windows")]
        if e.kind() == io::ErrorKind::PermissionDenied {
            return "Grove appears to be running — please close it before reinstalling".to_string();
        }
        format!("Failed to write Grove binary to disk: {}", e)
    })?;

    fs::rename(&tmp_bin, target_bin)
        .map_err(|e| {
            let _ = fs::remove_file(&tmp_bin);
            format!("Failed to finalize Grove binary: {}", e)
        })?;

    Ok(())
}

fn verify_binary(bin_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new(bin_path)
        .arg("pinkponyclub")
        .output()
        .map_err(|e| format!("Installed binary failed to run — the installation may be corrupt: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "Installed binary exited with error code {:?} — the installation may be corrupt",
            output.status.code()
        ).into());
    }

    Ok(())
}

// ============================================================
//  Executable bit
// ============================================================

#[cfg(target_os = "windows")]
fn set_executable_bit(_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_executable_bit(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)
        .map_err(|e| format!("Failed to read binary metadata: {}", e))?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)
        .map_err(|e| format!("Failed to set executable permissions on binary: {}", e))?;
    Ok(())
}

// ============================================================
//  PATH
// ============================================================

#[cfg(target_os = "windows")]
fn add_to_user_path(
    _home_dir: &Path,
    new_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let env_key = hkcu
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .map_err(|e| format!("Failed to open Registry Environment key: {}", e))?;

    let current_path: String = env_key.get_value("Path").unwrap_or_default();
    let new_path_str = new_path
        .to_str()
        .ok_or("Installation path contains non-UTF-8 characters")?;

    let already_present = current_path
        .split(';')
        .any(|entry| entry.to_lowercase() == new_path_str.to_lowercase());

    if already_present {
        println!("{} Grove is already in your PATH", em("✅", "OK"));
        poke_windows_to_refresh();
        return Ok(());
    }

    let separator = if current_path.ends_with(';') || current_path.is_empty() {
        ""
    } else {
        ";"
    };
    let updated_path = format!("{}{}{}", current_path, separator, new_path_str);

    // The registry PATH value has a hard ceiling of 32,767 UTF-16 code units.
    // Guard against pushing over the edge (8,000-char margin for safety).
    if updated_path.len() > 32_000 {
        return Err("Cannot add Grove to PATH: the registry PATH value is too long. Please add it manually.".into());
    }

    env_key
        .set_value("Path", &updated_path)
        .map_err(|e| format!("Failed to update PATH in registry: {}", e))?;

    println!("{} Added Grove to your User PATH", em("✅", "OK"));
    poke_windows_to_refresh();
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn add_to_user_path(
    home_dir: &Path,
    new_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let new_path_str = new_path
        .to_str()
        .ok_or("Installation path contains non-UTF-8 characters")?;
    let export_line = format!("\nexport PATH=\"{}:$PATH\"\n", new_path_str);

    let Some(config_path) = shell_config_path(home_dir) else {
        eprintln!("{}  Fish shell detected. Please add the following to your fish config manually:", em("⚠️", "!!"));
        eprintln!("   fish_add_path --universal {}", new_path_str);
        return Ok(());
    };

    if config_path.exists() {
        let existing = fs::read_to_string(&config_path).unwrap_or_default();
        if existing.split(':').any(|e| e.trim_matches('"') == new_path_str) {
            println!("{} Grove is already in your PATH (found in {})", em("✅", "OK"), config_path.display());
            return Ok(());
        }
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config_path)
        .map_err(|e| format!("Failed to open {} for writing: {}", config_path.display(), e))?;

    file.write_all(export_line.as_bytes())
        .map_err(|e| format!("Failed to write to {}: {}", config_path.display(), e))?;

    println!("{} Added Grove to your PATH in {}", em("✅", "OK"), config_path.display());
    Ok(())
}

// ============================================================
//  macOS quarantine removal
// ============================================================

#[cfg(target_os = "macos")]
fn remove_quarantine(path: &Path) {
    // Best-effort: silently ignore if the attribute isn't present.
    let _ = std::process::Command::new("xattr")
        .args(["-d", "com.apple.quarantine"])
        .arg(path)
        .status();
}

// ============================================================
//  Platform-specific launcher integration
// ============================================================

#[cfg(target_os = "linux")]
fn install_linux_desktop_entry(
    home_dir: &Path,
    bin_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let icon_dir = home_dir.join(".local/share/icons/hicolor/256x256/apps");
    fs::create_dir_all(&icon_dir)
        .map_err(|e| format!("Failed to create icon directory: {}", e))?;
    fs::write(icon_dir.join("grove.png"), GROVE_ICON_PNG)
        .map_err(|e| format!("Failed to write icon: {}", e))?;

    let apps_dir = home_dir.join(".local/share/applications");
    fs::create_dir_all(&apps_dir)
        .map_err(|e| format!("Failed to create applications directory: {}", e))?;

    let desktop = format!(
        "[Desktop Entry]\nName=Grove\nExec={}\nIcon=grove\nType=Application\nCategories=Utility;\nTerminal=true\n",
        bin_path.to_str().ok_or("Binary path contains non-UTF-8 characters")?
    );
    fs::write(apps_dir.join("grove.desktop"), desktop)
        .map_err(|e| format!("Failed to write .desktop file: {}", e))?;

    let icon_dir_hicolor = home_dir.join(".local/share/icons/hicolor");
    let _ = std::process::Command::new("update-desktop-database").arg(&apps_dir).status();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .arg("--force").arg("--quiet").arg(&icon_dir_hicolor).status();

    println!("{} Added Grove to your application launcher", em("✅", "OK"));
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_mac_app_bundle(
    home_dir: &Path,
    bin_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let app_root = home_dir.join("Applications/Grove.app/Contents");
    let macos_dir = app_root.join("MacOS");
    let resources_dir = app_root.join("Resources");

    fs::create_dir_all(&macos_dir)
        .map_err(|e| format!("Failed to create .app bundle: {}", e))?;
    fs::create_dir_all(&resources_dir)
        .map_err(|e| format!("Failed to create Resources directory: {}", e))?;

    fs::write(resources_dir.join("grove.icns"), GROVE_ICON_ICNS)
        .map_err(|e| format!("Failed to write icon: {}", e))?;

    let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>grove_launcher</string>
    <key>CFBundleIconFile</key>
    <string>grove</string>
    <key>CFBundleName</key>
    <string>Grove</string>
    <key>CFBundleIdentifier</key>
    <string>com.grove.app</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
</dict>
</plist>"#;
    fs::write(app_root.join("Info.plist"), plist)
        .map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    let bin_path_str = bin_path.to_str().ok_or("Binary path contains non-UTF-8 characters")?;
    let escaped_bin_path = bin_path_str.replace('"', "\\\"");
    let launcher = format!(
        "#!/bin/bash\nosascript <<'APPLESCRIPT'\ntell application \"Terminal\" to do script \"{}\"\nAPPLESCRIPT\n",
        escaped_bin_path
    );
    let launcher_path = macos_dir.join("grove_launcher");
    fs::write(&launcher_path, launcher)
        .map_err(|e| format!("Failed to write launcher script: {}", e))?;

    set_executable_bit(&launcher_path)?;
    remove_quarantine(&launcher_path);

    println!("{} Created Grove.app in ~/Applications", em("✅", "OK"));
    Ok(())
}

#[cfg(target_os = "windows")]
fn install_windows_start_menu_shortcut(
    bin_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use mslnk::ShellLink;

    let start_menu = dirs::data_dir()
        .ok_or("Could not locate APPDATA directory")?
        .join("Microsoft\\Windows\\Start Menu\\Programs\\Grove.lnk");

    let sl = ShellLink::new(bin_path)
        .map_err(|e| format!("Failed to create shortcut: {}", e))?;
    sl.create_lnk(&start_menu)
        .map_err(|e| format!("Failed to write Start Menu shortcut: {}", e))?;

    println!("{} Added Grove to Start Menu", em("✅", "OK"));
    Ok(())
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