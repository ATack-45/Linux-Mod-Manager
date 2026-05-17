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

#[derive(Serialize, Deserialize, Clone)]
pub struct NexusCollectionInfo {
    pub name: String,
    pub summary: String,
    pub slug: String,
    pub revision: u32,
    pub mod_count: usize,
    pub game_domain: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NexusCollectionMod {
    pub mod_id: u64,
    pub file_id: u64,
    pub name: String,
    pub version: String,
    pub game_domain: String,
    pub optional: bool,
}

// Internal parsed reference from a URL or NXM link
pub struct ParsedModRef {
    pub game_domain: String,
    pub mod_id: u64,
    pub file_id: Option<u64>,
    pub nxm_key: Option<String>,
    pub nxm_expires: Option<u64>,
    pub nxm_user_id: Option<u64>,
}

pub struct ParsedCollectionRef {
    pub game_domain: String,
    pub slug: String,
    pub revision: u32,
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
        nxm_user_id: None,
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

    if parts.get(1).copied() == Some("collections") {
        return Err(
            "This is a collection NXM link — use nexus_collection_lookup instead".to_string(),
        );
    }

    let game_domain = parts[0].to_string();
    let mod_id: u64 = parts[2].parse().map_err(|_| "NXM mod ID is not a number")?;
    let file_id: u64 = parts[4].parse().map_err(|_| "NXM file ID is not a number")?;

    let mut nxm_key = None;
    let mut nxm_expires = None;
    let mut nxm_user_id = None;
    for param in query_part.split('&') {
        if let Some(v) = param.strip_prefix("key=") {
            nxm_key = Some(v.to_string());
        } else if let Some(v) = param.strip_prefix("expires=") {
            nxm_expires = v.parse().ok();
        } else if let Some(v) = param.strip_prefix("user_id=") {
            nxm_user_id = v.parse().ok();
        }
    }

    Ok(ParsedModRef {
        game_domain,
        mod_id,
        file_id: Some(file_id),
        nxm_key,
        nxm_expires,
        nxm_user_id,
    })
}

pub fn parse_collection_ref(input: &str) -> Result<ParsedCollectionRef, String> {
    // nxm://cyberpunk2077/collections/devnx1/revisions/40
    let without_scheme = input.strip_prefix("nxm://").unwrap_or(input);
    let path_part = without_scheme.split('?').next().unwrap_or(without_scheme);
    let parts: Vec<&str> = path_part.trim_end_matches('/').split('/').collect();
    // parts: [game, "collections", slug, "revisions", revision]
    if parts.len() < 5 || parts.get(1).copied() != Some("collections") || parts.get(3).copied() != Some("revisions") {
        return Err("Not a valid collection NXM link".to_string());
    }
    let game_domain = parts[0].to_string();
    let slug = parts[2].to_string();
    let revision: u32 = parts[4]
        .parse()
        .map_err(|_| "Collection revision number is not valid".to_string())?;
    Ok(ParsedCollectionRef { game_domain, slug, revision })
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

    // Append NXM key params if present (required for free-tier users).
    // user_id is client-side metadata in the NXM link and is NOT sent to the API.
    if let (Some(key), Some(expires)) = (&r.nxm_key, r.nxm_expires) {
        url = format!("{url}?key={key}&expires={expires}");
    }

    let key_hint = if api_key.len() >= 6 { &api_key[..6] } else { api_key };
    eprintln!("[download_url] GET {url}  (api_key starts: {key_hint}...)");

    let resp = api_client(api_key)?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("Failed to read response: {e}"))?;
    eprintln!("[download_url] HTTP {status}  body: {body}");

    if !status.is_success() {
        return Err(match status.as_u16() {
            403 => "Download links require a NexusMods Premium account or a valid NXM key.".to_string(),
            400 => format!(
                "NexusMods rejected the download key (HTTP 400): {body}\n\
                Make sure you are logged into nexusmods.com in your browser \
                as the same account whose API key is saved in Settings."
            ),
            _ => format!("NexusMods API error: HTTP {status}: {body}"),
        });
    }

