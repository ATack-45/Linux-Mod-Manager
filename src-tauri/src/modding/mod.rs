use serde::{Deserialize, Serialize};
use std::fs;

pub mod cyberpunk;

// ── Shared types ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ModType {
    Archive,
    Redmod,
    Cet,
    Red4ext,
    Redscript,
    Tweak,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Mod {
    pub name: String,
    pub mod_type: ModType,
    pub path: String,
    pub enabled: bool,
}

// ── Dispatch ─────────────────────────────────────────────────────────

pub fn list_mods(app_id: &str, install_dir: &str) -> Vec<Mod> {
    match app_id {
        "1091500" => cyberpunk::list_mods(install_dir),
        _ => vec![],
    }
}

pub fn derive_components(app_id: &str, install_dir: &str, extracted: &[String]) -> Vec<String> {
    match app_id {
        "1091500" => cyberpunk::derive_components(install_dir, extracted),
        _ => vec![],
    }
}

pub fn toggle_mod(path: &str, _mod_type: &ModType, enabled: bool) -> Result<(), String> {
    if enabled {
        // Remove .disabled suffix
        let new_path = path
            .strip_suffix(".disabled")
            .ok_or_else(|| format!("path does not end in .disabled: {}", path))?;
        fs::rename(path, new_path).map_err(|e| e.to_string())
    } else {
        // Add .disabled suffix
        let new_path = format!("{}.disabled", path);
        fs::rename(path, &new_path).map_err(|e| e.to_string())
    }
}
