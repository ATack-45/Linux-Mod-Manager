use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

// ── Data structures ───────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct InstalledMod {
    pub id: String,
    pub name: String,
    pub version: String,
    pub nexus_mod_id: Option<u64>,
    pub nexus_file_id: Option<u64>,
    // Absolute top-level paths (scanner-visible files + dirs) for toggle/uninstall
    pub components: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct GameManifest {
    pub mods: Vec<InstalledMod>,
}

// ── Persistence ───────────────────────────────────────────────────────

fn manifest_path(app_id: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(format!(".config/linux-mod-manager/mods_{}.json", app_id))
}

pub fn load_manifest(app_id: &str) -> GameManifest {
    let content = match fs::read_to_string(manifest_path(app_id)) {
        Ok(c) => c,
        Err(_) => return GameManifest::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_manifest(app_id: &str, manifest: &GameManifest) -> Result<(), String> {
    let path = manifest_path(app_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn add_mod(app_id: &str, entry: InstalledMod) -> Result<(), String> {
    let mut manifest = load_manifest(app_id);
    // Replace any existing entry with the same id
    manifest.mods.retain(|m| m.id != entry.id);
    manifest.mods.push(entry);
    save_manifest(app_id, &manifest)
}

pub fn remove_mod(app_id: &str, mod_id: &str) -> Result<(), String> {
    let mut manifest = load_manifest(app_id);
    manifest.mods.retain(|m| m.id != mod_id);
    save_manifest(app_id, &manifest)
}

// ── Component enabled-state helpers ───────────────────────────────────

// A managed mod is enabled if its first component exists without a .disabled suffix.
pub fn is_enabled(components: &[String]) -> bool {
    components.first().map_or(false, |p| {
        let path = std::path::Path::new(p);
        if path.exists() {
            return true;
        }
        // Check for disabled variant
        let disabled = format!("{}.disabled", p);
        !std::path::Path::new(&disabled).exists()
    })
}