    let links: Vec<DownloadLink> = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse download links: {e}\nBody: {body}"))?;

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

// ── Collection API ────────────────────────────────────────────────────

pub async fn fetch_collection_revision(
    api_key: &str,
    r: &ParsedCollectionRef,
) -> Result<(NexusCollectionInfo, Vec<NexusCollectionMod>), String> {
    // ── Step 1: GraphQL — collection metadata + bundle download link ──
    //
    // We only query scalar fields we know exist. The nested modFiles/mod
    // schema has changed multiple times; instead we fetch the downloadLink
    // which points to a collection.json bundle containing the full mod list.

    const GQL_URL: &str = "https://api.nexusmods.com/v2/graphql";

    const QUERY: &str = "
        query CollectionRevision(
          $slug: String!, $revision: Int!, $domainName: String!, $viewAdultContent: Boolean
        ) {
          collectionRevision(
            slug: $slug, revision: $revision, domainName: $domainName,
            viewAdultContent: $viewAdultContent
          ) {
            collection {
              name
              summary
              slug
            }
            downloadLink
          }
        }
    ";

    #[derive(Deserialize)]
    struct GqlError { message: String }

    #[derive(Deserialize, Default)]
    struct GqlCollection {
        name: Option<String>,
        summary: Option<String>,
        slug: Option<String>,
    }

    #[derive(Deserialize)]
    struct GqlRevision {
        collection: Option<GqlCollection>,
        #[serde(rename = "downloadLink")]
        download_link: Option<String>,
    }

    #[derive(Deserialize)]
    struct GqlData {
        #[serde(rename = "collectionRevision")]
        collection_revision: Option<GqlRevision>,
    }

    #[derive(Deserialize)]
    struct GqlResponse {
        data: Option<GqlData>,
        errors: Option<Vec<GqlError>>,
    }

    let gql_body = serde_json::json!({
        "query": QUERY,
        "variables": {
            "slug": r.slug,
            "revision": r.revision,
            "domainName": r.game_domain,
            "viewAdultContent": true,
        }
    });

    let gql_raw = api_client(api_key)?
        .post(GQL_URL)
        .json(&gql_body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("NexusMods API error: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read collection response: {e}"))?;

    eprintln!("[collection] GraphQL response: {gql_raw}");

    let gql_resp: GqlResponse = serde_json::from_str(&gql_raw)
        .map_err(|e| format!("Failed to parse collection response: {e}\nBody: {gql_raw}"))?;

    if let Some(errs) = gql_resp.errors {
        if !errs.is_empty() {
            let msg = errs.into_iter().map(|e| e.message).collect::<Vec<_>>().join("; ");
            return Err(format!("NexusMods collection error: {msg}"));
        }
    }

    let revision = gql_resp
        .data
        .and_then(|d| d.collection_revision)
        .ok_or_else(|| "Collection revision not found".to_string())?;

    let col = revision.collection.unwrap_or_default();

    let download_link = revision.download_link.ok_or_else(|| {
        "No bundle download link returned — collection may require NexusMods Premium".to_string()
    })?;

    // The API returns a relative path — make it absolute and use the auth client
    let download_url = if download_link.starts_with('/') {
        format!("https://api.nexusmods.com{download_link}")
    } else {
        download_link
    };

    eprintln!("[collection] Bundle download URL: {download_url}");

    // ── Step 2: The download_url endpoint returns CDN link(s) to a .7z archive

    #[derive(Deserialize)]
    struct CdnLink {
        #[serde(rename = "URI")]
        uri: String,
    }
    #[derive(Deserialize)]
    struct DownloadLinksResp {
        download_links: Vec<CdnLink>,
    }

    let links_raw = api_client(api_key)?
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Download link request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download link error: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read download links: {e}"))?;

    eprintln!("[collection] Download links response: {links_raw}");

    let links_resp: DownloadLinksResp = serde_json::from_str(&links_raw)
        .map_err(|e| format!("Failed to parse download links: {e}\nBody: {links_raw}"))?;

    let cdn_url = links_resp
        .download_links
        .into_iter()
        .next()
        .map(|l| l.uri)
        .ok_or_else(|| "No CDN download links returned for collection".to_string())?;

    eprintln!("[collection] CDN URL: {cdn_url}");

    // ── Step 3: Download the .7z archive ─────────────────────────────

    let tmp_path = std::env::temp_dir()
        .join(format!("lmm_collection_{}_{}.7z", r.slug, r.revision));

    let archive_bytes = reqwest::get(&cdn_url)
        .await
        .map_err(|e| format!("Collection archive download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Collection archive download error: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read collection archive bytes: {e}"))?;

    fs::write(&tmp_path, &archive_bytes)
        .map_err(|e| format!("Failed to write collection archive: {e}"))?;

    eprintln!("[collection] Archive saved to {} ({} bytes)", tmp_path.display(), archive_bytes.len());

    // ── Step 4: Extract collection.json from the .7z archive ─────────

    #[derive(Deserialize)]
    struct BundleModSource {
        #[serde(rename = "modId")]
        mod_id: Option<u64>,
        #[serde(rename = "fileId")]
        file_id: Option<u64>,
    }

    #[derive(Deserialize)]
    struct BundleMod {
        name: Option<String>,
        version: Option<String>,
        optional: Option<bool>,
        #[serde(rename = "domainName")]
        domain_name: Option<String>,
        source: Option<BundleModSource>,
    }

    #[derive(Deserialize)]
    struct CollectionBundle {
        mods: Option<Vec<BundleMod>>,
    }

    let json_str = tokio::task::spawn_blocking({
        let tmp_path = tmp_path.clone();
        move || -> Result<String, String> {
            let mut sz = sevenz_rust::SevenZReader::open(&tmp_path, sevenz_rust::Password::empty())
                .map_err(|e| format!("Failed to open collection archive: {e}"))?;
            let mut found: Option<String> = None;
            sz.for_each_entries(&mut |entry: &sevenz_rust::SevenZArchiveEntry, reader: &mut dyn std::io::Read| {
                eprintln!("[collection] Archive entry: {}", entry.name());
                if entry.name().ends_with("collection.json") {
                    let mut buf = String::new();
                    reader.read_to_string(&mut buf)
                        .map_err(sevenz_rust::Error::io)?;
                    found = Some(buf);
                    return Ok(false); // stop iteration
                }
                Ok(true)
            })
            .map_err(|e| format!("Failed to read collection archive entries: {e}"))?;
            let _ = fs::remove_file(&tmp_path);
            found.ok_or_else(|| "collection.json not found inside archive".to_string())
        }
    })
    .await
    .map_err(|e| format!("Archive extraction task panicked: {e}"))??;

    eprintln!("[collection] collection.json (first 300 chars): {}", &json_str[..json_str.len().min(300)]);

    let bundle: CollectionBundle = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse collection.json: {e}"))?;

    // ── Step 5: Build the mod list ────────────────────────────────────

    let collection_mods: Vec<NexusCollectionMod> = bundle
        .mods
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let source = m.source?;
            Some(NexusCollectionMod {
                mod_id: source.mod_id?,
                file_id: source.file_id?,
                name: m.name.unwrap_or_default(),
                version: m.version.unwrap_or_default(),
                game_domain: m.domain_name.unwrap_or_else(|| r.game_domain.clone()),
                optional: m.optional.unwrap_or(false),
            })
        })
        .collect();

    Ok((
        NexusCollectionInfo {
            name: col.name.unwrap_or_else(|| r.slug.clone()),
            summary: col.summary.unwrap_or_default(),
            slug: col.slug.unwrap_or_else(|| r.slug.clone()),
            revision: r.revision,
            mod_count: collection_mods.len(),
            game_domain: r.game_domain.clone(),
        },
        collection_mods,
    ))
}

