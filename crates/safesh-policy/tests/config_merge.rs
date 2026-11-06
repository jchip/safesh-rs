use safesh_policy::config::{Config, EnvConfig, NetPermission, PermissionsConfig, merge_configs};

fn env(allow: &[&str], read_all: Option<bool>) -> Config {
    Config {
        env: Some(EnvConfig {
            allow: Some(allow.iter().map(|s| (*s).to_string()).collect()),
            mask: Some(vec![]),
            allow_read_all: read_all,
        }),
        ..Config::default()
    }
}

#[test]
fn cvs_23_base_allow_read_all_kept() {
    let m = merge_configs(&env(&["HOME"], Some(false)), &env(&["PATH"], None));
    assert_eq!(m.env.unwrap().allow_read_all, Some(false));
}

#[test]
fn cvs_24_override_false_wins() {
    let m = merge_configs(&env(&["HOME"], Some(true)), &env(&["PATH"], Some(false)));
    assert_eq!(m.env.unwrap().allow_read_all, Some(false));
}

#[test]
fn cvs_25_base_true_kept() {
    let m = merge_configs(&env(&[], Some(true)), &env(&[], None));
    assert_eq!(m.env.unwrap().allow_read_all, Some(true));
}

#[test]
fn cvs_26_override_true_wins() {
    let m = merge_configs(&env(&[], Some(false)), &env(&[], Some(true)));
    assert_eq!(m.env.unwrap().allow_read_all, Some(true));
}

#[test]
fn cvs_27_unset_stays_unset() {
    let m = merge_configs(&env(&[], None), &env(&[], None));
    assert_eq!(m.env.unwrap().allow_read_all, None);
}

#[test]
fn cvs_env_allow_is_union() {
    let m = merge_configs(&env(&["HOME"], None), &env(&["PATH", "HOME"], None));
    assert_eq!(
        m.env.unwrap().allow,
        Some(vec!["HOME".into(), "PATH".into()])
    );
}

#[test]
fn pd_22_override_scalars() {
    let base = Config {
        project_dir: Some("/base".into()),
        allow_project_commands: Some(false),
        ..Config::default()
    };
    let over = Config {
        project_dir: Some("/override".into()),
        allow_project_commands: Some(true),
        ..Config::default()
    };
    let m = merge_configs(&base, &over);
    assert_eq!(m.project_dir.as_deref(), Some("/override"));
    assert_eq!(m.allow_project_commands, Some(true));
}

#[test]
fn pd_23_empty_override_keeps_base() {
    let base = Config {
        project_dir: Some("/base".into()),
        allow_project_commands: Some(true),
        ..Config::default()
    };
    let m = merge_configs(&base, &Config::default());
    assert_eq!(m.project_dir.as_deref(), Some("/base"));
    assert_eq!(m.allow_project_commands, Some(true));
}

#[test]
fn pd_28_override_block_write() {
    let base = Config {
        block_project_dir_write: Some(false),
        ..Config::default()
    };
    let over = Config {
        block_project_dir_write: Some(true),
        ..Config::default()
    };
    assert_eq!(
        merge_configs(&base, &over).block_project_dir_write,
        Some(true)
    );
}

#[test]
fn pd_29_base_block_write_kept() {
    let base = Config {
        block_project_dir_write: Some(true),
        ..Config::default()
    };
    assert_eq!(
        merge_configs(&base, &Config::default()).block_project_dir_write,
        Some(true)
    );
}

fn net(n: NetPermission) -> Config {
    Config {
        permissions: Some(PermissionsConfig {
            net: Some(n),
            ..Default::default()
        }),
        ..Config::default()
    }
}

#[test]
fn net_true_wins_and_hosts_union() {
    let hosts = |h: &[&str]| NetPermission::Hosts(h.iter().map(|s| (*s).to_string()).collect());
    let m = merge_configs(&net(hosts(&["a"])), &net(NetPermission::All(true)));
    assert_eq!(m.permissions.unwrap().net, Some(NetPermission::All(true)));
    let m = merge_configs(&net(hosts(&["a"])), &net(hosts(&["b", "a"])));
    assert_eq!(m.permissions.unwrap().net, Some(hosts(&["a", "b"])));
}

#[test]
fn merge_does_not_mutate_inputs_and_empty_keeps_net() {
    let base = net(NetPermission::Hosts(vec!["existing-host.com".into()]));
    let copy = base.clone();
    let m = merge_configs(&base, &Config::default());
    assert_eq!(base, copy);
    assert_eq!(
        m.permissions.unwrap().net,
        Some(NetPermission::Hosts(vec!["existing-host.com".into()]))
    );
}

#[test]
fn array_fields_union_deduped() {
    let a = Config {
        workspace_roots: Some(vec!["/a".into()]),
        permissions: Some(PermissionsConfig {
            run: Some(vec!["git".into()]),
            ..Default::default()
        }),
        ..Config::default()
    };
    let b = Config {
        workspace_roots: Some(vec!["/b".into(), "/a".into()]),
        permissions: Some(PermissionsConfig {
            run: Some(vec!["git".into(), "ls".into()]),
            ..Default::default()
        }),
        ..Config::default()
    };
    let m = merge_configs(&a, &b);
    assert_eq!(m.workspace_roots, Some(vec!["/a".into(), "/b".into()]));
    assert_eq!(
        m.permissions.unwrap().run,
        Some(vec!["git".into(), "ls".into()])
    );
}
