//! Local preset and recent configuration persistence plus CLI command formatting.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::cli::HttpMethod;
use crate::config::{is_sensitive_header, sanitize_url, BenchmarkConfigInput, BodyMode};

/// Maximum number of recent run configurations retained in local storage.
pub const MAX_RECENT_CONFIGS: usize = 20;

const CONFIG_DIR_OVERRIDE_ENV: &str = "RUSTY_LOADER_CONFIG_DIR";
const PRESETS_FILENAME: &str = "presets.json";

/// Named, user-saved benchmark configuration preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedPreset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub config: BenchmarkConfigInput,
    pub created_at: String,
    pub updated_at: String,
}

/// Automatically recorded recent benchmark configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentRunConfig {
    pub id: String,
    pub timestamp: String,
    #[serde(default)]
    pub preset_name: Option<String>,
    pub config: BenchmarkConfigInput,
}

/// Serialized JSON schema persisted to `presets.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetsPayload {
    #[serde(default)]
    pub presets: Vec<SavedPreset>,
    #[serde(default)]
    pub recent: Vec<RecentRunConfig>,
}

/// Request payload for creating or updating a [`SavedPreset`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetUpsertRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub config: BenchmarkConfigInput,
}

/// Thread-safe store managing saved presets and recent run configurations on disk.
pub struct PresetStore {
    dir: PathBuf,
    file_path: PathBuf,
    data: RwLock<PresetsPayload>,
    seq: AtomicU64,
}

impl PresetStore {
    /// Resolves the default platform configuration directory and loads `presets.json`.
    pub fn load_default() -> Result<Self> {
        Self::load_from_dir(default_config_dir())
    }