// ── Public entry points (called from main.rs commands) ────────────────

pub async fn lookup_collection(
    api_key: &str,
    input: &str,
) -> Result<(NexusCollectionInfo, Vec<NexusCollectionMod>), String> {
    let r = parse_collection_ref(input)?;
    fetch_collection_revision(api_key, &r).await
}

pub async fn lookup(
    api_key: &str,
    input: &str,
) -> Result<(NexusModInfo, Vec<NexusModFile>), String> {
    let r = parse_mod_ref(input)?;
    let info = fetch_mod_info(api_key, &r).await?;
    let files = fetch_mod_files(api_key, &r).await?;
    Ok((info, files))
}

// Detect ZIP vs 7z by magic bytes and dispatch to the right extractor.
pub fn install_archive(path: &Path, game_root: &str) -> Result<Vec<String>, String> {
    let mut magic = [0u8; 6];
    let n = fs::File::open(path)
        .and_then(|mut f| f.read(&mut magic))
        .unwrap_or(0);
    // 7z magic: 37 7A BC AF 27 1C
    if n >= 6 && magic == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
        install_7z(path, game_root)
    } else {
        install_zip(path, game_root)
    }
}

fn install_7z(path: &Path, game_root: &str) -> Result<Vec<String>, String> {

    // Buffer all file contents in a single pass — sevenz_rust requires consuming
    // the reader in the callback to advance the iterator, so two-pass would
    // decompress the whole archive twice.
    let mut buffered: Vec<(String, Vec<u8>)> = Vec::new();

    let mut sz = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty())
        .map_err(|e| format!("Cannot open 7z archive: {e}"))?;

    sz.for_each_entries(&mut |entry: &sevenz_rust::SevenZArchiveEntry, reader: &mut dyn std::io::Read| {
        if entry.is_directory() {
            return Ok(true);
        }
        let name = entry.name().replace('\\', "/");
        let mut data = Vec::new();
        reader.read_to_end(&mut data).map_err(sevenz_rust::Error::io)?;
        buffered.push((name, data));
        Ok(true)
    }).map_err(|e| format!("7z extraction failed: {e}"))?;

    let names: Vec<String> = buffered.iter().map(|(n, _)| n.clone()).collect();
    let strategy = detect_install_strategy(&names)?;
    let mut installed = Vec::new();

    for (raw, data) in buffered {
        let dest_rel = match &strategy {
            InstallStrategy::GameRelative => raw.clone(),
            InstallStrategy::DropIntoArchiveMod => format!("archive/pc/mod/{raw}"),
        };
        let dest_path = PathBuf::from(game_root).join(&dest_rel);
        if let Some(p) = dest_path.parent() {
            fs::create_dir_all(p).map_err(|e| format!("Cannot create dir: {e}"))?;
        }
        fs::write(&dest_path, &data).map_err(|e| format!("Cannot write '{dest_rel}': {e}"))?;
        installed.push(dest_rel);
    }

    Ok(installed)
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

    let tmp_path = std::env::temp_dir().join(format!("lmm_mod_{file_id}.tmp"));
    download_file(&download_url, &tmp_path, app_handle).await?;

    let result = install_archive(&tmp_path, install_dir);
    let _ = fs::remove_file(&tmp_path);
    result
}

// Skips get_download_url — caller already has a CDN URL (e.g. from in-app WebView auth)
pub async fn install_from_url(
    url: &str,
    file_id: u64,
    install_dir: &str,
    app_handle: &AppHandle,
) -> Result<Vec<String>, String> {
    let tmp_path = std::env::temp_dir().join(format!("lmm_mod_{file_id}.tmp"));
    download_file(url, &tmp_path, app_handle).await?;
    let result = install_archive(&tmp_path, install_dir);
    let _ = fs::remove_file(&tmp_path);
    result
}

pub async fn fetch_game_id(api_key: &str, domain: &str) -> Result<u64, String> {
    #[derive(Deserialize)]
    struct GameInfo { id: u64 }
    let url = format!("{}/games/{}.json", API_BASE, domain);
    let resp: GameInfo = api_client(api_key)?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Game info request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Game info error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse game info: {e}"))?;
    Ok(resp.id)
}
