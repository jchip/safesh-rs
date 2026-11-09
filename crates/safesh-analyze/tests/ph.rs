mod common;
use common::*;
use safesh_analyze::is_control_plane;

#[test]
fn ph_1_leading_assignments() {
    assert!(is_control_plane(
        "TMPDIR=/tmp safesh retry-path --id=abc --choice=w2d"
    ));
    assert!(is_control_plane("FOO=bar BAZ=qux desh retry --id=x"));
}

#[test]
fn ph_2_run_and_retry() {
    assert!(is_control_plane("safesh retry --id=x"));
    assert!(is_control_plane("safesh run --id abc_123"));
    assert!(is_control_plane("safesh run --id abc_123 && cd '/tmp/x'"));
    assert!(is_control_plane("desh retry-path --id=x"));
    assert!(!is_control_plane("/bin/zsh -lc 'desh -q --id abc'"));
}

#[test]
fn ph_3_other_wrappers_are_not_control_plane() {
    assert!(!is_control_plane("deno test"));
    assert!(!is_control_plane("desh run git status"));
    assert!(!is_control_plane("safesh run git status"));
    assert!(!is_control_plane("desh run --id abc"));
}

#[test]
fn ph_5_structural_shapes() {
    // CHANGED: passthrough skips all checks, so other commands in the script disqualify it.
    // Such scripts go through the normal pre-check, where `safesh` is an allowed command.
    assert!(!is_control_plane(
        "cd /Users/jc/dev/fyn-prod-env && safesh retry --id=x --choice=1 2>&1 | tail -20"
    ));
    assert!(is_control_plane(
        "cd /tmp && safesh retry-path --id=x --choice=2"
    ));
    assert!(is_control_plane("safesh retry --id=x --choice=1 2>&1"));
    assert!(is_control_plane("(safesh retry --id=x)"));
    assert!(is_control_plane("{ safesh retry --id=x; }"));
    assert!(!is_control_plane("if true; then safesh retry --id=x; fi"));
}

#[test]
fn control_plane_cannot_carry_other_work() {
    for s in [
        "safesh retry --id=x; rm -rf ~",
        "safesh retry --id=x && rm -rf ~",
        "rm -rf ~ || safesh retry --id=x",
        "TMPDIR=$(rm -rf ~) safesh retry --id=x",
        "safesh retry --id=$(rm -rf ~)",
        "safesh retry --id=x > /etc/passwd",
        "safesh retry --id=x | sh",
        "cd $(rm -rf ~) && safesh retry --id=x",
        "(safesh retry --id=x) > out",
        "cd /tmp",
        "",
    ] {
        assert!(!is_control_plane(s), "{s}");
    }
}

#[test]
fn ph_6_not_control_plane() {
    assert!(!is_control_plane("ls -la"));
    assert!(!is_control_plane("TMPDIR=/tmp ls"));
    assert!(!is_control_plane("FOO=safesh"));
    assert!(!is_control_plane("echo safesh retry"));
    assert!(!is_control_plane("echo 'unterminated"));
}

#[test]
fn ph_8_function_not_external() {
    let a = run("norm() { sed \"s/'/\\\"/g\" \"$1\"; }\nnorm a.txt > b.txt\nnorm c.txt");
    assert_eq!(externals(&a), ["sed"]);
    assert_eq!(a.execs.iter().filter(|e| e.function).count(), 2);
}

#[test]
fn ph_9_function_table_global() {
    let a = run("if true; then walk() { find .; }; fi\nwalk");
    assert_eq!(externals(&a), ["find"]);
}

#[test]
fn ph_10_recursive_function() {
    let a = run("walk() { walk; awk '{print}'; }\nwalk");
    assert_eq!(externals(&a), ["awk"]);
}

#[test]
fn ph_11_call_before_definition() {
    let a = run("norm a.txt\nnorm() { sed -e x \"$1\"; }");
    assert_eq!(externals(&a), ["norm", "sed"]);
}

#[test]
fn ph_12_assignment_across_cd() {
    let a = run("cd /tmp && P=/usr/bin/python3; $P -m pytest -q");
    assert_eq!(externals(&a), ["/usr/bin/python3"]);
    assert_eq!(names(&a)[0], "cd");
}

#[test]
fn ph_13_assignment_in_chain() {
    assert_eq!(externals(&run("P=/usr/bin/true && $P")), ["/usr/bin/true"]);
    assert_eq!(
        externals(&run("true; P=/usr/bin/true; $P")),
        ["/usr/bin/true"]
    );
}

#[test]
fn ph_14_command_v_query() {
    let a = run("command -v rg || P=/usr/bin/grep; $P x");
    assert_eq!(externals(&a), ["/usr/bin/grep"]);
    assert!(a.execs[0].builtin);
}

#[test]
fn ph_15_16_conditional_reassign() {
    for s in [
        "P=/usr/bin/a; true && P=/usr/bin/b; $P",
        "P=/usr/bin/a; false || P=/usr/bin/b; $P",
    ] {
        let a = run(s);
        assert_eq!(unknown_execs(&a), 1, "{s}");
        assert!(!names(&a).iter().any(|n| n.contains("$P")));
    }
}

