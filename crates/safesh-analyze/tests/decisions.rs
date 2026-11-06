mod common;
use common::*;

// D1: cwd modes.
#[test]
fn d1_non_optimistic_exec_cwd_optimistic_final() {
    let a = run("cd a && cd b\n./t");
    assert_eq!(cwd_of(&a, "./t"), Cwd::Unknown);
    assert_eq!(fin(&a).as_deref(), Some("/base/a/b"));
}

#[test]
fn d1_unfollowable_cd_loses_final_cwd() {
    assert_eq!(run("cd /a; cd $X; cd /b").final_cwd, None);
}

// D2/D3: substitutions.
#[test]
fn d2_cmdsub_in_double_quotes_and_defaults() {
    let a = run("echo \"$(one)\" ${x:-$(two)} `three`");
    assert_eq!(externals(&a), ["one", "three", "two"]);
}

#[test]
fn d2_nested_cmdsub() {
    assert_eq!(externals(&run("echo $(a $(b))")), ["a", "b"]);
}

#[test]
fn d3_process_substitution_both_kinds() {
    let a = run("tee >(gzip > out.gz) < <(gen)");
    assert_eq!(externals(&a), ["gen", "gzip", "tee"]);
    assert_eq!(redirects(&a), [w("out.gz")]);
}

// D4: here-docs.
#[test]
fn d4_unquoted_heredoc_cmdsub_walked() {
    let a = run("cat <<EOF\nhi $(whoami) `id`\nEOF");
    assert_eq!(externals(&a), ["cat", "id", "whoami"]);
}

#[test]
fn d4_quoted_heredoc_literal() {
    let a = run("cat <<'EOF'\nhi $(whoami)\nEOF");
    assert_eq!(externals(&a), ["cat"]);
}

