use safesh_policy::config::Config;
use safesh_policy::rules::{Rule, RuleAction, evaluate, evaluate_pipeline, validate};

fn r(pat: &[&str], action: RuleAction) -> Rule {
    Rule {
        pattern: pat.iter().map(std::string::ToString::to_string).collect(),
        action,
    }
}

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(std::string::ToString::to_string).collect()
}

use RuleAction::{Allow, Ask, Deny};

#[test]
fn design_git_push_force_glob() {
    let rules = [r(&["git", "push", "--force*"], Ask)];
    assert_eq!(evaluate(&rules, &v(&["git", "push", "--force"])), Some(Ask));
    assert_eq!(
        evaluate(&rules, &v(&["git", "push", "--force-with-lease"])),
        Some(Ask)
    );
    assert_eq!(evaluate(&rules, &v(&["git", "push", "origin"])), None);
}

#[test]
fn design_git_push_dash_f() {
    let rules = [r(&["git", "push", "-f"], Ask)];
    assert_eq!(
        evaluate(&rules, &v(&["git", "push", "-f", "origin"])),
        Some(Ask)
    );
    assert_eq!(evaluate(&rules, &v(&["git", "push", "-fx"])), None);
}

#[test]
fn design_sudo_any() {
    let rules = [r(&["sudo", "**"], Ask)];
    assert_eq!(evaluate(&rules, &v(&["sudo"])), Some(Ask));
    assert_eq!(evaluate(&rules, &v(&["sudo", "rm", "-rf", "/"])), Some(Ask));
    assert_eq!(evaluate(&rules, &v(&["sudoku"])), None);
}

#[test]
fn design_curl_pipe_sh() {
    let rules = [r(&["curl", "**", "|", "sh"], Ask)];
    let p = [v(&["curl", "-s", "http://x"]), v(&["sh"])];
    assert_eq!(evaluate_pipeline(&rules, &p), Some(Ask));
    assert_eq!(evaluate(&rules, &v(&["curl", "-s"])), None);
    let p = [v(&["curl", "http://x"]), v(&["grep", "a"])];
    assert_eq!(evaluate_pipeline(&rules, &p), None);
}

#[test]
fn prefix_allows_extra_tokens_but_not_fewer() {
    let rules = [r(&["git", "status"], Allow)];
    assert_eq!(evaluate(&rules, &v(&["git", "status", "-s"])), Some(Allow));
    assert_eq!(evaluate(&rules, &v(&["git"])), None);
}

#[test]
fn basename_on_argv0() {
    let rules = [r(&["git", "push"], Deny)];
    assert_eq!(evaluate(&rules, &v(&["/usr/bin/git", "push"])), Some(Deny));
    assert_eq!(evaluate(&rules, &v(&["git", "push"])), Some(Deny));
    // Basename applies to the first element only.
    let rules = [r(&["tool", "git"], Deny)];
    assert_eq!(evaluate(&rules, &v(&["tool", "/usr/bin/git"])), None);
}

#[test]
fn double_star_middle_and_end() {
    let rules = [r(&["a", "**", "z"], Ask)];
    assert_eq!(evaluate(&rules, &v(&["a", "z"])), Some(Ask));
    assert_eq!(evaluate(&rules, &v(&["a", "b", "c", "z", "q"])), Some(Ask));
    assert_eq!(evaluate(&rules, &v(&["a", "b", "c"])), None);
    let rules = [r(&["a", "b", "**"], Ask)];
    assert_eq!(evaluate(&rules, &v(&["a", "b"])), Some(Ask));
    assert_eq!(evaluate(&rules, &v(&["a", "b", "c", "d"])), Some(Ask));
}

#[test]
fn globs_and_escapes() {
    let rules = [r(&["ls", "-?", "f*.txt"], Allow)];
    assert_eq!(evaluate(&rules, &v(&["ls", "-l", "foo.txt"])), Some(Allow));
    assert_eq!(evaluate(&rules, &v(&["ls", "-ll", "foo.txt"])), None);
    assert_eq!(evaluate(&rules, &v(&["ls", "-l", "bar.txt"])), None);
    let rules = [r(&["echo", "a\\*b"], Deny)];
    assert_eq!(evaluate(&rules, &v(&["echo", "a*b"])), Some(Deny));
    assert_eq!(evaluate(&rules, &v(&["echo", "axb"])), None);
    let rules = [r(&["echo", "a\\?"], Deny)];
    assert_eq!(evaluate(&rules, &v(&["echo", "a?"])), Some(Deny));
    assert_eq!(evaluate(&rules, &v(&["echo", "ab"])), None);
}

