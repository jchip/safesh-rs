mod common;
use common::*;

fn has_all(a: &Analysis, cmds: &[&str]) -> bool {
    cmds.iter().all(|c| has(a, c))
}

#[test]
fn pa_1_pipeline_commands() {
    let a = run("git log --oneline | grep foo | head -3");
    assert_eq!(externals(&a), ["git", "grep", "head"]);
}

#[test]
fn pa_2_cmdsub_commands() {
    assert!(has_all(
        &run("echo $(git rev-parse HEAD)"),
        &["git", "echo"]
    ));
}

#[test]
fn pa_3_assignment_cmdsub() {
    assert!(has_all(
        &run("FOO=$(curl example.com) bar --x"),
        &["curl", "bar"]
    ));
}

#[test]
fn pa_4_for_cmdsub() {
    assert!(has_all(
        &run("for f in $(ls); do wc -l \"$f\"; done"),
        &["ls", "wc"]
    ));
}

#[test]
fn pa_5_variable_command_name() {
    assert!(has(
        &run("TOOL=./bin/tool\n\"$TOOL\" --version"),
        "./bin/tool"
    ));
}

#[test]
fn pa_6_arith_and_lists() {
    let a = run("test -f x && echo $((1+2)) || false");
    assert!(has_all(&a, &["test", "echo", "false"]));
    assert_eq!(unknown_execs(&a), 0);
}

#[test]
fn pa_7_eval() {
    assert_eq!(names(&run("eval \"ls\"")), ["eval"]);
}

#[test]
fn pa_8_xargs() {
    let a = run("echo hi | xargs somecmd");
    assert_eq!(names(&a), ["echo", "xargs"]);
    assert_eq!(a.execs[1].argv[1], Word::Known("somecmd".into()));
}

#[test]
fn pa_9_bash_c() {
    assert_eq!(names(&run("bash -c 'ls'")), ["bash"]);
}

#[test]
fn pa_10_env_wrapper_is_plain_exec() {
    let a = run("env FOO=1 somecmd");
    assert_eq!(names(&a), ["env"]);
    assert_eq!(a.execs[0].argv.len(), 3);
}

#[test]
fn pa_11_find() {
    assert_eq!(names(&run("find . -name '*.ts'")), ["find"]);
}

#[test]
fn pa_12_rm() {
    let a = run("rm foo.txt");
    assert_eq!(names(&a), ["rm"]);
    assert!(!a.execs[0].builtin);
}

#[test]
fn pa_13_mv() {
    assert_eq!(names(&run("mv a b")), ["mv"]);
}

#[test]
fn pa_14_heredoc() {
    let a = run("cat <<EOF\nhello\nEOF");
    assert_eq!(names(&a), ["cat"]);
    assert!(a.opens.is_empty());
}

#[test]
fn pa_15_herestring() {
    let a = run("grep x <<< 'data'");
    assert_eq!(names(&a), ["grep"]);
    assert!(a.opens.is_empty());
}

#[test]
fn pa_16_process_substitution() {
    let a = run("diff <(sort a) <(sort b)");
    assert_eq!(externals(&a), ["diff", "sort"]);
}

#[test]
fn pa_17_function_body_walked() {
    let a = run("f() { ls; }");
    assert_eq!(names(&a), ["ls"]);
    assert!(a.functions.contains("f"));
}

#[test]
fn pa_18_background() {
    assert_eq!(names(&run("sleep 5 &")), ["sleep"]);
}

#[test]
fn pa_19_unresolved_command_name() {
    let a = run("\"$UNDEFINED_CMD_XYZ\" --version");
    assert!(names(&a).is_empty());
    assert_eq!(unknown_execs(&a), 1);
}

#[test]
fn pa_20_dd() {
    assert_eq!(names(&run("dd if=/dev/zero of=x")), ["dd"]);
}

#[test]
fn pa_21_nested_dd() {
    assert!(has_all(
        &run("echo $(dd if=/dev/zero of=x)"),
        &["echo", "dd"]
    ));
}

#[test]
fn pa_30_redirect_modes() {
    let a = run("sort data.txt > out.txt 2>/dev/null < in.txt");
    assert_eq!(redirects(&a), [w("out.txt"), r("in.txt")]);
}

