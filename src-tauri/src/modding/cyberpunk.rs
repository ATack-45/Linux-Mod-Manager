use std::fs;
use std::path::Path;

use super::{Mod, ModType};

// ── Name cleaning ─────────────────────────────────────────────────────
//
// Converts raw filenames/dirnames into readable display names:
//   "###-LUTSwitcher-Addon-Preem.archive" → "LUT Switcher Addon Preem"
//   "EquipmentEx.archive.xl"              → "Equipment Ex"
//   "PhotoModeScope.xl"                   → "Photo Mode Scope"
//   "ArchiveXL"                           → "Archive XL"

fn clean_mod_name(raw: &str) -> String {
    // Strip .disabled suffix
    let s = raw.strip_suffix(".disabled").unwrap_or(raw);

    // Strip known file extensions (longest match first)
    let s = s
        .strip_suffix(".archive.xl")
        .or_else(|| s.strip_suffix(".archive"))
        .or_else(|| s.strip_suffix(".xl"))
        .or_else(|| s.strip_suffix(".yaml"))
        .or_else(|| s.strip_suffix(".yml"))
        .or_else(|| s.strip_suffix(".tweak"))
        .unwrap_or(s);

    // Strip leading Nexus sort prefix: digits or '#'/'@'/'!' chars before first '-'
    // e.g. "###-LUTSwitcher-Core" → "LUTSwitcher-Core"
    //      "123-CoolMod"          → "CoolMod"
    let s = if let Some(dash) = s.find('-') {
        let prefix = &s[..dash];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit() || "#@!".contains(c)) {
            &s[dash + 1..]
        } else {
            s
        }
    } else {
        s
    };

    // Split on '-' and '_', then expand CamelCase within each token
    let mut words: Vec<String> = Vec::new();
    for token in s.split(|c| c == '-' || c == '_') {
        if token.is_empty() {
            continue;
        }
        expand_camel(token, &mut words);
    }

    words.join(" ")
}

// Splits a CamelCase/digit-mixed token into words and appends them to `out`.
//   "LUTSwitcher"  → ["LUT", "Switcher"]
//   "Nova4"        → ["Nova", "4"]
//   "RED4ext"      → ["RED", "4", "ext"]
//   "Addon"        → ["Addon"]
fn expand_camel(s: &str, out: &mut Vec<String>) {
    let chars: Vec<char> = s.chars().collect();
    let mut start = 0;

    for i in 1..chars.len() {
        let prev = chars[i - 1];
        let curr = chars[i];
        let next = chars.get(i + 1).copied();

        let split = {
            // lowercase/digit → uppercase letter:  "fooBar", "v2Bar"
            let lower_to_upper = !prev.is_uppercase() && curr.is_uppercase();
            // uppercase run → new title word: "LUTSwitcher" (T→S before 'w')
            let upper_run_end = prev.is_uppercase()
                && curr.is_uppercase()
                && next.map(|n| n.is_lowercase()).unwrap_or(false);
            // letter → digit:  "Nova4" → "Nova" "4"
            let letter_to_digit = prev.is_alphabetic() && curr.is_ascii_digit();
            // digit → letter:  "4ext" → "4" "ext"
            let digit_to_letter = prev.is_ascii_digit() && curr.is_alphabetic();

            lower_to_upper || upper_run_end || letter_to_digit || digit_to_letter
        };

        if split {
            let word: String = chars[start..i].iter().collect();
            if !word.is_empty() {
                out.push(word);
            }
            start = i;
        }
    }

    let tail: String = chars[start..].iter().collect();
    if !tail.is_empty() {
        out.push(tail);
    }
}

// ── Archive mods (archive/pc/mod/) ───────────────────────────────────
//
// Files with .archive or standalone .xl extensions.
// .archive.xl is a sidecar patch file — skipped when its parent .archive
// exists in the same directory so the mod only appears once.

