#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone)]
struct Game {
    app_id: String,
    name: String,
    install_dir: String,
    library_path: String,
    size_on_disk: u64,
    cover_path: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Settings {
    // Paths the user added that Steam didn't auto-detect
    extra_paths: Vec<String>,
    // Auto-detected paths the user explicitly removed
    excluded_paths: Vec<String>,
}

fn vdf_get(content: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{}\"", key);
    let mut results = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(&needle) {
            continue;
        }
        let rest = trimmed[needle.len()..].trim();
        if rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"') {
            results.push(rest[1..rest.len() - 1].to_string());
        }
    }
    results
}

fn vdf_get_first(content: &str, key: &str) -> Option<String> {
    vdf_get(content, key).into_iter().next()
}

fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config/linux-mod-manager/settings.json")
}

#[tauri::command]
fn discover_steam_libraries() -> Vec<String> {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return vec![],
    };

    let candidate_roots = [
        format!("{}/.steam/steam", home),
        format!("{}/.steam/root", home),
        format!("{}/.local/share/Steam", home),
        format!("{}/.var/app/com.valvesoftware.Steam/data/Steam", home),
        format!("{}/snap/steam/common/.local/share/Steam", home),
    ];

    let mut seen_roots: HashSet<PathBuf> = HashSet::new();
    let mut steam_roots: Vec<PathBuf> = Vec::new();
    for candidate in &candidate_roots {
        let p = Path::new(candidate);
        if !p.exists() {
            continue;
        }
        let real = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        if seen_roots.insert(real.clone()) {
            steam_roots.push(real);
        }
    }

    let mut seen_libs: HashSet<PathBuf> = HashSet::new();
    let mut libraries: Vec<String> = Vec::new();
    for root in steam_roots {
        let vdf_path = root.join("steamapps/libraryfolders.vdf");
        let content = match fs::read_to_string(&vdf_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for path_str in vdf_get(&content, "path") {
            let lib = PathBuf::from(&path_str);
            if !lib.join("steamapps").exists() {
                continue;
            }
            let real = fs::canonicalize(&lib).unwrap_or(lib);
            if seen_libs.insert(real.clone()) {
                libraries.push(real.to_string_lossy().to_string());
            }
        }
    }

    libraries
}

// Returns true if the entry is an actual game rather than a Steam tool/runtime.
fn is_real_game(name: &str) -> bool {
    const EXCLUDED: &[&str] = &[
        "Proton ",
        "Steam Linux Runtime",
        "Steamworks Common",
        "SteamVR",
        "Steam OST",
        "Proton Experimental",
        "Proton Hotfix",
        "Proton BattlEye Runtime",
        "Proton EasyAntiCheat Runtime",
    ];
    !EXCLUDED.iter().any(|prefix| name.starts_with(prefix))
}

// Steam uses two cache layouts:
//   Old flat:  {appid}/library_600x900.jpg
//   New hashed: {appid}/{hash}/library_600x900.jpg  (or library_capsule.jpg)
// We check both, preferring the portrait cover, falling back to the landscape header.
fn cover_art_path(app_id: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let cache = PathBuf::from(&home).join(".local/share/Steam/appcache/librarycache");
    let app_dir = cache.join(app_id);
    if !app_dir.exists() {
        return None;
    }

    // 1. Old flat layout
    for name in ["library_600x900.jpg", "header.jpg"] {
        let p = app_dir.join(name);
        if p.exists() {
            return Some(fs::canonicalize(&p).unwrap_or(p).to_string_lossy().into_owned());
        }
    }

    // 2. New hashed-subdir layout — collect subdirs then search by priority
    let subdirs: Vec<PathBuf> = fs::read_dir(&app_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();

    for name in ["library_600x900.jpg", "library_capsule.jpg", "library_header.jpg"] {
        for subdir in &subdirs {
            let p = subdir.join(name);
            if p.exists() {
                return Some(fs::canonicalize(&p).unwrap_or(p).to_string_lossy().into_owned());
            }
        }
    }

    None
}

#[tauri::command]
fn scan_games(paths: Vec<String>) -> Vec<Game> {
    let mut games: Vec<Game> = Vec::new();

    for lib_path in &paths {
        let steamapps = Path::new(lib_path).join("steamapps");
        let entries = match fs::read_dir(&steamapps) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            if !fname.starts_with("appmanifest_") || !fname.ends_with(".acf") {
                continue;
            }

            let content = match fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if vdf_get_first(&content, "StateFlags").as_deref() != Some("4") {
                continue;
            }

            let name = match vdf_get_first(&content, "name") {
                Some(n) if !n.is_empty() => n,
                _ => continue,
            };

            if !is_real_game(&name) {
                continue;
            }

            let app_id = vdf_get_first(&content, "appid").unwrap_or_default();
            let cover_path = cover_art_path(&app_id);

            games.push(Game {
                app_id,
                name,
                install_dir: vdf_get_first(&content, "installdir").unwrap_or_default(),
                library_path: lib_path.clone(),
                size_on_disk: vdf_get_first(&content, "SizeOnDisk")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                cover_path,
            });
        }
    }

    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    games
}

#[tauri::command]
fn read_cover_image(path: String) -> Option<String> {
    let data = fs::read(&path).ok()?;
    let mime = if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "image/png"
    };
    Some(format!("data:{};base64,{}", mime, STANDARD.encode(&data)))
}

#[tauri::command]
fn load_settings() -> Settings {
    let content = match fs::read_to_string(settings_path()) {
        Ok(c) => c,
        Err(_) => return Settings::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

#[tauri::command]
fn save_settings(settings: Settings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            if let Some(icon) = app.default_window_icon() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_icon(icon.clone());
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            discover_steam_libraries,
            scan_games,
            read_cover_image,
            load_settings,
            save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running application");
}