    /// Loads `presets.json` from `dir`, starting empty if no file exists yet.
    pub fn load_from_dir(dir: PathBuf) -> Result<Self> {
        let file_path = dir.join(PRESETS_FILENAME);
        let data = if file_path.exists() {
            match fs::read_to_string(&file_path) {
                Ok(raw) => match serde_json::from_str::<PresetsPayload>(&raw) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        eprintln!(
                            "Warning: failed to parse '{}': {err}; starting with empty presets store.",
                            file_path.display()
                        );
                        let backup_name = format!(
                            "{}.corrupt.{}",
                            PRESETS_FILENAME,
                            Utc::now().timestamp_millis()
                        );
                        let _ = fs::rename(&file_path, dir.join(backup_name));
                        PresetsPayload::default()
                    }
                },
                Err(err) => {
                    eprintln!(
                        "Warning: failed to read '{}': {err}; starting with empty presets store.",
                        file_path.display()
                    );
                    PresetsPayload::default()
                }
            }
        } else {
            PresetsPayload::default()
        };

        Ok(Self {
            dir,
            file_path,
            data: RwLock::new(data),
            seq: AtomicU64::new(1),
        })
    }

    /// Returns the resolved `presets.json` path on disk.
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Returns a snapshot of all saved presets and recent run configurations.
    pub fn list(&self) -> PresetsPayload {
        self.data
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Creates and persists a new [`SavedPreset`].
    pub fn create(&self, req: PresetUpsertRequest) -> Result<SavedPreset> {
        let name = req.name.trim();
        if name.is_empty() {
            bail!("preset name cannot be empty");
        }

        let now = Utc::now().to_rfc3339();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let preset = SavedPreset {
            id: format!("preset-{}-{seq}", Utc::now().timestamp_millis()),
            name: name.to_string(),
            description: normalize_description(req.description),
            config: req.config,
            created_at: now.clone(),
            updated_at: now,
        };

        let mut guard = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.presets.push(preset.clone());
        self.persist_locked(&guard)?;
        Ok(preset)
    }

    /// Updates an existing preset by `id`, returning `None` if no matching preset exists.
    pub fn update(&self, id: &str, req: PresetUpsertRequest) -> Result<Option<SavedPreset>> {
        let name = req.name.trim();
        if name.is_empty() {
            bail!("preset name cannot be empty");
        }

        let mut guard = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(existing) = guard.presets.iter_mut().find(|p| p.id == id) else {
            return Ok(None);
        };

        existing.name = name.to_string();
        existing.description = normalize_description(req.description);
        existing.config = req.config;
        existing.updated_at = Utc::now().to_rfc3339();
        let updated = existing.clone();

        self.persist_locked(&guard)?;
        Ok(Some(updated))
    }

    /// Deletes a saved preset by `id`, returning `true` if a preset was removed.
    pub fn delete(&self, id: &str) -> Result<bool> {
        let mut guard = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = guard.presets.len();
        guard.presets.retain(|p| p.id != id);
        if guard.presets.len() == before {
            return Ok(false);
        }

        self.persist_locked(&guard)?;
        Ok(true)
    }

    /// Records a recently executed configuration, deduplicating identical configs and capping at [`MAX_RECENT_CONFIGS`].
    pub fn record_recent(
        &self,
        config: BenchmarkConfigInput,
        preset_name: Option<String>,
    ) -> Result<()> {
        let mut guard = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.recent.retain(|item| item.config != config);

        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        guard.recent.insert(
            0,
            RecentRunConfig {
                id: format!("recent-{}-{seq}", Utc::now().timestamp_millis()),
                timestamp: Utc::now().to_rfc3339(),
                preset_name,
                config,
            },
        );
        guard.recent.truncate(MAX_RECENT_CONFIGS);

        self.persist_locked(&guard)
    }

    fn persist_locked(&self, payload: &PresetsPayload) -> Result<()> {
        ensure_secure_dir(&self.dir)?;

        let json = serde_json::to_string_pretty(payload)
            .context("failed to serialize presets store payload")?;

        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let tmp_path = self.dir.join(format!(
            ".{}.tmp.{}.{}",
            PRESETS_FILENAME,
            std::process::id(),
            seq
        ));

        let write_result = (|| -> Result<()> {
            let mut opts = OpenOptions::new();
            opts.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }

            let mut file = opts
                .open(&tmp_path)
                .with_context(|| format!("failed to create temp file '{}'", tmp_path.display()))?;
            file.write_all(json.as_bytes())
                .with_context(|| format!("failed to write '{}'", tmp_path.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync '{}'", tmp_path.display()))?;
            drop(file);

            fs::rename(&tmp_path, &self.file_path).with_context(|| {
                format!(
                    "failed to replace presets file '{}'",
                    self.file_path.display()
                )
            })?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }

        write_result
    }
}

/// Formats a [`BenchmarkConfigInput`] as an equivalent shell-escaped `rusty-loader` CLI command.
pub fn format_cli_command(input: &BenchmarkConfigInput, redact_secrets: bool) -> String {
    let mut parts = vec!["rusty-loader".to_string()];

    let url_display = if redact_secrets {
        Url::parse(input.url.trim())
            .map(|u| sanitize_url(&u))
            .unwrap_or_else(|_| input.url.trim().to_string())
    } else {
        input.url.trim().to_string()
    };
    parts.push(format!("--url {}", shell_quote(&url_display)));

    if input.method != HttpMethod::Get {
        parts.push(format!("--method {}", input.method));
    }

    if input.concurrency != 700 {
        parts.push(format!("--concurrency {}", input.concurrency));
    }

    let duration = input.duration.trim();
    if !duration.is_empty() && duration != "30s" {
        parts.push(format!("--duration {}", shell_quote(duration)));
    }

    let ramp_up = input.ramp_up.trim();
    if !ramp_up.is_empty() && ramp_up != "0s" && ramp_up != "0" {
        parts.push(format!("--ramp-up {}", shell_quote(ramp_up)));
    }

    let timeout = input.timeout.trim();
    if !timeout.is_empty() && timeout != "5000ms" {
        parts.push(format!("--timeout {}", shell_quote(timeout)));
    }

    for entry in &input.headers {
        if !entry.enabled || entry.name.trim().is_empty() {
            continue;
        }
        let name = entry.name.trim();
        let value = if redact_secrets && is_sensitive_header(name) {
            "[REDACTED]"
        } else {
            entry.value.trim()
        };
        parts.push(format!(
            "--header {}",
            shell_quote(&format!("{name}: {value}"))
        ));
    }

    match input.body_mode {
        BodyMode::None => {}
        BodyMode::Raw => {
            if let Some(ref body) = input.body {
                if !body.is_empty() {
                    parts.push(format!("--body {}", shell_quote(body)));
                }
            }
        }
        BodyMode::File => {
            if let Some(ref path) = input.body_file {
                let p = path.to_string_lossy();
                let trimmed = p.trim();
                if !trimmed.is_empty() {
                    parts.push(format!("--body {}", shell_quote(&format!("@{trimmed}"))));
                }
            }
        }
    }

    for (flag, path_opt) in [
        ("--output-json", &input.output_json),
        ("--output-csv", &input.output_csv),
        ("--output-pdf", &input.output_pdf),
    ] {
        if let Some(out_path) = path_opt {
            let p = out_path.to_string_lossy();
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                parts.push(format!("{flag} {}", shell_quote(trimmed)));
            }
        }
    }

    parts.join(" ")
}

fn default_config_dir() -> PathBuf {
    if let Some(override_dir) = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV) {
        if !override_dir.is_empty() {
            return PathBuf::from(override_dir);
        }
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("rusty-loader");
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return PathBuf::from(xdg).join("rusty-loader");
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home).join(".config").join("rusty-loader");
            }
        }
    }

    PathBuf::from(".rusty-loader")
}

fn ensure_secure_dir(dir: &Path) -> Result<()> {
    let created = !dir.exists();
    fs::create_dir_all(dir)
        .with_context(|| format!("failed to create config directory '{}'", dir.display()))?;

    #[cfg(unix)]
    if created {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }

    Ok(())
}

fn normalize_description(desc: Option<String>) -> Option<String> {
    desc.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn shell_quote(raw: &str) -> String {
    if raw.is_empty() {
        return "''".to_string();
    }
    if raw
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b':' | b'=' | b'@'))
    {
        return raw.to_string();
    }
    let escaped = raw.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}
