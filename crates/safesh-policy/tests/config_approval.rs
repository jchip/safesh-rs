use std::time::Duration;

use safesh_policy::config::{ApprovalConfig, Config, merge_configs, validate_config};

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| (*x).to_string()).collect()
}

fn with(a: ApprovalConfig) -> Config {
    Config {
        approval: Some(a),
        ..Config::default()
    }
}

#[test]
fn approval_parse_full_block() {
    let c: Config = serde_json::from_str(
        r#"{"approval":{"channels":["tmux"],"timeoutSec":10,"runtimeTimeoutSec":5}}"#,
    )
    .unwrap();
    let r = ApprovalConfig::resolved(c.approval.as_ref());
    assert_eq!(r.channels, s(&["tmux"]));
    assert_eq!(r.timeout, Duration::from_secs(10));
    assert_eq!(r.runtime_timeout, Duration::from_secs(5));
}

#[test]
fn approval_missing_block_gives_defaults() {
    let c: Config = serde_json::from_str("{}").unwrap();
    assert!(c.approval.is_none());
    let r = ApprovalConfig::resolved(c.approval.as_ref());
    assert_eq!(r.channels, s(&["toa", "ask", "tmux", "gui"]));
    assert_eq!(r.timeout, Duration::from_secs(300));
    assert_eq!(r.runtime_timeout, Duration::from_secs(45));
}

#[test]
fn approval_toa_channel_validates_cleanly() {
    let mut c = with(ApprovalConfig {
        channels: Some(s(&["toa", "tmux", "gui", "ask"])),
        ..ApprovalConfig::default()
    });
    c.project_dir = Some("/proj".to_string());
    let v = validate_config(&c);
    assert!(
        v.warnings.is_empty(),
        "expected no warnings for valid channels including toa, got: {:?}",
        v.warnings
    );
    assert!(v.errors.is_empty());
}

#[test]
fn approval_empty_channels_stay_empty() {
    let c: Config = serde_json::from_str(r#"{"approval":{"channels":[]}}"#).unwrap();
    let r = ApprovalConfig::resolved(c.approval.as_ref());
    assert!(r.channels.is_empty());
    assert_eq!(r.timeout, Duration::from_secs(300));
}

#[test]
fn approval_merge_overrides_per_field() {
    let base = with(ApprovalConfig {
        channels: Some(s(&["ask", "tmux"])),
        timeout_sec: Some(30),
        runtime_timeout_sec: Some(20),
    });
    let over = with(ApprovalConfig {
        channels: Some(s(&["gui"])),
        timeout_sec: None,
        runtime_timeout_sec: Some(9),
    });
    let m = merge_configs(&base, &over).approval.unwrap();
    assert_eq!(m.channels, Some(s(&["gui"])));
    assert_eq!(m.timeout_sec, Some(30));
    assert_eq!(m.runtime_timeout_sec, Some(9));
}

#[test]
fn approval_merge_unset_stays_unset() {
    let m = merge_configs(&Config::default(), &Config::default());
    assert!(m.approval.is_none());
}

#[test]
fn approval_unknown_channel_warns() {
    let v = validate_config(&with(ApprovalConfig {
        channels: Some(s(&["ask", "carrier-pigeon"])),
        ..ApprovalConfig::default()
    }));
    assert!(
        v.warnings
            .iter()
            .any(|w| w.contains("carrier-pigeon") && w.contains("approval"))
    );
    assert!(v.errors.is_empty());
}

#[test]
fn approval_zero_timeout_warns() {
    let v = validate_config(&with(ApprovalConfig {
        timeout_sec: Some(0),
        ..ApprovalConfig::default()
    }));
    assert!(v.warnings.iter().any(|w| w.contains("timeoutSec")));
}

#[test]
fn approval_serialize_skips_unset() {
    let c = with(ApprovalConfig {
        timeout_sec: Some(7),
        ..ApprovalConfig::default()
    });
    let j = serde_json::to_value(&c).unwrap();
    assert_eq!(j, serde_json::json!({"approval": {"timeoutSec": 7}}));
}