#[test]
fn ph_17_pipe_assignment_does_not_leak() {
    let a = run("P=/usr/bin/true | cat; $P");
    assert_eq!(externals(&a), ["cat"]);
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn ph_18_unknown_shadows_env() {
    let a = run_env(
        "SSH746_TOOL=$(pick); $SSH746_TOOL",
        &[("SSH746_TOOL", "/usr/bin/true")],
    );
    assert_eq!(externals(&a), ["pick"]);
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn ph_19_may_change_cwd() {
    let no = [
        "echo hi | sort",
        "export A=1; X=2; for f in a b; do echo $f; done",
        "command -v git && builtin echo hi",
    ];
    let yes = [
        "cd sub && ls",
        "if true; then pushd /tmp; fi",
        "f() { cd /tmp; }; f",
        "for d in a b; do popd; done",
        "source ./env.sh",
        "eval \"$CMD\"",
        "$CMD /tmp",
        "command cd /tmp && ls",
        "builtin pushd /tmp",
    ];
    for s in no {
        assert!(!run(s).may_change_cwd, "{s}");
    }
    for s in yes {
        assert!(run(s).may_change_cwd, "{s}");
    }
}

#[test]
fn ph_20_21_23_var_expansion() {
    assert!(has(
        &run("TOOL=./bin/tool\n\"$TOOL\" --version"),
        "./bin/tool"
    ));
    assert!(has(
        &run("cd /proj && TOOL=/proj/bin/tool; $TOOL --version"),
        "/proj/bin/tool"
    ));
    assert!(has(
        &run_env("TOOL=~/bin/tool\n\"$TOOL\" --version", &[("HOME", "/proj")]),
        "/proj/bin/tool"
    ));
}

#[test]
fn ph_22_unresolved_tool() {
    let a = run("TOOL=$(pick-tool); $TOOL --version");
    assert_eq!(externals(&a), ["pick-tool"]);
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn ph_24_25_pipeline_commands() {
    assert_eq!(
        externals(&run("grep -n foo data.txt | sort | head -2")),
        ["grep", "head", "sort"]
    );
    assert_eq!(
        externals(&run("echo $(sort data.txt | head -1)")),
        ["head", "sort"]
    );
}

#[test]
fn ph_26_redirect() {
    assert_eq!(
        redirects(&run("sort data.txt > sorted.txt 2>/dev/null")),
        [w("sorted.txt")]
    );
}

#[test]
fn ph_27_xargs() {
    assert_eq!(externals(&run("echo hi | xargs wc -l")), ["xargs"]);
}

#[test]
fn ph_28_nested_cmdsub() {
    assert!(has(
        &run("echo $(definitely-not-allowed-cmd-xyz)"),
        "definitely-not-allowed-cmd-xyz"
    ));
}

#[test]
fn ph_29_absolute_redirect() {
    assert_eq!(
        redirects(&run("sort data.txt > /etc/ssh576-test-output.txt")),
        [w("/etc/ssh576-test-output.txt")]
    );
}

#[test]
fn ph_30_plain_command() {
    assert_eq!(
        externals(&run("definitely-not-allowed-cmd-xyz --help")),
        ["definitely-not-allowed-cmd-xyz"]
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn ph_35_final_cwd() {
    assert_eq!(
        fin(&run("cd sub && echo ===")).as_deref(),
        Some("/base/sub")
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn ph_36_var_through_chain() {
    let a = run("S=/base/s; mkdir -p $S/pm && cd $S/pm && echo x > f.txt; ls *.txt");
    assert_eq!(fin(&a).as_deref(), Some("/base/s/pm"));
    assert_eq!(a.opens[0].cwd, known("/base/s/pm"));
    assert_eq!(cwd_of(&a, "ls"), Cwd::Unknown);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn ph_37_exit_chain() {
    assert_eq!(
        fin(&run("cd sub && echo === && exit 7")).as_deref(),
        Some("/base/sub")
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn ph_38_back_to_base() {
    assert_eq!(
        fin(&run("cd sub && cd .. && echo ===")).as_deref(),
        Some("/base")
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn ph_39_no_cd() {
    let a = run("echo ===");
    assert_eq!(fin(&a).as_deref(), Some("/base"));
    assert!(!a.may_change_cwd);
}

#[test]
fn ph_40_unresolved_cd_target() {
    assert_eq!(run("cd \"$(echo sub)\" && echo ===").final_cwd, None);
}

#[test]
fn ph_41_function_changes_cwd() {
    assert_eq!(run("f() { cd sub; }; f; echo ===").final_cwd, None);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn ph_45_exit_only() {
    assert_eq!(fin(&run("echo === && exit 7")).as_deref(), Some("/base"));
}

#[test]
fn parse_error_is_error() {
    assert!(safesh_analyze::analyze("echo (", &opts()).is_err());
    assert!(safesh_analyze::analyze("echo 'x", &opts()).is_err());
}

#[test]
fn run_with_pending_is_not_control_plane() {
    assert!(is_control_plane("safesh run --id x"));
    assert!(!is_control_plane("safesh run --id x --pending p"));
    assert!(!is_control_plane("safesh run --id=x --pending=p"));
    assert!(!is_control_plane("/abs/path/safesh run --id x --pending p"));
}