fn scan_archive_mods(install_dir: &Path, mods: &mut Vec<Mod>) {
    let dir = install_dir.join("archive/pc/mod");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let all_entries: Vec<_> = entries.flatten().collect();

    // Build the set of present base names (without .disabled) for companion detection
    let present: std::collections::HashSet<String> = all_entries
        .iter()
        .filter_map(|e| e.path().file_name().and_then(|n| n.to_str()).map(String::from))
        .map(|name| {
            name.strip_suffix(".disabled")
                .unwrap_or(&name)
                .to_string()
        })
        .collect();

    for entry in &all_entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let base_name = filename
            .strip_suffix(".disabled")
            .unwrap_or(&filename)
            .to_string();
        let enabled = !filename.ends_with(".disabled");

        if !base_name.ends_with(".archive")
            && !base_name.ends_with(".xl")
            && !base_name.ends_with(".archive.xl")
        {
            continue;
        }

        // Skip .archive.xl sidecar when the parent .archive is also present
        if base_name.ends_with(".archive.xl") {
            let parent = base_name.strip_suffix(".xl").unwrap();
            if present.contains(parent) {
                continue;
            }
        }

        mods.push(Mod {
            name: clean_mod_name(&filename),
            mod_type: ModType::Archive,
            path: path.to_string_lossy().into_owned(),
            enabled,
        });
    }
}

// ── REDmod (mods/) ───────────────────────────────────────────────────
//
// Subdirectories that contain an info.json. Disabled = .disabled suffix
// on the directory name.

fn scan_redmod_mods(install_dir: &Path, mods: &mut Vec<Mod>) {
    let dir = install_dir.join("mods");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dirname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let enabled = !dirname.ends_with(".disabled");

        // Must contain info.json to be a valid REDmod
        if !path.join("info.json").exists() {
            continue;
        }

        mods.push(Mod {
            name: clean_mod_name(&dirname),
            mod_type: ModType::Redmod,
            path: path.to_string_lossy().into_owned(),
            enabled,
        });
    }
}

// ── CET mods (bin/x64/plugins/cyber_engine_tweaks/mods/) ─────────────
//
// Subdirectories. Disabled = .disabled suffix on the directory name.

fn scan_cet_mods(install_dir: &Path, mods: &mut Vec<Mod>) {
    let dir = install_dir.join("bin/x64/plugins/cyber_engine_tweaks/mods");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dirname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let enabled = !dirname.ends_with(".disabled");

        mods.push(Mod {
            name: clean_mod_name(&dirname),
            mod_type: ModType::Cet,
            path: path.to_string_lossy().into_owned(),
            enabled,
        });
    }
}

// ── RED4ext plugins (red4ext/plugins/) ───────────────────────────────
//
// Subdirectories that contain a .dll file. Disabled = .disabled suffix
// on the directory name.

fn scan_red4ext_mods(install_dir: &Path, mods: &mut Vec<Mod>) {
    let dir = install_dir.join("red4ext/plugins");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dirname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let enabled = !dirname.ends_with(".disabled");

        // Must contain at least one .dll to be a valid RED4ext plugin
        let has_dll = fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("dll"))
                    .unwrap_or(false)
            });
        if !has_dll {
            continue;
        }

        mods.push(Mod {
            name: clean_mod_name(&dirname),
            mod_type: ModType::Red4ext,
            path: path.to_string_lossy().into_owned(),
            enabled,
        });
    }
}

// ── REDscript mods (r6/scripts/) ─────────────────────────────────────
//
// Subdirectories containing .reds files. Disabled = .disabled suffix
// on the directory name.

fn scan_redscript_mods(install_dir: &Path, mods: &mut Vec<Mod>) {
    let dir = install_dir.join("r6/scripts");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dirname = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let enabled = !dirname.ends_with(".disabled");

        // Must contain at least one .reds file (recursively) to count
        if !dir_contains_extension(&path, "reds") {
            continue;
        }

        mods.push(Mod {
            name: clean_mod_name(&dirname),
            mod_type: ModType::Redscript,
            path: path.to_string_lossy().into_owned(),
            enabled,
        });
    }
}

// ── TweakXL tweaks (r6/tweaks/) ───────────────────────────────────────
//
// Both subdirectories and individual .yaml/.yml/.tweak files.
// Disabled = .disabled suffix on the name.