#[test]
fn pa_31_fd_dup_and_null() {
    assert!(run("ls 2>&1 >/dev/null").opens.is_empty());
}

#[test]
fn pa_32_tilde_target() {
    assert_eq!(
        redirects(&run("echo hi > ~/out.txt")),
        [w("/home/u/out.txt")]
    );
}

#[test]
fn pa_33_unknown_target() {
    let a = run("echo hi > $UNKNOWN_TARGET_XYZ");
    assert_eq!(unknown_opens(&a), 1);
    assert_eq!(a.opens[0].access, Access::Write);
}

#[test]
fn pa_34_cd_is_not_an_open() {
    let a = run("cd src && cat mod.ts");
    assert!(a.opens.is_empty());
    assert_eq!(names(&a), ["cd", "cat"]);
    assert!(a.execs[0].builtin);
}

#[test]
fn pa_35_cd_unresolved() {
    let a = run("cd \"$SOME_UNSET_DIR_XYZ\"");
    assert_eq!(a.final_cwd, None);
    assert!(a.may_change_cwd);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pa_36_cd_then_relative_redirect() {
    let a = run("cd /home/u && echo x > .ssh/authorized_keys");
    assert_eq!(redirects(&a), [w(".ssh/authorized_keys")]);
    assert_eq!(a.opens[0].cwd, known("/home/u"));
}

#[test]
fn pa_37_cd_relative() {
    let a = run("cd sub && echo x > out.txt");
    assert_eq!(redirects(&a), [w("out.txt")]);
    assert_eq!(a.opens[0].cwd, known("/base/sub"));
}

#[test]
fn pa_38_cd_chain() {
    let a = run("cd a/b && cd ../c && echo x > y");
    assert_eq!(a.opens[0].cwd, known("/base/a/c"));
}

#[test]
fn pa_39_or_chain_cd_unknown() {
    let a = run("cd /tmp || cd $D\necho x > y");
    assert_eq!(a.opens[0].cwd, Cwd::Unknown);
}

#[test]
fn pa_40_if_cd_unknown_after() {
    let a = run("if true; then cd /etc; fi\necho x > passwd");
    assert_eq!(a.opens[0].cwd, Cwd::Unknown);
}

#[test]
fn pa_41_subshell_isolated() {
    let a = run("(cd /etc); echo x > local.txt");
    assert_eq!(a.opens[0].cwd, known("/base"));
}

#[test]
fn pa_42_pipe_segment_isolated() {
    let a = run("cd /etc | cat\necho x > local.txt");
    assert_eq!(a.opens[0].cwd, known("/base"));
}

#[test]
fn pa_43_loop_relative_cd() {
    let a = run("for i in 1 2; do cd sub; echo x > out; done");
    assert_eq!(a.opens[0].cwd, Cwd::Unknown);
    let b = run("for i in 1 2; do cd sub; done\necho x > out");
    assert_eq!(b.opens[0].cwd, Cwd::Unknown);
}

#[test]
fn pa_44_cd_dash() {
    let a = run("cd -\necho x > y");
    assert_eq!(a.opens[0].cwd, Cwd::Unknown);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pa_46_and_chain_not_optimistic() {
    let a = run("cd /outside && cd /project\necho secret > x");
    assert_eq!(a.opens[0].cwd, Cwd::Unknown);
    assert_eq!(fin(&a).as_deref(), Some("/project"));
}

#[test]
fn pa_47_or_chain() {
    let a = run("cd missing || cd /tmp\necho x > y");
    assert_eq!(a.opens[0].cwd, Cwd::Unknown);
    assert_eq!(a.final_cwd, None);
}

#[test]
fn pa_48_if_cd_condition() {
    let a = run("if cd sub; then echo x > out; fi");
    assert_eq!(redirects(&a), [w("out")]);
    assert_eq!(a.opens[0].cwd, known("/base/sub"));
}

#[test]
fn pa_49_case_pattern_cmdsub() {
    assert!(has(
        &run("case $1 in $(evilcmd)) echo hi;; esac"),
        "evilcmd"
    ));
}

#[test]
fn pa_50_case_pattern_eval() {
    assert!(has(
        &run("case $1 in $(eval payload)) echo hi;; esac"),
        "eval"
    ));
}

#[test]
fn pa_51_loop_var_shadows_assignment() {
    let a = run("d=.\nfor d in /etc /root; do cd \"$d\"; done");
    assert!(a.opens.is_empty());
    assert_eq!(a.final_cwd, None);
}

#[test]
fn pa_52_body_assigned_var_invalidated() {
    let a = run("p=src\nfor i in 1 2; do cd \"$p\"; p=/etc; done");
    let cd = a
        .execs
        .iter()
        .find(|e| e.builtin && e.argv[0] == Word::Known("cd".into()))
        .unwrap();
    assert!(matches!(cd.argv[1], Word::Unknown(_)));
}

#[test]
fn pa_53_loop_var_unknown_after() {
    let a = run("x=safe\nfor x in a b; do echo \"$x\"; done\ncd \"$x\"");
    assert_eq!(a.final_cwd, None);
    let cd = a
        .execs
        .iter()
        .rev()
        .find(|e| e.argv[0] == Word::Known("cd".into()))
        .unwrap();
    assert!(matches!(cd.argv[1], Word::Unknown(_)));
}

#[test]
fn pa_54_cd_user_home() {
    let a = run("cd ~root/x");
    assert_eq!(a.final_cwd, None);
}

#[test]
fn pa_55_redirect_user_home() {
    assert_eq!(unknown_opens(&run("echo x > ~root/y")), 1);
}

#[test]
fn pa_57_read_write() {
    let a = run("grep x <> both.txt");
    assert_eq!(redirects(&a), [w("both.txt")]);
}

#[test]
fn pa_58_dup_to_file_write() {
    assert_eq!(redirects(&run("echo hi >& out.log")), [w("out.log")]);
}

#[test]
fn pa_59_dup_input_file_read() {
    assert_eq!(redirects(&run("cat <& in.txt")), [r("in.txt")]);
}

#[test]
fn pa_60_amp_redirects() {
    let a = run("echo hi &> all.log\necho hi &>> all.log");
    assert_eq!(redirects(&a), [w("all.log"), w("all.log")]);
}

#[test]
fn pa_61_clobber() {
    assert_eq!(redirects(&run("echo hi >| out.txt")), [w("out.txt")]);
}

#[test]
fn pa_62_fd_close_and_move() {
    assert!(run("echo hi >&-\necho hi 2>&1-").opens.is_empty());
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pa_63_exec_cwd_after_cd() {
    let a = run("cd /ws/pkg && ./gradlew build");
    assert_eq!(cwd_of(&a, "./gradlew"), known("/ws/pkg"));
}

#[test]
fn pa_64_exec_cwd_relative() {
    assert_eq!(
        cwd_of(&run("cd sub && ./tool"), "./tool"),
        known("/base/sub")
    );
}

#[test]
fn pa_65_exec_cwd_base() {
    assert_eq!(cwd_of(&run("./tool --x"), "./tool"), known("/base"));
}

#[test]
fn pa_66_exec_cwd_chain() {
    assert_eq!(
        cwd_of(&run("cd a/b && cd ../c && ./tool"), "./tool"),
        known("/base/a/c")
    );
}

#[test]
fn pa_67_68_names_unaffected_by_cwd() {
    assert!(has(&run("cd /ws && git status"), "git"));
    assert!(has(&run("cd /ws && /usr/bin/tool"), "/usr/bin/tool"));
}

#[test]
fn pa_69_subshell_cwd_restored() {
    assert_eq!(cwd_of(&run("(cd /etc); ./tool"), "./tool"), known("/base"));
}

#[test]
fn pa_70_or_chain_exec_cwd_unknown() {
    assert_eq!(
        cwd_of(&run("cd /a || cd /b\n./tool"), "./tool"),
        Cwd::Unknown
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pa_71_cwd_per_call() {
    let a = run("./tool; cd /ws/pkg && ./tool");
    assert_eq!(cwds_of(&a, "./tool"), [known("/base"), known("/ws/pkg")]);
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Unix path literals: on Windows /x is a Git Bash path"
)]
fn pa_pipeline_stage_vars_isolated_but_final_cwd() {
    let a = run("cd sub && make");
    assert_eq!(fin(&a).as_deref(), Some("/base/sub"));
}
