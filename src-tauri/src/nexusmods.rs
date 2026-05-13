use reqwest::header;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

const API_BASE: &str = "https://api.nexusmods.com/v1";

// ── Public data types ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct NexusModInfo {
    pub mod_id: u64,
    pub name: String,
    pub summary: String,
    pub version: String,
    pub picture_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NexusModFile {
    pub file_id: u64,
    pub name: String,
    pub version: String,
    pub size_kb: u64,
    pub category: String,
}

// Internal parsed reference from a URL or NXM link
pub struct ParsedModRef {
    pub game_domain: String,
    pub mod_id: u64,
    pub file_id: Option<u64>,
    pub nxm_key: Option<String>,
    pub nxm_expires: Option<u64>,
}

// ── URL / NXM parsing ─────────────────────────────────────────────────

// Accepts:
//   https://www.nexusmods.com/cyberpunk2077/mods/6945
//   https://www.nexusmods.com/cyberpunk2077/mods/6945?tab=files
//   nxm://cyberpunk2077/mods/6945/files/12345?key=abc&expires=9999&user_id=1
pub fn parse_mod_ref(input: &str) -> Result<ParsedModRef, String> {
    let input = input.trim();

    if input.starts_with("nxm://") {
        parse_nxm(input)
    } else if input.contains("nexusmods.com") {
        parse_web_url(input)
    } else {
        Err("Not a recognised NexusMods URL or NXM link".to_string())
    }
}

fn parse_web_url(input: &str) -> Result<ParsedModRef, String> {
    // Strip query string and fragment
    let path = input
        .split('?')
        .next()
        .unwrap_or(input)
        .split('#')
        .next()
        .unwrap_or(input);

    // nexusmods.com/{game}/mods/{id}
    let parts: Vec<&str> = path
        .trim_end_matches('/')
        .split('/')
        .collect();

    let mods_idx = parts
        .iter()
        .position(|&s| s == "mods")
        .ok_or("URL does not contain /mods/")?;

    let game_domain = parts
        .get(mods_idx - 1)
        .ok_or("Could not determine game from URL")?
        .to_string();

    let mod_id: u64 = parts
        .get(mods_idx + 1)
        .ok_or("Could not determine mod ID from URL")?
        .parse()
        .map_err(|_| "Mod ID is not a number")?;

    Ok(ParsedModRef {
        game_domain,
        mod_id,
        file_id: None,
        nxm_key: None,
        nxm_expires: None,
    })
}

fn parse_nxm(input: &str) -> Result<ParsedModRef, String> {
    // nxm://cyberpunk2077/mods/6945/files/12345?key=abc&expires=9999
    let without_scheme = input.strip_prefix("nxm://").unwrap_or(input);
    let (path_part, query_part) = without_scheme
        .split_once('?')
        .unwrap_or((without_scheme, ""));

    let parts: Vec<&str> = path_part.trim_end_matches('/').split('/').collect();
    // parts: [game, "mods", mod_id, "files", file_id]
    if parts.len() < 5 {
        return Err("NXM link is malformed (too short)".to_string());
    }

    let game_domain = parts[0].to_string();
    let mod_id: u64 = parts[2].parse().map_err(|_| "NXM mod ID is not a number")?;
    let file_id: u64 = parts[4].parse().map_err(|_| "NXM file ID is not a number")?;

    let mut nxm_key = None;
    let mut nxm_expires = None;
    for param in query_part.split('&') {
        if let Some(v) = param.strip_prefix("key=") {
            nxm_key = Some(v.to_string());
        } else if let Some(v) = param.strip_prefix("expires=") {
            nxm_expires = v.parse().ok();
        }
    }

    Ok(ParsedModRef {
        game_domain,
        mod_id,
        file_id: Some(file_id),
        nxm_key,
        nxm_expires,
    })
}

// ── HTTP helpers ──────────────────────────────────────────────────────

fn api_client(api_key: &str) -> Result<reqwest::Client, String> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        "apikey",
        header::HeaderValue::from_str(api_key)
            .map_err(|_| "Invalid API key characters".to_string())?,
    );
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| e.to_string())
}

// ── API calls ─────────────────────────────────────────────────────────

