#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

mod installed;
mod modding;
mod nexusmods;

// ── Data structures ──────────────────────────────────────────────────

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
    // App IDs the user has pinned to the sidebar
    #[serde(default)]
    pinned_games: Vec<String>,
    // NexusMods personal API key (optional)
    #[serde(default)]
    nexus_api_key: String,
    // NexusMods username — saved on login so the UI can restore without re-login
    #[serde(default)]
    nexus_username: String,
}

// Single mod entry returned to the frontend — represents either a managed
// mod (installed via our app, grouped under one name) or an unmanaged mod
// (individually detected file/directory not in the manifest).
#[derive(Serialize, Deserialize, Clone)]
struct DisplayMod {
    id: String,
    name: String,
    version: String,
    enabled: bool,
    managed: bool,
    components: Vec<String>,
}

// ── VDF helpers ──────────────────────────────────────────────────────

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

// ── Filesystem helpers ───────────────────────────────────────────────

fn settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config/linux-mod-manager/settings.json")
}

// ── Game filtering ───────────────────────────────────────────────────

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

// ── Cover art ────────────────────────────────────────────────────────

// Steam uses two cache layouts:
//   Old flat:   {appid}/library_600x900.jpg
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

// ── Tauri commands — library / game scanning ─────────────────────────

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

// ── Tauri commands — mod management ──────────────────────────────────

#[tauri::command]
fn list_mods(app_id: String, install_dir: String) -> Vec<modding::Mod> {
    modding::list_mods(&app_id, &install_dir)
}

#[tauri::command]
fn toggle_mod(path: String, mod_type: modding::ModType, enabled: bool) -> Result<(), String> {
    modding::toggle_mod(&path, &mod_type, enabled)
}

// ── Tauri commands — mod management ──────────────────────────────────

fn delete_paths(paths: &[String]) -> Result<(), String> {
    for path_str in paths {
        let path = std::path::Path::new(path_str);
        if !path.exists() {
            continue;
        }
        if path.is_dir() {
            fs::remove_dir_all(path)
                .map_err(|e| format!("Cannot remove {}: {e}", path_str))?;
        } else {
            fs::remove_file(path)
                .map_err(|e| format!("Cannot remove {}: {e}", path_str))?;
        }
    }
    Ok(())
}