fn scan_tweak_mods(install_dir: &Path, mods: &mut Vec<Mod>) {
    let dir = install_dir.join("r6/tweaks");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let entry_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        if path.is_dir() {
            let enabled = !entry_name.ends_with(".disabled");
            mods.push(Mod {
                name: clean_mod_name(&entry_name),
                mod_type: ModType::Tweak,
                path: path.to_string_lossy().into_owned(),
                enabled,
            });
        } else if path.is_file() {
            let base = entry_name
                .strip_suffix(".disabled")
                .unwrap_or(&entry_name);

            if !base.ends_with(".yaml")
                && !base.ends_with(".yml")
                && !base.ends_with(".tweak")
            {
                continue;
            }

            let enabled = !entry_name.ends_with(".disabled");
            mods.push(Mod {
                name: clean_mod_name(&entry_name),
                mod_type: ModType::Tweak,
                path: path.to_string_lossy().into_owned(),
                enabled,
            });
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────

fn dir_contains_extension(dir: &Path, ext: &str) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() {
            if p.extension().and_then(|e| e.to_str()) == Some(ext) {
                return true;
            }
        } else if p.is_dir() && dir_contains_extension(&p, ext) {
            return true;
        }
    }
    false
}

// ── Component derivation ─────────────────────────────────────────────
//
// Given the flat list of relative file paths returned by install_zip,
// return the deduplicated set of scanner-visible top-level paths.
// These are the same paths our mod scanner detects, so they work
// directly with toggle_mod and uninstall_mod.

pub fn derive_components(install_dir: &str, extracted: &[String]) -> Vec<String> {
    let mut components = std::collections::HashSet::new();

    for raw in extracted {
        let path = raw.replace('\\', "/");

        // Direct archive/xl files
        if path.starts_with("archive/pc/mod/") {
            components.insert(format!("{}/{}", install_dir, path));
            continue;
        }

        // CET mods: take the immediate subdir name
        if let Some(rest) = path.strip_prefix("bin/x64/plugins/cyber_engine_tweaks/mods/") {
            if let Some(name) = rest.split('/').next().filter(|s| !s.is_empty()) {
                components.insert(format!(
                    "{}/bin/x64/plugins/cyber_engine_tweaks/mods/{}",
                    install_dir, name
                ));
            }
            continue;
        }

        // RED4ext plugins: take the immediate subdir name
        if let Some(rest) = path.strip_prefix("red4ext/plugins/") {
            if let Some(name) = rest.split('/').next().filter(|s| !s.is_empty()) {
                components.insert(format!("{}/red4ext/plugins/{}", install_dir, name));
            }
            continue;
        }

        // REDscript: take the immediate subdir name
        if let Some(rest) = path.strip_prefix("r6/scripts/") {
            if let Some(name) = rest.split('/').next().filter(|s| !s.is_empty()) {
                components.insert(format!("{}/r6/scripts/{}", install_dir, name));
            }
            continue;
        }

        // TweakXL: direct files or immediate subdir
        if let Some(rest) = path.strip_prefix("r6/tweaks/") {
            if let Some(name) = rest.split('/').next().filter(|s| !s.is_empty()) {
                components.insert(format!("{}/r6/tweaks/{}", install_dir, name));
            }
            continue;
        }

        // REDmod: take the immediate subdir name
        if let Some(rest) = path.strip_prefix("mods/") {
            if let Some(name) = rest.split('/').next().filter(|s| !s.is_empty()) {
                components.insert(format!("{}/mods/{}", install_dir, name));
            }
            continue;
        }

        // Anything else: ignore (engine files, config, etc. are not mod scanner targets)
    }

    let mut result: Vec<String> = components.into_iter().collect();
    result.sort();
    result
}

// ── Public entry points ───────────────────────────────────────────────

pub fn list_mods(install_dir: &str) -> Vec<Mod> {
    let base = Path::new(install_dir);
    let mut mods = Vec::new();

    scan_archive_mods(base, &mut mods);
    scan_redmod_mods(base, &mut mods);
    scan_cet_mods(base, &mut mods);
    scan_red4ext_mods(base, &mut mods);
    scan_redscript_mods(base, &mut mods);
    scan_tweak_mods(base, &mut mods);

    mods.sort_by(|a, b| {
        let type_ord = format!("{:?}", a.mod_type).cmp(&format!("{:?}", b.mod_type));
        type_ord.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    mods
}