pub async fn fetch_mod_info(
    api_key: &str,
    r: &ParsedModRef,
) -> Result<NexusModInfo, String> {
    #[derive(Deserialize)]
    struct ApiResponse {
        mod_id: u64,
        name: String,
        summary: Option<String>,
        version: Option<String>,
        picture_url: Option<String>,
    }

    let url = format!("{}/games/{}/mods/{}.json", API_BASE, r.game_domain, r.mod_id);
    let resp: ApiResponse = api_client(api_key)?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("NexusMods API error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    Ok(NexusModInfo {
        mod_id: resp.mod_id,
        name: resp.name,
        summary: resp.summary.unwrap_or_default(),
        version: resp.version.unwrap_or_default(),
        picture_url: resp.picture_url,
    })
}

pub async fn fetch_mod_files(
    api_key: &str,
    r: &ParsedModRef,
) -> Result<Vec<NexusModFile>, String> {
    #[derive(Deserialize)]
    struct FileEntry {
        file_id: u64,
        name: Option<String>,
        version: Option<String>,
        size_kb: Option<u64>,
        category_name: Option<String>,
    }
    #[derive(Deserialize)]
    struct ApiResponse {
        files: Vec<FileEntry>,
    }

    let url = format!(
        "{}/games/{}/mods/{}/files.json",
        API_BASE, r.game_domain, r.mod_id
    );
    let resp: ApiResponse = api_client(api_key)?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("NexusMods API error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    let mut files: Vec<NexusModFile> = resp
        .files
        .into_iter()
        .map(|f| NexusModFile {
            file_id: f.file_id,
            name: f.name.unwrap_or_default(),
            version: f.version.unwrap_or_default(),
            size_kb: f.size_kb.unwrap_or(0),
            category: f.category_name.unwrap_or_else(|| "MAIN".to_string()),
        })
        .filter(|f| {
            let cat = f.category.to_uppercase();
            cat == "MAIN" || cat == "UPDATE" || cat == "OPTIONAL"
        })
        .collect();

    // Sort newest-first within each category (higher file_id = later upload)
    files.sort_by(|a, b| {
        let cat_ord = match a.category.to_uppercase().as_str() {
            "MAIN" => 0u8,
            "UPDATE" => 1,
            _ => 2,
        }
        .cmp(&match b.category.to_uppercase().as_str() {
            "MAIN" => 0u8,
            "UPDATE" => 1,
            _ => 2,
        });
        cat_ord.then(b.file_id.cmp(&a.file_id))
    });

    // For MAIN and UPDATE keep only the single latest upload.
    // OPTIONAL files each serve a distinct purpose, so keep all of them.
    let mut seen_main = false;
    let mut seen_update = false;
    files.retain(|f| match f.category.to_uppercase().as_str() {
        "MAIN" => {
            if seen_main {
                false
            } else {
                seen_main = true;
                true
            }
        }
        "UPDATE" => {
            if seen_update {
                false
            } else {
                seen_update = true;
                true
            }
        }
        _ => true,
    });

    Ok(files)
}

pub async fn get_download_url(
    api_key: &str,
    r: &ParsedModRef,
    file_id: u64,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct DownloadLink {
        #[serde(rename = "URI")]
        uri: String,
    }

    let mut url = format!(
        "{}/games/{}/mods/{}/files/{}/download_link.json",
        API_BASE, r.game_domain, r.mod_id, file_id
    );

    // Append NXM key params if present (required for free-tier users)
    if let (Some(key), Some(expires)) = (&r.nxm_key, r.nxm_expires) {
        url = format!("{url}?key={key}&expires={expires}");
    }

    let links: Vec<DownloadLink> = api_client(api_key)?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?
        .error_for_status()
        .map_err(|e| {
            if e.status().map(|s| s.as_u16()) == Some(403) {
                "Download links require a NexusMods Premium account or an NXM link with a valid key. Use the 'Mod Manager Download' button on the NexusMods website to get an NXM link.".to_string()
            } else {
                format!("NexusMods API error: {e}")
            }
        })?
        .json()
        .await
        .map_err(|e| format!("Failed to parse download links: {e}"))?;

    links
        .into_iter()
        .next()
        .map(|l| l.uri)
        .ok_or_else(|| "No download links returned".to_string())
}

// ── Download ──────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct DownloadProgress {
    pct: u8,
    downloaded_kb: u64,
    total_kb: u64,
}

