use std::fs;
use std::path::PathBuf;

use rusty_loader::cli::HttpMethod;
use rusty_loader::config::{BenchmarkConfigInput, BodyMode, HeaderEntry};
use rusty_loader::presets::{
    format_cli_command, PresetStore, PresetUpsertRequest, MAX_RECENT_CONFIGS,
};
use tempfile::TempDir;

fn sample_input(url: &str) -> BenchmarkConfigInput {
    BenchmarkConfigInput {
        url: url.to_string(),
        method: HttpMethod::Post,
        concurrency: 120,
        duration: "15s".to_string(),
        ramp_up: "2s".to_string(),
        timeout: "2500ms".to_string(),
        headers: vec![
            HeaderEntry {
                name: "Authorization".to_string(),
                value: "Bearer super-secret-token".to_string(),
                enabled: true,
            },
            HeaderEntry {
                name: "Content-Type".to_string(),
                value: "application/json".to_string(),
                enabled: true,
            },
        ],
        body_mode: BodyMode::Raw,
        body: Some(r#"{"ping":true}"#.to_string()),
        body_file: None,
        output_json: None,
    }
}

#[test]
fn test_preset_store_crud_persistence_and_permissions() {
    let dir = TempDir::new().expect("temp dir");
    let store = PresetStore::load_from_dir(dir.path().to_path_buf()).expect("load store");

    assert!(store.list().presets.is_empty());

    // Reject empty name
    let err = store
        .create(PresetUpsertRequest {
            name: "   ".to_string(),
            description: None,
            config: sample_input("https://api.example.com/v1/ping"),
        })
        .unwrap_err();
    assert!(err.to_string().contains("empty"));

    // Create
    let created = store
        .create(PresetUpsertRequest {
            name: "Prod Ping".to_string(),
            description: Some("Baseline check".to_string()),
            config: sample_input("https://api.example.com/v1/ping"),
        })
        .expect("create preset");
    assert_eq!(created.name, "Prod Ping");
    assert_eq!(created.description.as_deref(), Some("Baseline check"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(store.file_path())
            .expect("presets.json metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "presets.json must have 0600 permissions");
    }

    // Update
    let updated = store
        .update(
            &created.id,
            PresetUpsertRequest {
                name: "Prod Ping Updated".to_string(),
                description: None,
                config: sample_input("https://api.example.com/v2/ping"),
            },
        )
        .expect("update should succeed")
        .expect("preset should exist");
    assert_eq!(updated.name, "Prod Ping Updated");
    assert_eq!(updated.config.url, "https://api.example.com/v2/ping");

    // Reload from disk and verify persistence
    let reloaded = PresetStore::load_from_dir(dir.path().to_path_buf()).expect("reload store");
    let list = reloaded.list();
    assert_eq!(list.presets.len(), 1);
    assert_eq!(list.presets[0].id, created.id);
    assert_eq!(list.presets[0].name, "Prod Ping Updated");

    // Delete
    assert!(reloaded.delete(&created.id).expect("delete should succeed"));
    assert!(!reloaded
        .delete("nonexistent")
        .expect("missing delete returns false"));

    let reloaded_after_delete =
        PresetStore::load_from_dir(dir.path().to_path_buf()).expect("reload after delete");
    assert!(reloaded_after_delete.list().presets.is_empty());
}

#[test]
fn test_preset_store_recent_dedup_cap_and_corrupt_recovery() {
    let dir = TempDir::new().expect("temp dir");
    let corrupt_path = dir.path().join("presets.json");
    fs::write(&corrupt_path, "{not-valid-json").expect("write corrupt file");

    let store = PresetStore::load_from_dir(dir.path().to_path_buf())
        .expect("corrupt presets.json should recover gracefully");
    assert!(store.list().presets.is_empty());

    for i in 0..(MAX_RECENT_CONFIGS + 5) {
        let cfg = sample_input(&format!("https://api.example.com/v1/item/{i}"));
        store
            .record_recent(cfg, Some(format!("Preset {i}")))
            .expect("record recent");
    }

    // Re-record an older config and verify deduplication moves it to front without exceeding cap
    let repeat_cfg = sample_input("https://api.example.com/v1/item/10");
    store
        .record_recent(repeat_cfg.clone(), Some("Repeated".to_string()))
        .expect("record repeat");

    let list = store.list();
    assert_eq!(list.recent.len(), MAX_RECENT_CONFIGS);
    assert_eq!(list.recent[0].config, repeat_cfg);
    assert_eq!(list.recent[0].preset_name.as_deref(), Some("Repeated"));
}

#[test]
fn test_format_cli_command_redaction_and_file_body() {
    let mut cfg = sample_input("https://svc_user:pass123@api.example.com/v1/orders");
    cfg.body_mode = BodyMode::File;
    cfg.body = None;
    cfg.body_file = Some(PathBuf::from("fixtures/order payload.json"));
    cfg.output_json = Some(PathBuf::from("artifacts/report.json"));

    let redacted_cmd = format_cli_command(&cfg, true);
    assert!(!redacted_cmd.contains("super-secret-token"));
    assert!(!redacted_cmd.contains("pass123"));
    assert!(redacted_cmd.contains("Authorization: [REDACTED]"));
    assert!(redacted_cmd.contains("--method POST"));
    assert!(redacted_cmd.contains("--concurrency 120"));
    assert!(redacted_cmd.contains("--body '@fixtures/order payload.json'"));
    assert!(redacted_cmd.contains("--output-json artifacts/report.json"));

    let raw_cmd = format_cli_command(&cfg, false);
    assert!(raw_cmd.contains("super-secret-token"));
    assert!(raw_cmd.contains("pass123"));
}