#[test]
fn strongest_action_wins() {
    let rules = [
        r(&["git", "**"], Allow),
        r(&["git", "push", "**"], Ask),
        r(&["git", "push", "-f"], Deny),
    ];
    assert_eq!(evaluate(&rules, &v(&["git", "log"])), Some(Allow));
    assert_eq!(evaluate(&rules, &v(&["git", "push", "x"])), Some(Ask));
    assert_eq!(evaluate(&rules, &v(&["git", "push", "-f"])), Some(Deny));
    let rev = [rules[2].clone(), rules[1].clone(), rules[0].clone()];
    assert_eq!(evaluate(&rev, &v(&["git", "push", "-f"])), Some(Deny));
}

#[test]
fn evaluate_ignores_pipeline_rules_and_vice_versa() {
    let rules = [r(&["curl", "|", "sh"], Deny), r(&["curl"], Allow)];
    assert_eq!(evaluate(&rules, &v(&["curl", "x"])), Some(Allow));
    let p = [v(&["curl"]), v(&["sh"])];
    assert_eq!(evaluate_pipeline(&rules, &p), Some(Deny));
    assert_eq!(evaluate_pipeline(&rules[1..], &p), None);
}

#[test]
fn pipeline_matches_in_middle_not_across_gaps() {
    let rules = [r(&["curl", "**", "|", "sh"], Ask)];
    let mid = [
        v(&["echo", "x"]),
        v(&["curl", "u"]),
        v(&["sh", "-s"]),
        v(&["tee", "f"]),
    ];
    assert_eq!(evaluate_pipeline(&rules, &mid), Some(Ask));
    let gap = [v(&["curl", "u"]), v(&["cat"]), v(&["sh"])];
    assert_eq!(evaluate_pipeline(&rules, &gap), None);
    let single = [v(&["curl", "u"])];
    assert_eq!(evaluate_pipeline(&rules, &single), None);
}

#[test]
fn pipeline_three_stages_and_strongest() {
    let rules = [
        r(&["a", "|", "b", "|", "c"], Ask),
        r(&["b", "|", "c"], Deny),
    ];
    let p = [v(&["a"]), v(&["b"]), v(&["c"])];
    assert_eq!(evaluate_pipeline(&rules, &p), Some(Deny));
    assert_eq!(evaluate_pipeline(&rules[..1], &p), Some(Ask));
}

#[test]
fn validate_errors() {
    assert!(validate(&[r(&["git"], Ask), r(&["a", "|", "b"], Ask)]).is_empty());
    assert_eq!(validate(&[r(&[], Ask)]).len(), 1);
    assert_eq!(validate(&[r(&["|", "sh"], Ask)]).len(), 1);
    assert_eq!(validate(&[r(&["**", "x"], Ask)]).len(), 1);
    assert_eq!(validate(&[r(&["a", "|"], Ask)]).len(), 1);
    assert_eq!(validate(&[r(&["a", "|", "|", "b"], Ask)]).len(), 1);
    let errs = validate(&[r(&["ok"], Ask), r(&[], Deny)]);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("rules[1]"));
}

#[test]
fn config_json_with_rules() {
    let json = r#"{"rules":[{"match":["git","push","-f"],"action":"deny"},
        {"match":["sudo","**"],"action":"ask"}]}"#;
    let cfg: Config = serde_json::from_str(json).unwrap();
    let rules = cfg.rules.unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].action, Deny);
    assert_eq!(rules[1].pattern, v(&["sudo", "**"]));
    let out = serde_json::to_value(&rules[0]).unwrap();
    assert_eq!(out["match"][2], "-f");
    assert_eq!(out["action"], "deny");
}

#[test]
fn literal_evaluation_has_no_basename_rule() {
    use safesh_policy::rules::evaluate_literal;
    let rules = vec![
        r(&["git", "status"], Allow),
        r(&["/opt/tools/deploy", "**"], Allow),
    ];
    assert_eq!(evaluate(&rules, &v(&["/tmp/x/git", "status"])), Some(Allow));
    assert_eq!(
        evaluate_literal(&rules, &v(&["/tmp/x/git", "status"])),
        None
    );
    assert_eq!(
        evaluate_literal(&rules, &v(&["git", "status"])),
        Some(Allow)
    );
    assert_eq!(
        evaluate_literal(&rules, &v(&["/opt/tools/deploy", "x"])),
        Some(Allow)
    );
}
