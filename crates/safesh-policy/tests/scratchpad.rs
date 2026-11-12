//! The Unix layout, `/tmp/claude-<uid>`. The Windows layout has unit tests in `scratchpad`.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use safesh_policy::scratchpad::*;

const SID: &str = "0b9d7a52-6f1e-4c3a-9d2b-1a2b3c4d5e6f";

fn pad() -> String {
    format!("/private/tmp/claude-502/-Users-me-proj/{SID}/scratchpad")
}

#[test]
fn sp_1_under_pad_both_spellings() {
    assert!(is_under_session_scratchpad(
        &format!("{}/run.sh", pad()),
        Some(SID),
        Some(502)
    ));
    assert!(is_under_session_scratchpad(
        &format!("/tmp/claude-502/-Users-me-proj/{SID}/scratchpad/a/run.sh"),
        Some(SID),
        Some(502)
    ));
}
#[test]
fn sp_2_other_session() {
    assert!(!is_under_session_scratchpad(
        &format!("{}/run.sh", pad()),
        Some("other-session"),
        Some(502)
    ));
}
#[test]
fn sp_3_uid_mismatch() {
    assert!(!is_under_session_scratchpad(
        &format!("{}/run.sh", pad()),
        Some(SID),
        Some(501)
    ));
}
#[test]
fn sp_4_traversal() {
    assert!(!is_under_session_scratchpad(
        &format!("{}/../../x/run.sh", pad()),
        Some(SID),
        Some(502)
    ));
}
#[test]
fn sp_5_dir_itself() {
    assert!(!is_under_session_scratchpad(
        &format!("{}/", pad()),
        Some(SID),
        Some(502)
    ));
}
#[test]
fn sp_6_relative() {
    assert!(!is_under_session_scratchpad(
        "scratchpad/run.sh",
        Some(SID),
        Some(502)
    ));
}
#[test]
fn sp_7_missing_sid_or_uid() {
    let p = format!("{}/run.sh", pad());
    assert!(!is_under_session_scratchpad(&p, None, Some(502)));
    assert!(!is_under_session_scratchpad(&p, Some(SID), None));
}
#[test]
fn sp_8_regex_syntax_in_sid() {
    assert!(!is_under_session_scratchpad(
        &format!("{}/run.sh", pad()),
        Some(".*"),
        Some(502)
    ));
}
#[test]
fn sp_9_slash_in_sid() {
    assert!(!is_under_session_scratchpad(
        "/tmp/claude-502/p/a/b/scratchpad/x",
        Some("a/b"),
        Some(502)
    ));
}

struct Fixture {
    root: tempfile::TempDir,
    _proj: PathBuf,
    pad: PathBuf,
    outside: tempfile::TempDir,
    sid: String,
    uid: u32,
}

fn fixture(tag: &str) -> Fixture {
    let uid = rustix::process::getuid().as_raw();
    let root = tempfile::TempDir::new().unwrap();
    let sid = format!("safesh-test-{tag}-{}", std::process::id());
    let proj = root.path().join(&sid);
    let pad = proj.join(&sid).join("scratchpad");
    fs::create_dir_all(&pad).unwrap();
    fs::write(pad.join("run.sh"), "echo").unwrap();
    Fixture {
        root,
        _proj: proj,
        pad,
        outside: tempfile::TempDir::new().unwrap(),
        sid,
        uid,
    }
}

#[test]
fn sp_10_existing_script() {
    let f = fixture("sp10");
    let out_file = f.outside.path().join("out.sh");
    fs::write(&out_file, "x").unwrap();
    symlink(&out_file, f.pad.join("link.sh")).unwrap();
    let p = |n: &str| f.pad.join(n).to_string_lossy().into_owned();
    let (sid, uid, root) = (Some(f.sid.as_str()), Some(f.uid), f.root.path());
    assert!(is_session_scratchpad_script_in(&p("run.sh"), sid, uid, root));
    assert!(!is_session_scratchpad_script_in(&p("missing.sh"), sid, uid, root));
    assert!(!is_session_scratchpad_script_in(&p("link.sh"), sid, uid, root));
    assert!(!is_session_scratchpad_script_in(
        &p("run.sh"),
        Some("other-session"),
        uid,
        root
    ));
}

#[test]
fn sp_11_new_script() {
    let f = fixture("sp11");
    symlink(f.outside.path(), f.pad.join("out")).unwrap();
    let p = |n: &str| f.pad.join(n).to_string_lossy().into_owned();
    let (sid, uid, root) = (Some(f.sid.as_str()), Some(f.uid), f.root.path());
    assert!(is_new_session_scratchpad_script_in(&p("new.sh"), sid, uid, root));
    assert!(is_new_session_scratchpad_script_in(
        &p("sub/dir/new.sh"),
        sid,
        uid,
        root
    ));
    assert!(!is_new_session_scratchpad_script_in(&p("run.sh"), sid, uid, root));
    assert!(!is_new_session_scratchpad_script_in(
        &p("out/new.sh"),
        sid,
        uid,
        root
    ));
    assert!(!is_new_session_scratchpad_script_in(
        &p("new.sh"),
        Some("other-session"),
        uid,
        root
    ));
}