pub async fn download_file(
    url: &str,
    dest: &Path,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let mut resp = reqwest::get(url)
        .await
        .map_err(|e| format!("Download request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download error: {e}"))?;

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut file = fs::File::create(dest).map_err(|e| format!("Cannot create temp file: {e}"))?;

    // Use chunk() instead of bytes_stream() to avoid a futures-util dependency
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("Stream error: {e}"))?
    {
        file.write_all(&chunk)
            .map_err(|e| format!("Write error: {e}"))?;
        downloaded += chunk.len() as u64;

        if total > 0 {
            let pct = ((downloaded * 100) / total).min(100) as u8;
            let _ = app_handle.emit(
                "download-progress",
                DownloadProgress {
                    pct,
                    downloaded_kb: downloaded / 1024,
                    total_kb: total / 1024,
                },
            );
        }
    }

    Ok(())
}

// ── Installation ──────────────────────────────────────────────────────

// Known top-level directories that indicate a game-relative mod layout
const GAME_ROOT_DIRS: &[&str] = &[
    "archive", "red4ext", "bin", "r6", "mods", "engine",
];

pub fn install_zip(zip_path: &Path, game_root: &str) -> Result<Vec<String>, String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    // Collect all entry names to decide the install strategy
    let entry_names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
        .collect();

    let strategy = detect_install_strategy(&entry_names)?;
    let mut installed = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let raw_name = entry.name().to_string();

        if raw_name.ends_with('/') {
            continue; // directory entry
        }

        let dest_rel = match &strategy {
            InstallStrategy::GameRelative => raw_name.clone(),
            InstallStrategy::DropIntoArchiveMod => {
                format!("archive/pc/mod/{}", raw_name)
            }
        };

        let dest_path = PathBuf::from(game_root).join(&dest_rel);

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create dir {}: {e}", parent.display()))?;
        }

        let mut dest_file = fs::File::create(&dest_path)
            .map_err(|e| format!("Cannot create {}: {e}", dest_path.display()))?;

        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        dest_file.write_all(&buf).map_err(|e| e.to_string())?;

        installed.push(dest_rel);
    }

    Ok(installed)
}

enum InstallStrategy {
    // Zip has game-relative paths (archive/pc/mod/, red4ext/plugins/, etc.)
    GameRelative,
    // Zip contains only .archive/.xl files at root — drop into archive/pc/mod/
    DropIntoArchiveMod,
}

fn detect_install_strategy(entries: &[String]) -> Result<InstallStrategy, String> {
    // Check if any entry starts with a known game-root directory
    let has_game_root = entries.iter().any(|name| {
        let first_seg = name.split('/').next().unwrap_or("");
        GAME_ROOT_DIRS.contains(&first_seg)
    });

    if has_game_root {
        return Ok(InstallStrategy::GameRelative);
    }

    // Check if all files are archive-type at the root
    let all_archive = entries.iter().filter(|n| !n.ends_with('/')).all(|name| {
        let base = name.split('/').last().unwrap_or(name);
        base.ends_with(".archive") || base.ends_with(".xl") || base.ends_with(".archive.xl")
    });

    if all_archive {
        return Ok(InstallStrategy::DropIntoArchiveMod);
    }

    // Unknown structure — report back what we found
    let top_level: std::collections::BTreeSet<&str> = entries
        .iter()
        .map(|n| n.split('/').next().unwrap_or(n.as_str()))
        .collect();
    let dirs = top_level
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    Err(format!(
        "Unrecognised mod structure. Top-level entries: {dirs}. \
         Please install this mod manually."
    ))
}

// ── Public entry points (called from main.rs commands) ────────────────

pub async fn lookup(
    api_key: &str,
    input: &str,
) -> Result<(NexusModInfo, Vec<NexusModFile>), String> {
    let r = parse_mod_ref(input)?;
    let info = fetch_mod_info(api_key, &r).await?;
    let files = fetch_mod_files(api_key, &r).await?;
    Ok((info, files))
}

pub async fn install(
    api_key: &str,
    input: &str,
    file_id: u64,
    install_dir: &str,
    app_handle: &AppHandle,
) -> Result<Vec<String>, String> {
    let r = parse_mod_ref(input)?;
    let download_url = get_download_url(api_key, &r, file_id).await?;

    // Download to a temp file
    let tmp_path = std::env::temp_dir().join(format!("lmm_mod_{file_id}.zip"));
    download_file(&download_url, &tmp_path, app_handle).await?;

    // Install and clean up temp file
    let result = install_zip(&tmp_path, install_dir);
    let _ = fs::remove_file(&tmp_path);
    result
}
