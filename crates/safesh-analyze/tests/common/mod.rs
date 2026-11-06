#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

pub use safesh_analyze::{Access, Analysis, AnalyzeOptions, Cwd, Exec, OpenKind, Word, analyze};

pub fn opts() -> AnalyzeOptions {
    AnalyzeOptions {
        base_cwd: PathBuf::from("/base"),
        env: BTreeMap::from([("HOME".to_string(), "/home/u".to_string())]),
    }
}

pub fn run(script: &str) -> Analysis {
    analyze(script, &opts()).expect("parse")
}

pub fn run_env(script: &str, env: &[(&str, &str)]) -> Analysis {
    let mut o = opts();
    o.env = env
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    analyze(script, &o).expect("parse")
}

fn name(e: &Exec) -> Option<String> {
    match &e.argv[0] {
        Word::Known(s) => Some(s.clone()),
        Word::Unknown(_) => None,
    }
}

/// Known argv[0] of every exec, in order.
pub fn names(a: &Analysis) -> Vec<String> {
    a.execs.iter().filter_map(name).collect()
}

/// Known argv[0] of non-builtin, non-function execs, sorted and unique.
pub fn externals(a: &Analysis) -> Vec<String> {
    let mut v: Vec<String> = a
        .execs
        .iter()
        .filter(|e| !e.builtin && !e.function)
        .filter_map(name)
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn has(a: &Analysis, cmd: &str) -> bool {
    names(a).iter().any(|n| n == cmd)
}

pub fn unknown_execs(a: &Analysis) -> usize {
    a.execs
        .iter()
        .filter(|e| matches!(e.argv[0], Word::Unknown(_)))
        .count()
}

/// Redirect opens with a known path, as (path, access).
pub fn redirects(a: &Analysis) -> Vec<(String, Access)> {
    a.opens
        .iter()
        .filter(|o| o.kind == OpenKind::Redirect)
        .filter_map(|o| match &o.path {
            Word::Known(p) => Some((p.clone(), o.access)),
            Word::Unknown(_) => None,
        })
        .collect()
}

pub fn unknown_opens(a: &Analysis) -> usize {
    a.opens
        .iter()
        .filter(|o| matches!(o.path, Word::Unknown(_)))
        .count()
}

pub fn w(p: &str) -> (String, Access) {
    (p.to_string(), Access::Write)
}

pub fn r(p: &str) -> (String, Access) {
    (p.to_string(), Access::Read)
}

pub fn known(p: &str) -> Cwd {
    Cwd::Known(PathBuf::from(p))
}

/// Cwd of the first exec whose argv[0] is `cmd`.
pub fn cwd_of(a: &Analysis, cmd: &str) -> Cwd {
    a.execs
        .iter()
        .find(|e| name(e).as_deref() == Some(cmd))
        .unwrap_or_else(|| panic!("no exec {cmd}"))
        .cwd
        .clone()
}

/// Exec cwds of every exec of `cmd`.
pub fn cwds_of(a: &Analysis, cmd: &str) -> Vec<Cwd> {
    a.execs
        .iter()
        .filter(|e| name(e).as_deref() == Some(cmd))
        .map(|e| e.cwd.clone())
        .collect()
}

pub fn fin(a: &Analysis) -> Option<String> {
    a.final_cwd
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
}