// D5: scoping.
#[test]
fn d5_subshell_assignment_does_not_leak() {
    let a = run("(P=/bin/a); $P");
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn d5_brace_group_assignment_leaks() {
    assert_eq!(externals(&run("{ P=/bin/a; }; $P")), ["/bin/a"]);
}

#[test]
fn d5_cmdsub_assignment_does_not_leak() {
    let a = run("x=$(P=/bin/a; echo); $P");
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn d5_function_body_scope_cloned() {
    let a = run("P=/bin/a\nf() { P=/bin/b; }\n$P");
    assert_eq!(externals(&a), ["/bin/a"]);
    let b = run("P=/bin/a\nf() { P=/bin/b; }\nf\n$P");
    assert_eq!(unknown_execs(&b), 1);
}

#[test]
fn d5_env_value_used_until_shadowed() {
    assert_eq!(externals(&run_env("$T", &[("T", "/bin/e")])), ["/bin/e"]);
    assert_eq!(
        externals(&run_env("T=/bin/s; $T", &[("T", "/bin/e")])),
        ["/bin/s"]
    );
}

// D6: control plane covered in ph tests.

// D7: wrappers stay plain.
#[test]
fn d7_wrappers_are_plain_execs() {
    for s in [
        "timeout 5 ls",
        "sudo rm x",
        "watch ls",
        "parallel echo ::: a",
        "nohup ls &",
    ] {
        let a = run(s);
        assert_eq!(a.execs.len(), 1, "{s}");
        assert!(!a.execs[0].builtin);
    }
}

// D8: prefix assignment.
#[test]
fn d8_prefix_assignment_is_command_scoped() {
    let a = run("P=/bin/a env true; $P");
    assert_eq!(unknown_execs(&a), 1);
}

// D9: declaration builtins assign.
#[test]
fn d9_declaration_builtins() {
    for s in [
        "export P=/bin/a",
        "declare P=/bin/a",
        "local P=/bin/a",
        "readonly P=/bin/a",
        "typeset P=/bin/a",
    ] {
        let a = run(&format!("{s}\n$P"));
        assert!(has(&a, "/bin/a"), "{s}");
    }
}

// D10: source.
#[test]
fn d10_source_opens_and_may_change_cwd() {
    for s in ["source ./env.sh", ". ./env.sh"] {
        let a = run(s);
        assert_eq!(a.opens.len(), 1);
        assert_eq!(a.opens[0].kind, OpenKind::Source);
        assert_eq!(a.opens[0].access, Access::Read);
        assert_eq!(a.opens[0].path, Word::Known("./env.sh".into()));
        assert!(a.may_change_cwd);
        assert_eq!(a.final_cwd, None);
        assert!(a.execs[0].builtin);
    }
}

// D11: globs make words unknown.
#[test]
fn d11_glob_targets_unknown() {
    assert_eq!(unknown_opens(&run("echo x > out*.txt")), 1);
    assert_eq!(run("cd src*").final_cwd, None);
    assert_eq!(unknown_execs(&run("./bin/t* --x")), 1);
}

// D12: HOME handling.
#[test]
fn d12_cd_home() {
    assert_eq!(fin(&run("cd")).as_deref(), Some("/home/u"));
    assert_eq!(fin(&run("cd ~")).as_deref(), Some("/home/u"));
    assert_eq!(run_env("cd", &[]).final_cwd, None);
    assert_eq!(run_env("cd", &[("HOME", "rel")]).final_cwd, None);
    assert_eq!(fin(&run("HOME=/x; cd")).as_deref(), Some("/x"));
}

#[test]
fn d12_unfollowable_cd_forms() {
    for s in ["cd -", "cd ~root", "pushd /tmp", "popd"] {
        let a = run(s);
        assert_eq!(a.final_cwd, None, "{s}");
        assert!(a.may_change_cwd, "{s}");
    }
}

// Other rules.
#[test]
fn command_and_builtin_unwrap() {
    let a = run("command ls -l");
    assert_eq!(
        a.execs[0].argv,
        [Word::Known("ls".into()), Word::Known("-l".into())]
    );
    assert!(!a.execs[0].builtin);
    let b = run("builtin echo hi");
    assert_eq!(names(&b), ["echo"]);
    assert!(b.execs[0].builtin);
}

#[test]
fn command_bypasses_function() {
    let a = run("ls() { :; }\ncommand ls");
    assert!(!a.execs.last().unwrap().function);
}

#[test]
fn exec_records_target() {
    let a = run("exec ls -l");
    assert_eq!(names(&a), ["ls"]);
    assert!(!a.execs[0].builtin);
    let b = run("exec > out.log");
    assert!(b.execs.is_empty());
    assert_eq!(redirects(&b), [w("out.log")]);
}

#[test]
fn eval_alias_builtin_flag() {
    let a = run("eval 'rm -rf x'; alias ll='ls -l'");
    assert_eq!(names(&a), ["eval", "alias"]);
    assert!(a.execs.iter().all(|e| e.builtin));
    assert!(a.may_change_cwd);
    assert!(!a.functions.contains("ll"));
}

#[test]
fn builtin_list_and_normal_execs() {
    let a = run("echo a; rm b; cp c d; ls; [ -f x ]; true; test -f y");
    let builtin: Vec<&str> = a
        .execs
        .iter()
        .filter(|e| e.builtin)
        .map(|e| match &e.argv[0] {
            Word::Known(s) => s.as_str(),
            Word::Unknown(_) => "?",
        })
        .collect();
    assert_eq!(builtin, ["echo", "[", "true", "test"]);
}

#[test]
fn function_call_flag() {
    let a = run("f() { :; }\nf x");
    assert!(a.execs.last().unwrap().function);
    assert!(a.functions.contains("f"));
}

#[test]
fn device_paths_not_opens() {
    let a = run("a > /dev/null 2> /dev/stderr < /dev/stdin >/dev/tty >&2 2>&1");
    assert!(a.opens.is_empty());
}

#[test]
fn redirect_access_table() {
    let a = run("a > w; a >> x; a >| y; a &> z; a &>> z2; a <> rw; a < r");
    assert_eq!(
        redirects(&a),
        [w("w"), w("x"), w("y"), w("z"), w("z2"), w("rw"), r("r")]
    );
}

#[test]
fn compound_redirects_recorded() {
    let a = run("{ a; } > out; while read l; do b; done < in");
    assert_eq!(redirects(&a), [w("out"), r("in")]);
}

#[test]
fn relative_paths_stay_as_written() {
    let a = run("cd sub; a > ./x/../out");
    assert_eq!(redirects(&a), [w("./x/../out")]);
}

#[test]
fn arithmetic_with_cmdsub_is_unknown_exec() {
    let a = run("echo $(( $(id -u) + 1 ))");
    assert_eq!(unknown_execs(&a), 1);
    let b = run("(( x = `id -u` ))");
    assert_eq!(unknown_execs(&b), 1);
    assert_eq!(unknown_execs(&run("echo $((1+2)); ((i++))")), 0);
}

#[test]
fn aliases_not_expanded() {
    let a = run("alias ll='rm -rf x'\nll");
    assert_eq!(names(&a), ["alias", "ll"]);
}

#[test]
fn extended_test_walks_cmdsub() {
    assert!(has(&run("[[ -f $(find_it) ]] && echo ok"), "find_it"));
}

#[test]
fn if_branch_assignments_merge() {
    let a = run("if c; then P=/a; else P=/b; fi\n$P");
    assert_eq!(unknown_execs(&a), 1);
    let b = run("if c; then P=/a; fi\n$P");
    assert_eq!(externals(&b), ["/a", "c"]);
}

#[test]
fn read_invalidates_var() {
    let a = run("P=/a; read P; $P");
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn arithmetic_assignment_invalidates_var() {
    let a = run("P=/a; (( P = 3 )); $P");
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn unknown_command_name_flags_cwd() {
    let a = run("$CMD x");
    assert!(a.may_change_cwd);
    assert_eq!(a.final_cwd, None);
}

#[test]
fn deep_nesting_does_not_crash() {
    let s = format!("{}x{}", "echo $(".repeat(100), ")".repeat(100));
    let _ = analyze_big(&s);
}

fn analyze_big(s: &str) -> bool {
    let s = s.to_string();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || safesh_analyze::analyze(&s, &opts()).is_ok())
        .unwrap()
        .join()
        .unwrap()
}
