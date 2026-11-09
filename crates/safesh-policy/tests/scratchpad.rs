//! The Unix layout, `/tmp/claude-<uid>`. The Windows layout has unit tests in `scratchpad`.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{DirBuilderExt, symlink};
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

/// Real `/tmp/claude-<uid>` is required: the trust rule hardcodes that root.
struct Fixture {
    proj: PathBuf,
    pad: PathBuf,
    outside: tempfile::TempDir,
    sid: String,
    uid: u32,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.proj);
    }
}

fn fixture(tag: &str) -> Fixture {
    let uid = rustix::process::getuid().as_raw();
    let root = PathBuf::from(format!("/tmp/claude-{uid}"));
    // Tests run in parallel, so another test may create it between check and create.
    match fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => panic!("{}: {e}", root.display()),
    }
    let sid = format!("safesh-test-{tag}-{}", std::process::id());
    let proj = root.join(&sid);
    let pad = proj.join(&sid).join("scratchpad");
    fs::create_dir_all(&pad).unwrap();
    fs::write(pad.join("run.sh"), "echo").unwrap();
    Fixture {
        proj,
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
    let (sid, uid) = (Some(f.sid.as_str()), Some(f.uid));
    assert!(is_session_scratchpad_script(&p("run.sh"), sid, uid));
    assert!(!is_session_scratchpad_script(&p("missing.sh"), sid, uid));
    assert!(!is_session_scratchpad_script(&p("link.sh"), sid, uid));
    assert!(!is_session_scratchpad_script(
        &p("run.sh"),
        Some("other-session"),
        uid
    ));
}

#[test]
fn sp_11_new_script() {
    let f = fixture("sp11");
    symlink(f.outside.path(), f.pad.join("out")).unwrap();
    let p = |n: &str| f.pad.join(n).to_string_lossy().into_owned();
    let (sid, uid) = (Some(f.sid.as_str()), Some(f.uid));
    assert!(is_new_session_scratchpad_script(&p("new.sh"), sid, uid));
    assert!(is_new_session_scratchpad_script(
        &p("sub/dir/new.sh"),
        sid,
        uid
    ));
    assert!(!is_new_session_scratchpad_script(&p("run.sh"), sid, uid));
    assert!(!is_new_session_scratchpad_script(
        &p("out/new.sh"),
        sid,
        uid
    ));
    assert!(!is_new_session_scratchpad_script(
        &p("new.sh"),
        Some("other-session"),
        uid
    ));
}