fn toggle_paths(components: &[String], enabled: bool) -> Result<(), String> {
    for path_str in components {
        let path = std::path::Path::new(path_str);
        if enabled {
            // Enable: remove .disabled suffix if present
            let disabled = format!("{}.disabled", path_str);
            let disabled_path = std::path::Path::new(&disabled);
            if disabled_path.exists() {
                fs::rename(disabled_path, path).map_err(|e| e.to_string())?;
            }
        } else {
            // Disable: add .disabled suffix if not already disabled
            if path.exists() {
                let disabled = format!("{}.disabled", path_str);
                fs::rename(path, &disabled).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

// Legacy per-file commands (still used by unmanaged mods in the frontend)
#[tauri::command]
fn uninstall_mod(paths: Vec<String>) -> Result<(), String> {
    delete_paths(&paths)
}

// ── Tauri commands — display mods (manifest-aware) ───────────────────

#[tauri::command]
fn list_display_mods(app_id: String, install_dir: String) -> Vec<DisplayMod> {
    let manifest = installed::load_manifest(&app_id);

    // Build a set of all component paths owned by managed mods
    let mut managed_paths: HashSet<String> = HashSet::new();
    for m in &manifest.mods {
        for p in &m.components {
            managed_paths.insert(p.clone());
        }
    }

    let mut result: Vec<DisplayMod> = Vec::new();

    // Managed mods from manifest — one entry per manifest record
    for m in &manifest.mods {
        // Enabled = first component exists without .disabled suffix
        let enabled = m.components.first().map_or(false, |p| Path::new(p).exists());
        result.push(DisplayMod {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            enabled,
            managed: true,
            components: m.components.clone(),
        });
    }

    // Unmanaged mods — filesystem scan, excluding anything already in the manifest,
    // then grouped by display name so multi-file mods still appear as one row.
    let scanned = modding::list_mods(&app_id, &install_dir);
    let mut unmanaged: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut unmanaged_enabled: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();

    for m in scanned {
        let canonical = m.path.strip_suffix(".disabled").unwrap_or(&m.path).to_string();
        if managed_paths.contains(&m.path) || managed_paths.contains(&canonical) {
            continue;
        }
        let paths = unmanaged.entry(m.name.clone()).or_default();
        paths.push(m.path.clone());
        // Mod is enabled only if ALL components are enabled
        let entry = unmanaged_enabled.entry(m.name.clone()).or_insert(true);
        *entry = *entry && m.enabled;
    }

    for (name, paths) in unmanaged {
        let enabled = unmanaged_enabled.get(&name).copied().unwrap_or(true);
        // Use the first path as the stable ID for unmanaged mods
        let id = paths.first().cloned().unwrap_or_else(|| name.clone());
        result.push(DisplayMod {
            id,
            name,
            version: String::new(),
            enabled,
            managed: false,
            components: paths,
        });
    }

    result.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    result
}

#[tauri::command]
fn toggle_display_mod(components: Vec<String>, enabled: bool) -> Result<(), String> {
    toggle_paths(&components, enabled)
}

#[tauri::command]
fn uninstall_display_mod(
    app_id: String,
    mod_id: String,
    components: Vec<String>,
) -> Result<(), String> {
    delete_paths(&components)?;
    // Only remove from manifest if it was a managed mod
    installed::remove_mod(&app_id, &mod_id).ok();
    Ok(())
}

// ── Tauri commands — NexusMods ────────────────────────────────────────

#[tauri::command]
async fn nexus_lookup(
    api_key: String,
    input: String,
) -> Result<(nexusmods::NexusModInfo, Vec<nexusmods::NexusModFile>), String> {
    nexusmods::lookup(&api_key, &input).await
}

#[tauri::command]
async fn nexus_collection_lookup(
    api_key: String,
    input: String,
) -> Result<(nexusmods::NexusCollectionInfo, Vec<nexusmods::NexusCollectionMod>), String> {
    nexusmods::lookup_collection(&api_key, &input).await
}

#[tauri::command]
async fn nexus_install(
    api_key: String,
    input: String,
    file_id: u64,
    install_dir: String,
    app_id: String,
    mod_name: String,
    mod_version: String,
    nexus_mod_id: u64,
    app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
    let extracted = nexusmods::install(&api_key, &input, file_id, &install_dir, &app_handle).await?;

    // Derive scanner-visible top-level component paths
    let components = modding::derive_components(&app_id, &install_dir, &extracted);

    // Record in manifest so the mod shows as one entry
    let entry = installed::InstalledMod {
        id: format!("{}_{}", nexus_mod_id, file_id),
        name: mod_name,
        version: mod_version,
        nexus_mod_id: Some(nexus_mod_id),
        nexus_file_id: Some(file_id),
        components,
    };
    installed::add_mod(&app_id, entry).ok(); // non-fatal if manifest write fails

    Ok(extracted)
}

// ── Tauri commands — NexusMods WebView auth ──────────────────────────

#[derive(serde::Deserialize, Clone)]
struct AuthModInfo {
    mod_id: u64,
    file_id: u64,
    game_domain: String,
    name: String,
    version: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone)]
struct AuthDownloadResult {
    mod_id: u64,
    file_id: u64,
    name: String,
    version: String,
    game_domain: String,
    url: Option<String>,
    error: Option<String>,
}

#[tauri::command]
async fn open_nexusmods_auth(
    app: tauri::AppHandle,
    api_key: String,
    mods: Vec<AuthModInfo>,
) -> Result<(), String> {
    println!("[lmm-auth] open_nexusmods_auth called with {} mods", mods.len());
    let domain = mods.first().map(|m| m.game_domain.as_str()).unwrap_or("cyberpunk2077").to_string();
    println!("[lmm-auth] fetching game_id for domain={domain}");
    let game_id = nexusmods::fetch_game_id(&api_key, &domain).await
        .map_err(|e| { println!("[lmm-auth] fetch_game_id FAILED: {e}"); e })?;
    println!("[lmm-auth] game_id={game_id}");

    let mods_json = serde_json::to_string(
        &mods.iter().map(|m| serde_json::json!({
            "mod_id": m.mod_id,
            "file_id": m.file_id,
            "game_id": game_id,
            "name": m.name,
            "version": m.version,
            "game_domain": m.game_domain,
        })).collect::<Vec<_>>()
    ).map_err(|e| e.to_string())?;

    // Each mod page is loaded in sequence. On each page the init_script:
    //   1. Reads accumulated results from sessionStorage
    //   2. POSTs GenerateDownloadUrl with the current page as Referer
    //   3. Stores result back to sessionStorage
    //   4. Navigates to the next mod's page, or writes the final hash when done
    // sessionStorage persists across same-origin navigations so we can accumulate results.
    // The window is hidden — no popup for the user.
    let init_script = format!(r#"
(async function() {{
    function b64(str) {{
        var bytes = new TextEncoder().encode(str);
        var b = '';
        for (var i = 0; i < bytes.length; i++) b += String.fromCharCode(bytes[i]);
        return btoa(b).replace(/\+/g,'-').replace(/\//g,'_').replace(/=/g,'');
    }}
    function dbg(msg) {{ console.log('[lmm-auth]', msg); }}

    if (window.location.hostname !== 'www.nexusmods.com') {{
        dbg('not www — skipping');
        return;
    }}

    const mods = {mods_json};
    var pathname = window.location.pathname;
    dbg('page: ' + pathname + '  mods total: ' + mods.length);

    // Find which mod index this page corresponds to
    var modMatch = pathname.match(/\/[^\/]+\/mods\/(\d+)/);
    if (!modMatch) {{
        dbg('not a mod page, nothing to do');
        return;
    }}
    var pageModId = parseInt(modMatch[1]);
    var idx = mods.findIndex(function(m) {{ return m.mod_id === pageModId; }});
    if (idx === -1) {{
        dbg('mod_id ' + pageModId + ' not in our list — skipping');
        return;
    }}

    await new Promise(function(r) {{ setTimeout(r, 600); }});

    // Load accumulated results from previous pages
    var stored = [];
    try {{ stored = JSON.parse(sessionStorage.getItem('lmm_auth_results') || '[]'); }} catch(_) {{}}
    dbg('idx=' + idx + ' accumulated so far: ' + stored.length);

    var mod = mods[idx];
    try {{
        const body = new URLSearchParams();
        body.append('game_id', String(mod.game_id));
        body.append('id', String(mod.file_id));
        body.append('nmm', '1');
        dbg('POST GenerateDownloadUrl  game_id=' + mod.game_id + ' file_id=' + mod.file_id);
        const resp = await fetch('/Core/Libs/Common/Managers/Downloads?GenerateDownloadUrl',
            {{ method: 'POST', headers: {{'X-Requested-With': 'XMLHttpRequest'}}, body }});
        const text = await resp.text();
        dbg('HTTP ' + resp.status + '  response: ' + text.substring(0, 300));
        var parsed;
        try {{ parsed = JSON.parse(text); }} catch(_) {{ parsed = null; }}
        // Response is an array of CDN servers — take the first URI
        var entry = Array.isArray(parsed) ? parsed[0] : parsed;
        var url = (entry && (entry.URI || entry.url || entry.download_url)) || null;
        dbg('resolved url: ' + (url ? url.substring(0, 80) + '...' : 'null'));
        stored.push({{ mod_id: mod.mod_id, file_id: mod.file_id, name: mod.name,
            version: mod.version, game_domain: mod.game_domain, url: url,
            error: url ? null : 'HTTP ' + resp.status + ': ' + text }});
    }} catch(e) {{
        dbg('fetch error: ' + e);
        stored.push({{ mod_id: mod.mod_id, file_id: mod.file_id, name: mod.name,
            version: mod.version, game_domain: mod.game_domain, url: null, error: String(e) }});
    }}

    sessionStorage.setItem('lmm_auth_results', JSON.stringify(stored));

    var nextIdx = idx + 1;
    if (nextIdx < mods.length) {{
        var next = mods[nextIdx];
        var nextUrl = 'https://www.nexusmods.com/' + next.game_domain + '/mods/' + next.mod_id;
        dbg('navigating to next mod: ' + nextUrl);
        window.location.href = nextUrl;
    }} else {{
        dbg('all ' + stored.length + ' mods processed — writing hash');
        sessionStorage.removeItem('lmm_auth_results');
        window.location.hash = 'lmm-results:' + b64(JSON.stringify(stored));
    }}
}})();
    "#);

    let first = mods.first().ok_or("no mods provided")?;
    let start_url = format!("https://www.nexusmods.com/{}/mods/{}", first.game_domain, first.mod_id);
    println!("[lmm-auth] starting on mod page: {start_url}");
    let login_url: url::Url = start_url.parse().map_err(|e: url::ParseError| e.to_string())?;

    tauri::webview::WebviewWindowBuilder::new(
        &app,
        "nexusmods-auth",
        tauri::WebviewUrl::External(login_url),
    )
    .title("NexusMods auth")
    .inner_size(1.0, 1.0)
    .visible(false)
    .initialization_script(&init_script)
    .devtools(true)
    .build()
    .map_err(|e| { println!("[lmm-auth] window build FAILED: {e}"); e.to_string() })?;

    println!("[lmm-auth] window opened");
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_url = String::new();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let win = match app2.get_webview_window("nexusmods-auth") {
                Some(w) => w,
                None => { println!("[lmm-auth] window closed"); break; }
            };
            if let Ok(url) = win.url() {
                let url_str = url.to_string();
                if url_str != last_url {
                    println!("[lmm-auth] url: {url_str}");
                    last_url = url_str.clone();
                }
                // Check for base64-encoded results in fragment
                if let Some(frag) = url.fragment() {
                    if let Some(b64) = frag.strip_prefix("lmm-results:") {
                        println!("[lmm-auth] results hash detected, decoding...");
                        // Restore URL-safe base64 to standard
                        let b64_std = b64.replace('-', "+").replace('_', "/");
                        let pad = (4 - b64_std.len() % 4) % 4;
                        let b64_padded = format!("{b64_std}{}", "=".repeat(pad));
                        use base64::{engine::general_purpose, Engine as _};
                        match general_purpose::STANDARD.decode(&b64_padded) {
                            Ok(bytes) => match String::from_utf8(bytes) {
                                Ok(json) => {
                                    println!("[lmm-auth] decoded JSON ({} bytes)", json.len());
                                    match serde_json::from_str::<Vec<AuthDownloadResult>>(&json) {
                                        Ok(results) => {
                                            let ok = results.iter().filter(|r| r.url.is_some()).count();
                                            println!("[lmm-auth] parsed {} results, {} with URL", results.len(), ok);
                                            let _ = win.close();
                                            let _ = app2.emit("collection-auth-complete", &results);
                                            break;
                                        }
                                        Err(e) => println!("[lmm-auth] JSON parse error: {e}"),
                                    }
                                }
                                Err(e) => println!("[lmm-auth] UTF-8 decode error: {e}"),
                            },
                            Err(e) => println!("[lmm-auth] base64 decode error: {e}"),
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

// Opens a login window from Settings. Detects login via init_script + Rust polling,
// extracts username, optionally extracts API key, then emits nexusmods-login-complete.
#[tauri::command]
async fn open_nexusmods_login(app: tauri::AppHandle) -> Result<(), String> {
    // Init script tries __TAURI_INTERNALS__ (the actual Tauri 2 IPC for external pages).
    // Rust URL polling below is the authoritative fallback.
    let init_script = r#"
(async function() {
    async function log(msg) {
        console.log('[lmm-login]', msg);
        try { await window.__TAURI_INTERNALS__.invoke('lmm_debug_log', { message: '[login-js] ' + msg }); } catch(e) {
            console.warn('[lmm-login] lmm_debug_log invoke failed:', e);
        }
    }
    var hostname = window.location.hostname;
    var pathname = window.location.pathname;
    await log('page: ' + hostname + pathname);
    if (hostname.indexOf('nexusmods.com') === -1) { await log('not nexusmods'); return; }
    await new Promise(function(r) { setTimeout(r, 1500); });
    var bodyText = document.body ? (document.body.innerText || '') : '';
    await log('bodyLen=' + bodyText.length + ' host=' + hostname + ' path=' + pathname);
    // URL rule: any users.nexusmods.com page outside /auth/ = logged in
    if (hostname === 'users.nexusmods.com' && pathname.indexOf('/auth/') !== 0) {
        var m = bodyText.match(/Welcome\s+back\s+([^\s\n,!.]+)/i);
        var username = m ? m[1].replace(/[^a-zA-Z0-9_\-]/g, '') : null;
        await log('URL rule hit, username=' + username);
        try {
            await window.__TAURI_INTERNALS__.invoke('nexusmods_login_complete', { apiKey: null, username: username || null });
            await log('invoke ok');
        } catch(e) { await log('invoke FAILED: ' + String(e)); }
        return;
    }
    var m = bodyText.match(/Welcome\s+back\s+([^\s\n,!.]+)/i);
    var username = m ? m[1].replace(/[^a-zA-Z0-9_\-]/g, '') : null;
    var signOut = document.querySelector('a[href*="sign_out"],a[href*="logout"],a[href*="sign-out"]');
    await log('DOM: username=' + username + ' signOut=' + !!signOut + ' preview=' + bodyText.substring(0, 80).replace(/\s+/g, ' '));
    if (username || signOut) {
        try {
            await window.__TAURI_INTERNALS__.invoke('nexusmods_login_complete', { apiKey: null, username: username || null });
            await log('invoke ok');
        } catch(e) { await log('invoke FAILED: ' + String(e)); }
    } else {
        await log('not logged in, waiting for user');
    }
})();
    "#;

    let login_url: url::Url = "https://users.nexusmods.com/auth/sign_in"
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;

    tauri::webview::WebviewWindowBuilder::new(
        &app,
        "nexusmods-login",
        tauri::WebviewUrl::External(login_url),
    )
    .title("NexusMods Login")
    .inner_size(900.0, 700.0)
    .initialization_script(init_script)
    .devtools(true)
    .build()
    .map_err(|e| e.to_string())?;

    // Rust-side URL polling: authoritative fallback if __TAURI_INTERNALS__ invoke doesn't fire.
    // Logs every URL change so we can see exactly what the WebView is doing in the terminal.
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_url = String::new();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let win = match app2.get_webview_window("nexusmods-login") {
                Some(w) => w,
                None => { println!("[lmm-login] window closed"); break; }
            };
            if let Ok(url) = win.url() {
                let url_str = url.to_string();
                if url_str != last_url {
                    println!("[lmm-login] url: {url_str}");
                    last_url = url_str;
                }
                let host = url.host_str().unwrap_or("");
                let path = url.path();
                if host == "users.nexusmods.com" && !path.starts_with("/auth/") {
                    let fragment = url.fragment().unwrap_or("");
                    if let Some(raw) = fragment.strip_prefix("lmm-user:") {
                        // Username extracted from hash on previous iteration
                        let username = if raw.is_empty() { None } else { Some(raw.to_string()) };
                        println!("[lmm-login] login complete, username={username:?}");
                        let _ = win.close();
                        let _ = app2.emit("nexusmods-login-complete", serde_json::json!({
                            "api_key": Option::<String>::None,
                            "username": username,
                        }));
                        break;
                    }
                    // First time on this URL: eval JS to write username into hash, read it next poll
                    println!("[lmm-login] login URL detected, extracting username via hash (waiting 1.5s for render)");
                    let _ = win.eval(r#"
                        setTimeout(function() {
                            var b = document.body ? document.body.innerText : '';
                            var m = b.match(/Welcome\s+back\s+([^\s\n,!.]+)/i);
                            var u = m ? m[1].replace(/[^a-zA-Z0-9_\-]/g, '') : '';
                            console.log('[lmm-login] username from DOM:', u, 'bodyLen:', b.length);
                            window.location.hash = 'lmm-user:' + u;
                        }, 1500);
                    "#);
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn lmm_debug_log(message: String) {
    println!("[lmm] {message}");
}

// Called by the login window's JS. Closes the window and emits to the frontend.
#[tauri::command]
fn nexusmods_login_complete(
    app: tauri::AppHandle,
    api_key: Option<String>,
    username: Option<String>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("nexusmods-login") {
        let _ = window.close();
        app.emit("nexusmods-login-complete", serde_json::json!({
            "api_key": api_key,
            "username": username,
        })).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Called by the auth window's JS. Closes the window and emits download URLs.
// Deduplicated: only acts if the window is still open (first call wins).
#[tauri::command]
fn nexusmods_download_urls_ready(
    app: tauri::AppHandle,
    results: Vec<AuthDownloadResult>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("nexusmods-auth") {
        let _ = window.close();
        app.emit("collection-auth-complete", &results)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn nexus_install_from_url(
    url: String,
    file_id: u64,
    install_dir: String,
    app_id: String,
    mod_name: String,
    mod_version: String,
    nexus_mod_id: u64,
    app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
    let extracted = nexusmods::install_from_url(&url, file_id, &install_dir, &app_handle).await?;
    let components = modding::derive_components(&app_id, &install_dir, &extracted);
    let entry = installed::InstalledMod {
        id: format!("{}_{}", nexus_mod_id, file_id),
        name: mod_name,
        version: mod_version,
        nexus_mod_id: Some(nexus_mod_id),
        nexus_file_id: Some(file_id),
        components,
    };
    installed::add_mod(&app_id, entry).ok();
    Ok(extracted)
}

// ── Tauri commands — utilities ───────────────────────────────────────

#[tauri::command]
fn open_in_browser(url: String) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("xdg-open failed: {e}"))?;
    Ok(())
}

// ── Tauri commands — settings ─────────────────────────────────────────

#[tauri::command]
fn load_settings() -> Settings {
    let content = match fs::read_to_string(settings_path()) {
        Ok(c) => c,
        Err(_) => return Settings::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

#[tauri::command]
fn save_nexus_username(username: String) -> Result<(), String> {
    let mut s = load_settings();
    s.nexus_username = username;
    save_settings(s)
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

// Stores an NXM URL received before the webview was ready
struct PendingNxmUrl(Mutex<Option<String>>);

// ── Tauri commands — deep link ────────────────────────────────────────

// Frontend calls this after init to pick up any NXM link that arrived
// while the webview was still loading (cold-start via NXM click).
#[tauri::command]
fn get_pending_nxm_url(state: tauri::State<PendingNxmUrl>) -> Option<String> {
    state.0.lock().unwrap().take()
}

// Frontend calls this when it processes a live "nxm-link" event so the
// pending slot is cleared before get_pending_nxm_url runs.
#[tauri::command]
fn clear_pending_nxm_url(state: tauri::State<PendingNxmUrl>) {
    *state.0.lock().unwrap() = None;
}

// ── Entry point ───────────────────────────────────────────────────────

fn raise_and_deliver_nxm(app: &tauri::AppHandle, url: &str) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    if let Some(state) = app.try_state::<PendingNxmUrl>() {
        *state.0.lock().unwrap() = Some(url.to_string());
    }
    let _ = app.emit("nxm-link", url);
}

fn main() {
    tauri::Builder::default()
        // Single-instance guard: when a second process is launched (e.g. by an
        // NXM link click while the app is already running), this kills it and
        // forwards its argv to the existing instance instead.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Bring the existing window to front
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            // If the second process was launched with an NXM URL, handle it
            for arg in &argv {
                if arg.starts_with("nxm://") {
                    raise_and_deliver_nxm(app, arg);
                    break;
                }
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .manage(PendingNxmUrl(Mutex::new(None)))
        .setup(|app| {
            if let Some(icon) = app.default_window_icon() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_icon(icon.clone());
                }
            }

            // Register the nxm:// scheme handler in dev mode.
            // In production this is handled by the installed .desktop file.
            #[cfg(debug_assertions)]
            app.deep_link().register("nxm").ok();

            // Cold-start: app launched directly by an NXM link click.
            // With single-instance running, the "app already open" case is handled
            // by the single-instance callback above; this only fires for a fresh start.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    raise_and_deliver_nxm(&handle, &url.to_string());
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            discover_steam_libraries,
            scan_games,
            read_cover_image,
            load_settings,
            save_settings,
            list_mods,
            toggle_mod,
            uninstall_mod,
            list_display_mods,
            toggle_display_mod,
            uninstall_display_mod,
            nexus_lookup,
            nexus_collection_lookup,
            nexus_install,
            open_nexusmods_auth,
            open_nexusmods_login,
            open_in_browser,
            lmm_debug_log,
            save_nexus_username,
            nexusmods_login_complete,
            nexusmods_download_urls_ready,
            nexus_install_from_url,
            get_pending_nxm_url,
            clear_pending_nxm_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running application");
}
