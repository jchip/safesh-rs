//! Interpret stage: argv in, run/path/ask items out.

use safesh_policy::effect::Access;
use safesh_policy::spec::{Item, interpret};

fn go(parts: &[&str]) -> Vec<Item> {
    let argv: Vec<String> = parts.iter().map(|s| (*s).to_string()).collect();
    interpret(&argv)
}

fn run(p: &str) -> Item {
    Item::Run(p.to_string())
}

fn w(p: &str) -> Item {
    Item::Path {
        path: p.to_string(),
        access: Access::Write,
    }
}

fn r(p: &str) -> Item {
    Item::Path {
        path: p.to_string(),
        access: Access::Read,
    }
}

fn ask(s: &str) -> Item {
    Item::Ask(s.to_string())
}

// ------------------------------------------------------------ basics

#[test]
fn empty_argv_yields_nothing() {
    assert!(go(&[]).is_empty());
}

#[test]
fn unknown_command_is_run_only() {
    assert_eq!(go(&["ls", "-la", "/etc"]), vec![run("ls")]);
}

#[test]
fn run_keeps_program_as_written_and_matches_by_basename() {
    assert_eq!(go(&["/bin/rm", "x"]), vec![run("/bin/rm"), w("x")]);
}

#[test]
fn outer_only_commands() {
    assert_eq!(go(&["ssh", "host", "rm", "-rf", "/"]), vec![run("ssh")]);
    assert_eq!(go(&["docker", "exec", "c", "rm", "x"]), vec![run("docker")]);
    assert_eq!(
        go(&["docker", "run", "img", "rm", "x"]),
        vec![run("docker")]
    );
    assert_eq!(
        go(&["kubectl", "exec", "p", "--", "rm", "x"]),
        vec![run("kubectl")]
    );
}

// ------------------------------------------------------------ shells

#[test]
fn shells_emit_no_run() {
    assert!(go(&["bash", "-c", "rm -rf /"]).is_empty());
    assert!(go(&["sh", "-c", "echo"]).is_empty());
    assert!(go(&["/bin/dash", "-c", "echo"]).is_empty());
}

#[test]
fn shell_script_is_read() {
    assert_eq!(go(&["bash", "script.sh", "arg"]), vec![r("script.sh")]);
    assert_eq!(go(&["sh", "script.sh"]), vec![r("script.sh")]);
    assert_eq!(go(&["bash", "-e", "-x", "s.sh"]), vec![r("s.sh")]);
}

#[test]
fn shell_option_values_are_skipped() {
    assert_eq!(go(&["bash", "-o", "pipefail", "s.sh"]), vec![r("s.sh")]);
    assert_eq!(go(&["bash", "--rcfile", "rc", "s.sh"]), vec![r("s.sh")]);
}

#[test]
fn shell_dash_c_in_cluster_has_no_script() {
    assert!(go(&["bash", "-lc", "echo hi"]).is_empty());
}

#[test]
fn shell_dash_s_reads_stdin() {
    assert!(go(&["bash", "-s"]).is_empty());
}

#[test]
fn shell_double_dash_then_script() {
    assert_eq!(go(&["bash", "--", "-weird.sh"]), vec![r("-weird.sh")]);
}

// ------------------------------------------------------------ all-operand writers

#[test]
fn rm_all_operands() {
    assert_eq!(go(&["rm", "a", "b"]), vec![run("rm"), w("a"), w("b")]);
}

#[test]
fn rm_cluster_flags() {
    assert_eq!(go(&["rm", "-rf", "x"]), vec![run("rm"), w("x")]);
    assert_eq!(go(&["rm", "-r", "-f", "x"]), vec![run("rm"), w("x")]);
}

#[test]
fn rm_double_dash_ends_options() {
    assert_eq!(
        go(&["rm", "--", "-rf", "x"]),
        vec![run("rm"), w("-rf"), w("x")]
    );
}

#[test]
fn rm_long_flag_with_equals() {
    assert_eq!(
        go(&["rm", "--interactive=never", "x"]),
        vec![run("rm"), w("x")]
    );
}

#[test]
fn rmdir_operands() {
    assert_eq!(go(&["rmdir", "-p", "a/b"]), vec![run("rmdir"), w("a/b")]);
}

#[test]
fn touch_operands_and_value_flags() {
    // GNU/BSD touch: -d DATE, -r REF, -t STAMP take a value.
    assert_eq!(
        go(&["touch", "-d", "yesterday", "f"]),
        vec![run("touch"), w("f")]
    );
    assert_eq!(go(&["touch", "-r", "ref", "f"]), vec![run("touch"), w("f")]);
    assert_eq!(
        go(&["touch", "-t", "202001010000", "f"]),
        vec![run("touch"), w("f")]
    );
    assert_eq!(
        go(&["touch", "--date=2020-01-01", "f"]),
        vec![run("touch"), w("f")]
    );
}

#[test]
fn touch_value_in_cluster() {
    // `-ad` takes the value for `d` from the next arg.
    assert_eq!(
        go(&["touch", "-ad", "now", "f"]),
        vec![run("touch"), w("f")]
    );
}

#[test]
fn mkdir_mode_value_flag() {
    // GNU/BSD mkdir: -m MODE.
    assert_eq!(
        go(&["mkdir", "-p", "-m", "755", "d"]),
        vec![run("mkdir"), w("d")]
    );
    assert_eq!(go(&["mkdir", "-m755", "d"]), vec![run("mkdir"), w("d")]);
    assert_eq!(
        go(&["mkdir", "--mode=700", "d"]),
        vec![run("mkdir"), w("d")]
    );
}

#[test]
fn truncate_size_and_reference_values() {
    // GNU truncate: -s SIZE, -r RFILE take a value.
    assert_eq!(
        go(&["truncate", "-s", "0", "f"]),
        vec![run("truncate"), w("f")]
    );
    assert_eq!(
        go(&["truncate", "--size=10", "f", "g"]),
        vec![run("truncate"), w("f"), w("g")]
    );
    assert_eq!(
        go(&["truncate", "-r", "ref", "f"]),
        vec![run("truncate"), w("f")]
    );
}

#[test]
fn shred_iterations_value() {
    // GNU shred: -n N, -s SIZE take a value.
    assert_eq!(
        go(&["shred", "-n", "3", "-u", "f"]),
        vec![run("shred"), w("f")]
    );
    assert_eq!(go(&["shred", "-n3", "f"]), vec![run("shred"), w("f")]);
}

#[test]
fn tee_operands() {
    assert_eq!(
        go(&["tee", "-a", "out", "out2"]),
        vec![run("tee"), w("out"), w("out2")]
    );
    assert_eq!(go(&["tee"]), vec![run("tee")]);
}

#[test]
fn value_flag_without_value_asks() {
    assert_eq!(
        go(&["touch", "-d"]),
        vec![run("touch"), ask("touch: cannot read its arguments")]
    );
}

// ------------------------------------------------------------ mv, cp, install, ln, rsync

#[test]
fn mv_sources_and_dest() {
    assert_eq!(
        go(&["mv", "a", "b", "dir"]),
        vec![run("mv"), w("a"), w("b"), w("dir")]
    );
}

#[test]
fn mv_target_directory_short_and_long() {
    // GNU mv: -t DIR / --target-directory=DIR puts all operands inside DIR.
    assert_eq!(
        go(&["mv", "-t", "dir", "a", "b"]),
        vec![run("mv"), w("dir"), w("a"), w("b")]
    );
    assert_eq!(
        go(&["mv", "--target-directory=dir", "a"]),
        vec![run("mv"), w("dir"), w("a")]
    );
    assert_eq!(
        go(&["mv", "--target-directory", "dir", "a"]),
        vec![run("mv"), w("dir"), w("a")]
    );
}

#[test]
fn mv_no_target_directory_flag() {
    // GNU mv -T: treat DEST as a normal file.
    assert_eq!(go(&["mv", "-T", "a", "b"]), vec![run("mv"), w("a"), w("b")]);
}

#[test]
fn mv_one_operand_asks() {
    assert_eq!(
        go(&["mv", "a"]),
        vec![run("mv"), ask("mv: cannot read its arguments")]
    );
}

#[test]
fn cp_destination_only() {
    assert_eq!(go(&["cp", "-a", "src", "dst"]), vec![run("cp"), w("dst")]);
    assert_eq!(go(&["cp", "a", "b", "dir"]), vec![run("cp"), w("dir")]);
}

#[test]
fn cp_target_directory() {
    assert_eq!(
        go(&["cp", "-t", "dir", "a", "b"]),
        vec![run("cp"), w("dir")]
    );
    assert_eq!(go(&["cp", "-rt", "dir", "a"]), vec![run("cp"), w("dir")]);
    assert_eq!(
        go(&["cp", "--target-directory=dir", "a"]),
        vec![run("cp"), w("dir")]
    );
}

#[test]
fn cp_no_target_directory_flag() {
    assert_eq!(go(&["cp", "-T", "a", "b"]), vec![run("cp"), w("b")]);
}

#[test]
fn cp_suffix_value_flag() {
    // GNU cp: -S SUFFIX takes a value.
    assert_eq!(
        go(&["cp", "-b", "-S", ".orig", "a", "b"]),
        vec![run("cp"), w("b")]
    );
}

#[test]
fn cp_double_dash() {
    assert_eq!(go(&["cp", "--", "-a", "-b"]), vec![run("cp"), w("-b")]);
}

#[test]
fn cp_too_few_operands_asks() {
    assert_eq!(
        go(&["cp", "a"]),
        vec![run("cp"), ask("cp: cannot read its arguments")]
    );
    assert_eq!(
        go(&["cp"]),
        vec![run("cp"), ask("cp: cannot read its arguments")]
    );
}

#[test]
fn install_destination() {
    assert_eq!(
        go(&["install", "-m", "755", "bin", "/usr/local/bin/x"]),
        vec![run("install"), w("/usr/local/bin/x")]
    );
    assert_eq!(
        go(&["install", "-m755", "-o", "root", "-g", "wheel", "a", "b"]),
        vec![run("install"), w("b")]
    );
}

#[test]
fn install_target_directory() {
    assert_eq!(
        go(&["install", "-t", "/opt/bin", "a", "b"]),
        vec![run("install"), w("/opt/bin")]
    );
}

#[test]
fn install_directory_mode_writes_all_operands() {
    // install -d creates each operand as a directory.
    assert_eq!(
        go(&["install", "-d", "-m", "755", "d1", "d2"]),
        vec![run("install"), w("d1"), w("d2")]
    );
}

#[test]
fn ln_destination() {
    assert_eq!(
        go(&["ln", "-sf", "target", "link"]),
        vec![run("ln"), w("link")]
    );
}

#[test]
fn ln_single_operand_links_into_cwd() {
    assert_eq!(go(&["ln", "-s", "/a/b"]), vec![run("ln"), w(".")]);
}

#[test]
fn ln_target_directory_and_no_target() {
    assert_eq!(
        go(&["ln", "-s", "-t", "dir", "a"]),
        vec![run("ln"), w("dir")]
    );
    assert_eq!(go(&["ln", "-sT", "a", "l"]), vec![run("ln"), w("l")]);
}

#[test]
fn rsync_destination() {
    assert_eq!(
        go(&["rsync", "-av", "src/", "dst/"]),
        vec![run("rsync"), w("dst/")]
    );
}

#[test]
fn rsync_value_flags_do_not_become_operands() {
    // rsync: -e COMMAND, --exclude=PAT, --port N take a value.
    assert_eq!(
        go(&[
            "rsync",
            "-a",
            "-e",
            "ssh -p 22",
            "--exclude",
            "*.o",
            "s",
            "d"
        ]),
        vec![run("rsync"), w("d")]
    );
    assert_eq!(
        go(&["rsync", "--exclude=*.o", "s", "d"]),
        vec![run("rsync"), w("d")]
    );
}

#[test]
fn rsync_remote_destination_writes_nothing_local() {
    assert_eq!(go(&["rsync", "-a", "s", "host:/d"]), vec![run("rsync")]);
    assert_eq!(
        go(&["rsync", "-a", "s", "rsync://host/mod"]),
        vec![run("rsync")]
    );
}

#[test]
fn rsync_remote_source_local_dest() {
    assert_eq!(
        go(&["rsync", "-a", "host:/s", "d"]),
        vec![run("rsync"), w("d")]
    );
}

// ------------------------------------------------------------ chmod, chown, chgrp

#[test]
fn chmod_skips_mode() {
    assert_eq!(
        go(&["chmod", "-R", "755", "a", "b"]),
        vec![run("chmod"), w("a"), w("b")]
    );
}

#[test]
fn chmod_symbolic_mode_with_leading_dash() {
    // `-w` is a mode here (remove write), not an option.
    assert_eq!(go(&["chmod", "-w", "f"]), vec![run("chmod"), w("f")]);
    assert_eq!(go(&["chmod", "a+x", "f"]), vec![run("chmod"), w("f")]);
}

#[test]
fn chmod_reference_means_all_operands_are_files() {
    // GNU chmod --reference=RFILE: no MODE operand.
    assert_eq!(
        go(&["chmod", "--reference=ref", "f", "g"]),
        vec![run("chmod"), w("f"), w("g")]
    );
    assert_eq!(
        go(&["chmod", "--reference", "ref", "f"]),
        vec![run("chmod"), w("f")]
    );
}

#[test]
fn chmod_double_dash() {
    assert_eq!(go(&["chmod", "--", "-x", "f"]), vec![run("chmod"), w("f")]);
}

#[test]
fn chown_skips_owner() {
    assert_eq!(
        go(&["chown", "-R", "root:wheel", "d"]),
        vec![run("chown"), w("d")]
    );
}

#[test]
fn chown_reference_and_from() {
    assert_eq!(
        go(&["chown", "--reference=ref", "f"]),
        vec![run("chown"), w("f")]
    );
    // GNU chown --from=CURRENT: filter, takes a value.
    assert_eq!(
        go(&["chown", "--from=a:b", "c:d", "f"]),
        vec![run("chown"), w("f")]
    );
}

#[test]
fn chgrp_skips_group() {
    assert_eq!(
        go(&["chgrp", "-h", "staff", "f", "g"]),
        vec![run("chgrp"), w("f"), w("g")]
    );
    assert_eq!(
        go(&["chgrp", "--reference=ref", "f"]),
        vec![run("chgrp"), w("f")]
    );
}

// ------------------------------------------------------------ sed, perl, dd

#[test]
fn sed_without_in_place_writes_nothing() {
    assert_eq!(go(&["sed", "s/a/b/", "f"]), vec![run("sed")]);
}

#[test]
fn sed_in_place_gnu() {
    assert_eq!(
        go(&["sed", "-i", "s/a/b/", "f", "g"]),
        vec![run("sed"), w("f"), w("g")]
    );
}

#[test]
fn sed_in_place_with_suffix_attached() {
    // GNU/BSD: -i.bak attaches the backup suffix.
    assert_eq!(
        go(&["sed", "-i.bak", "s/a/b/", "f"]),
        vec![run("sed"), w("f")]
    );
}

#[test]
fn sed_in_place_bsd_empty_suffix() {
    // BSD/macOS: `-i ''` passes an empty backup suffix as its own arg.
    assert_eq!(
        go(&["sed", "-i", "", "s/a/b/", "f"]),
        vec![run("sed"), w("f")]
    );
}

#[test]
fn sed_long_in_place() {
    assert_eq!(
        go(&["sed", "--in-place", "s/a/b/", "f"]),
        vec![run("sed"), w("f")]
    );
    assert_eq!(
        go(&["sed", "--in-place=.bak", "s/a/b/", "f"]),
        vec![run("sed"), w("f")]
    );
}

#[test]
fn sed_script_flags_skip_the_script_operand() {
    // -e SCRIPT and -f FILE supply the script, so every operand is a file.
    assert_eq!(
        go(&["sed", "-i", "-e", "s/a/b/", "-e", "s/c/d/", "f"]),
        vec![run("sed"), w("f")]
    );
    assert_eq!(
        go(&["sed", "-i", "-f", "script.sed", "f"]),
        vec![run("sed"), w("f")]
    );
    assert_eq!(
        go(&["sed", "--in-place", "--expression=s/a/b/", "f"]),
        vec![run("sed"), w("f")]
    );
}

#[test]
fn sed_cluster_with_e_value() {
    assert_eq!(
        go(&["sed", "-n", "-i", "-e", "p", "f"]),
        vec![run("sed"), w("f")]
    );
    assert_eq!(go(&["sed", "-Ei", "s/a/b/", "f"]), vec![run("sed"), w("f")]);
}

#[test]
fn sed_double_dash() {
    assert_eq!(
        go(&["sed", "-i", "s/a/b/", "--", "-f"]),
        vec![run("sed"), w("-f")]
    );
}

#[test]
fn sed_in_place_without_script_asks() {
    assert_eq!(
        go(&["sed", "-i"]),
        vec![run("sed"), ask("sed: cannot read its arguments")]
    );
}

#[test]
fn perl_in_place() {
    assert_eq!(
        go(&["perl", "-i", "-pe", "s/a/b/", "f"]),
        vec![run("perl"), w("f")]
    );
    assert_eq!(
        go(&["perl", "-pi", "-e", "s/a/b/", "f", "g"]),
        vec![run("perl"), w("f"), w("g")]
    );
}

#[test]
fn perl_in_place_with_suffix() {
    assert_eq!(
        go(&["perl", "-i.bak", "-pe", "s/a/b/", "f"]),
        vec![run("perl"), w("f")]
    );
}

#[test]
fn perl_pie_makes_e_the_suffix() {
    // In `-pie`, perl reads `e` as the -i suffix, so the next arg is the script file.
    assert_eq!(
        go(&["perl", "-pie", "s/a/b/", "f"]),
        vec![run("perl"), w("f")]
    );
}

#[test]
fn perl_script_file_is_not_written() {
    assert_eq!(
        go(&["perl", "-i", "script.pl", "f"]),
        vec![run("perl"), w("f")]
    );
}

#[test]
fn perl_without_in_place_writes_nothing() {
    assert_eq!(go(&["perl", "-e", "print 1", "f"]), vec![run("perl")]);
}

#[test]
fn perl_digit_flags_keep_the_cluster_going() {
    // -l and -0 take digits only, so `i` is still seen in `-0777pi`.
    assert_eq!(
        go(&["perl", "-0777pi", "-e", "1", "f"]),
        vec![run("perl"), w("f")]
    );
    assert_eq!(
        go(&["perl", "-lpi", "-e", "1", "f"]),
        vec![run("perl"), w("f")]
    );
}

#[test]
fn perl_include_dir_value() {
    assert_eq!(
        go(&["perl", "-I", "lib", "-i", "-e", "1", "f"]),
        vec![run("perl"), w("f")]
    );
}

#[test]
fn dd_of() {
    assert_eq!(
        go(&["dd", "if=/dev/zero", "of=/tmp/x", "bs=1M"]),
        vec![run("dd"), w("/tmp/x")]
    );
}

#[test]
fn dd_without_of_writes_no_path() {
    assert_eq!(go(&["dd", "if=a"]), vec![run("dd")]);
}

// ------------------------------------------------------------ tar, unzip

#[test]
fn tar_extract_default_cwd() {
    assert_eq!(go(&["tar", "-xzf", "a.tgz"]), vec![run("tar"), w(".")]);
    assert_eq!(
        go(&["tar", "--extract", "-f", "a.tar"]),
        vec![run("tar"), w(".")]
    );
    assert_eq!(
        go(&["tar", "--get", "-f", "a.tar"]),
        vec![run("tar"), w(".")]
    );
}

#[test]
fn tar_extract_with_directory() {
    // GNU/BSD tar: -C DIR / --directory=DIR changes into DIR first.
    assert_eq!(
        go(&["tar", "-xf", "a.tar", "-C", "out"]),
        vec![run("tar"), w("out")]
    );
    assert_eq!(
        go(&["tar", "-xf", "a.tar", "--directory=out"]),
        vec![run("tar"), w("out")]
    );
    assert_eq!(
        go(&["tar", "-xf", "a.tar", "--directory", "out"]),
        vec![run("tar"), w("out")]
    );
}

#[test]
fn tar_old_style_bundle() {
    // `tar xzf a.tgz`: no dash, `f` takes the next operand.
    assert_eq!(go(&["tar", "xzf", "a.tgz"]), vec![run("tar"), w(".")]);
    assert_eq!(
        go(&["tar", "xfC", "a.tar", "out"]),
        vec![run("tar"), w("out")]
    );
}

#[test]
fn tar_create_and_list_write_nothing_by_table() {
    assert_eq!(go(&["tar", "-czf", "a.tgz", "d"]), vec![run("tar")]);
    assert_eq!(go(&["tar", "-tf", "a.tar"]), vec![run("tar")]);
}

#[test]
fn tar_value_flags_are_skipped() {
    assert_eq!(
        go(&[
            "tar",
            "-xf",
            "a.tar",
            "--exclude",
            "*.o",
            "--strip-components=1"
        ]),
        vec![run("tar"), w(".")]
    );
}

#[test]
fn tar_missing_value_asks() {
    assert_eq!(
        go(&["tar", "-xf"]),
        vec![run("tar"), ask("tar: cannot read its arguments")]
    );
}

#[test]
fn unzip_default_cwd() {
    assert_eq!(go(&["unzip", "a.zip"]), vec![run("unzip"), w(".")]);
    assert_eq!(
        go(&["unzip", "-o", "-q", "a.zip"]),
        vec![run("unzip"), w(".")]
    );
}

#[test]
fn unzip_d_target() {
    // Info-ZIP unzip: -d DIR extracts into DIR; also accepted attached.
    assert_eq!(
        go(&["unzip", "a.zip", "-d", "out"]),
        vec![run("unzip"), w("out")]
    );
    assert_eq!(
        go(&["unzip", "-d", "out", "a.zip"]),
        vec![run("unzip"), w("out")]
    );
    assert_eq!(
        go(&["unzip", "-dout", "a.zip"]),
        vec![run("unzip"), w("out")]
    );
}

#[test]
fn unzip_list_and_test_write_nothing() {
    // Info-ZIP unzip: -l list, -t test, -p to stdout.
    assert_eq!(go(&["unzip", "-l", "a.zip"]), vec![run("unzip")]);
    assert_eq!(go(&["unzip", "-t", "a.zip"]), vec![run("unzip")]);
    assert_eq!(go(&["unzip", "-p", "a.zip"]), vec![run("unzip")]);
}

#[test]
fn unzip_missing_d_value_asks() {
    assert_eq!(
        go(&["unzip", "a.zip", "-d"]),
        vec![run("unzip"), ask("unzip: cannot read its arguments")]
    );
}

// ------------------------------------------------------------ curl, wget

#[test]
fn curl_output_file() {
    assert_eq!(
        go(&["curl", "-o", "out", "http://x"]),
        vec![run("curl"), w("out")]
    );
    assert_eq!(
        go(&["curl", "-sSo", "out", "http://x"]),
        vec![run("curl"), w("out")]
    );
    assert_eq!(
        go(&["curl", "--output", "out", "http://x"]),
        vec![run("curl"), w("out")]
    );
    assert_eq!(
        go(&["curl", "--output=out", "http://x"]),
        vec![run("curl"), w("out")]
    );
    assert_eq!(
        go(&["curl", "-oout", "http://x"]),
        vec![run("curl"), w("out")]
    );
}

#[test]
fn curl_remote_name_writes_into_cwd() {
    // curl -O saves under the remote file name in the current directory.
    assert_eq!(go(&["curl", "-O", "http://x/f"]), vec![run("curl"), w(".")]);
    assert_eq!(
        go(&["curl", "--remote-name", "http://x/f"]),
        vec![run("curl"), w(".")]
    );
    assert_eq!(
        go(&["curl", "-LO", "http://x/f"]),
        vec![run("curl"), w(".")]
    );
}

#[test]
fn curl_output_dir() {
    // curl --output-dir DIR: -o and -O resolve under DIR.
    assert_eq!(
        go(&["curl", "--output-dir", "d", "-o", "f", "http://x"]),
        vec![run("curl"), w("d/f")]
    );
    assert_eq!(
        go(&["curl", "--output-dir", "d", "-O", "http://x/f"]),
        vec![run("curl"), w("d")]
    );
}

#[test]
fn curl_stdout_and_plain_write_nothing() {
    assert_eq!(go(&["curl", "-o", "-", "http://x"]), vec![run("curl")]);
    assert_eq!(go(&["curl", "-s", "http://x"]), vec![run("curl")]);
}

#[test]
fn curl_value_flags_are_skipped() {
    // -H HEADER and -d DATA take a value.
    assert_eq!(
        go(&["curl", "-H", "A: b", "-d", "x=1", "-o", "out", "http://x"]),
        vec![run("curl"), w("out")]
    );
}

#[test]
fn curl_dump_header_and_cookie_jar_are_writes() {
    assert_eq!(
        go(&["curl", "-D", "h.txt", "-c", "jar", "http://x"]),
        vec![run("curl"), w("h.txt"), w("jar")]
    );
}

#[test]
fn curl_missing_output_value_asks() {
    assert_eq!(
        go(&["curl", "http://x", "-o"]),
        vec![run("curl"), ask("curl: cannot read its arguments")]
    );
}

#[test]
fn wget_output_document() {
    // GNU wget: -O FILE / --output-document=FILE.
    assert_eq!(
        go(&["wget", "-O", "out", "http://x"]),
        vec![run("wget"), w("out")]
    );
    assert_eq!(
        go(&["wget", "--output-document=out", "http://x"]),
        vec![run("wget"), w("out")]
    );
    assert_eq!(
        go(&["wget", "-qO", "out", "http://x"]),
        vec![run("wget"), w("out")]
    );
    assert_eq!(go(&["wget", "-O-", "http://x"]), vec![run("wget")]);
}

#[test]
fn wget_directory_prefix_and_default() {
    // GNU wget: -P PREFIX / --directory-prefix=PREFIX.
    assert_eq!(
        go(&["wget", "-P", "dl", "http://x"]),
        vec![run("wget"), w("dl")]
    );
    assert_eq!(
        go(&["wget", "--directory-prefix=dl", "http://x"]),
        vec![run("wget"), w("dl")]
    );
    assert_eq!(go(&["wget", "http://x"]), vec![run("wget"), w(".")]);
}

#[test]
fn wget_negation_cluster_and_log() {
    // `-nv` is one option (no-verbose); -o FILE is a log file write.
    assert_eq!(
        go(&["wget", "-nv", "-o", "log", "http://x"]),
        vec![run("wget"), w("."), w("log")]
    );
}

#[test]
fn wget_missing_value_asks() {
    assert_eq!(
        go(&["wget", "http://x", "-O"]),
        vec![run("wget"), ask("wget: cannot read its arguments")]
    );
}

// ------------------------------------------------------------ find, git

#[test]
fn find_delete_uses_each_root() {
    assert_eq!(
        go(&["find", "a", "b", "-name", "*.o", "-delete"]),
        vec![run("find"), w("a"), w("b")]
    );
}

#[test]
fn find_delete_defaults_to_cwd() {
    assert_eq!(
        go(&["find", "-name", "*.o", "-delete"]),
        vec![run("find"), w(".")]
    );
}

#[test]
fn find_without_delete_writes_nothing() {
    assert_eq!(go(&["find", ".", "-name", "x"]), vec![run("find")]);
}

#[test]
fn find_skips_global_options_and_value_primaries() {
    assert_eq!(
        go(&["find", "-L", "d", "-name", "-delete", "-type", "f"]),
        vec![run("find")]
    );
    assert_eq!(
        go(&["find", "-H", "d", "-delete"]),
        vec![run("find"), w("d")]
    );
}

#[test]
fn find_exec_with_placeholder() {
    assert_eq!(
        go(&["find", "d", "e", "-exec", "rm", "{}", ";"]),
        vec![run("find"), run("rm"), w("d"), w("e")]
    );
}

#[test]
fn find_exec_plus_terminator() {
    assert_eq!(
        go(&["find", "d", "-exec", "rm", "-f", "{}", "+"]),
        vec![run("find"), run("rm"), w("d")]
    );
}

#[test]
fn find_exec_variants() {
    for flag in ["-execdir", "-ok", "-okdir"] {
        assert_eq!(
            go(&["find", "d", flag, "rm", "{}", ";"]),
            vec![run("find"), run("rm"), w("d")],
            "{flag}"
        );
    }
}

#[test]
fn find_exec_non_writer() {
    assert_eq!(
        go(&["find", ".", "-exec", "grep", "x", "{}", ";"]),
        vec![run("find"), run("grep")]
    );
}

#[test]
fn find_exec_literal_path_stays() {
    assert_eq!(
        go(&["find", ".", "-exec", "cp", "{}", "/backup", ";"]),
        vec![run("find"), run("cp"), w("/backup")]
    );
}

#[test]
fn find_exec_dest_placeholder() {
    assert_eq!(
        go(&["find", "d", "-exec", "cp", "x", "{}", ";"]),
        vec![run("find"), run("cp"), w("d")]
    );
}

#[test]
fn find_exec_unterminated_asks() {
    assert_eq!(
        go(&["find", ".", "-exec", "rm", "{}"]),
        vec![run("find"), ask("find: cannot read its arguments")]
    );
}

#[test]
fn find_exec_nested_wrapper() {
    assert_eq!(
        go(&["find", "d", "-exec", "env", "A=1", "rm", "{}", ";"]),
        vec![run("find"), run("env"), run("rm"), w("d")]
    );
}

#[test]
fn find_delete_and_exec_together() {
    assert_eq!(
        go(&["find", "d", "-exec", "touch", "{}", ";", "-delete"]),
        vec![run("find"), run("touch"), w("d"), w("d")]
    );
}

#[test]
fn find_fprint_writes_file() {
    assert_eq!(
        go(&["find", ".", "-fprint", "list.txt"]),
        vec![run("find"), w("list.txt")]
    );
}

#[test]
fn git_clean_writes_cwd() {
    assert_eq!(go(&["git", "clean", "-fdx"]), vec![run("git"), w(".")]);
}

#[test]
fn git_clean_after_global_options() {
    assert_eq!(
        go(&["git", "-c", "a=b", "--no-pager", "clean", "-f"]),
        vec![run("git"), w(".")]
    );
    assert_eq!(
        go(&["git", "-C", "repo", "clean", "-f"]),
        vec![run("git"), w("repo")]
    );
}

#[test]
fn git_other_subcommands_run_only() {
    assert_eq!(go(&["git", "status"]), vec![run("git")]);
    assert_eq!(go(&["git", "-C", "clean", "status"]), vec![run("git")]);
}

// ------------------------------------------------------------ wrappers

#[test]
fn env_strips_assignments_and_flags() {
    assert_eq!(
        go(&["env", "A=1", "B=2", "rm", "x"]),
        vec![run("env"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["env", "-i", "A=1", "rm", "x"]),
        vec![run("env"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["env", "-", "rm", "x"]),
        vec![run("env"), run("rm"), w("x")]
    );
}

#[test]
fn env_unset_value() {
    // env -u NAME removes NAME from the environment.
    assert_eq!(
        go(&["env", "-u", "HOME", "rm", "x"]),
        vec![run("env"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["env", "--unset=HOME", "rm", "x"]),
        vec![run("env"), run("rm"), w("x")]
    );
}

#[test]
fn env_chdir_rebases_relative_paths() {
    // GNU env -C DIR runs the command in DIR.
    assert_eq!(
        go(&["env", "-C", "d", "rm", "x", "/abs"]),
        vec![run("env"), run("rm"), w("d/x"), w("/abs")]
    );
}

#[test]
fn env_dash_s_asks() {
    assert_eq!(
        go(&["env", "-S", "rm x"]),
        vec![run("env"), ask("env: cannot read its arguments")]
    );
}

#[test]
fn env_without_command() {
    assert_eq!(go(&["env", "A=1"]), vec![run("env")]);
    assert_eq!(go(&["env"]), vec![run("env")]);
}

#[test]
fn env_double_dash() {
    assert_eq!(
        go(&["env", "--", "rm", "x"]),
        vec![run("env"), run("rm"), w("x")]
    );
}

#[test]
fn nice_adjustment() {
    // nice -n N sets the niceness; `-10` legacy form is a flag cluster.
    assert_eq!(
        go(&["nice", "-n", "5", "rm", "x"]),
        vec![run("nice"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["nice", "--adjustment=5", "rm", "x"]),
        vec![run("nice"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["nice", "-10", "rm", "x"]),
        vec![run("nice"), run("rm"), w("x")]
    );
    assert_eq!(go(&["nice", "ls"]), vec![run("nice"), run("ls")]);
}

#[test]
fn nohup_wraps() {
    assert_eq!(
        go(&["nohup", "rm", "-rf", "x"]),
        vec![run("nohup"), run("rm"), w("x")]
    );
}

#[test]
fn time_wraps() {
    assert_eq!(
        go(&["time", "-p", "rm", "x"]),
        vec![run("time"), run("rm"), w("x")]
    );
}

#[test]
fn time_output_file_is_a_write() {
    // /usr/bin/time -o FILE writes the report to FILE.
    assert_eq!(
        go(&["time", "-o", "t.txt", "ls"]),
        vec![run("time"), w("t.txt"), run("ls")]
    );
}

#[test]
fn timeout_options_then_duration() {
    // timeout [OPTION] DURATION COMMAND; -k and -s take values.
    assert_eq!(
        go(&["timeout", "5", "rm", "x"]),
        vec![run("timeout"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["timeout", "-k", "2", "-s", "KILL", "5", "rm", "x"]),
        vec![run("timeout"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["timeout", "--signal=KILL", "--foreground", "5s", "rm", "x"]),
        vec![run("timeout"), run("rm"), w("x")]
    );
}

#[test]
fn timeout_without_command() {
    assert_eq!(go(&["timeout", "5"]), vec![run("timeout")]);
}

#[test]
fn stdbuf_modes() {
    // stdbuf -i/-o/-e MODE; attached and long forms.
    assert_eq!(
        go(&["stdbuf", "-o", "L", "rm", "x"]),
        vec![run("stdbuf"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["stdbuf", "-oL", "-eL", "rm", "x"]),
        vec![run("stdbuf"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["stdbuf", "--output=0", "-i", "0", "rm", "x"]),
        vec![run("stdbuf"), run("rm"), w("x")]
    );
}

#[test]
fn command_and_builtin_prefixes_are_not_wrappers() {
    // The engine runs the inner program itself, so the exec hook sees it directly.
    assert_eq!(go(&["command", "rm", "x"]), vec![run("command")]);
    assert_eq!(go(&["builtin", "echo"]), vec![run("builtin")]);
}

#[test]
fn nested_wrappers() {
    assert_eq!(
        go(&["env", "FOO=1", "timeout", "5", "rm", "-rf", "x"]),
        vec![run("env"), run("timeout"), run("rm"), w("x")]
    );
    assert_eq!(
        go(&["nice", "nohup", "env", "A=1", "mv", "a", "b"]),
        vec![
            run("nice"),
            run("nohup"),
            run("env"),
            run("mv"),
            w("a"),
            w("b")
        ]
    );
}

#[test]
fn wrapper_around_shell_emits_no_run_for_the_shell() {
    assert_eq!(
        go(&["env", "A=1", "bash", "s.sh"]),
        vec![run("env"), r("s.sh")]
    );
}

// ------------------------------------------------------------ xargs

#[test]
fn xargs_writer_asks_instead_of_paths() {
    assert_eq!(
        go(&["xargs", "rm"]),
        vec![
            run("xargs"),
            run("rm"),
            ask("xargs rm: paths come from input")
        ]
    );
    assert_eq!(
        go(&["xargs", "rm", "-f", "known"]),
        vec![
            run("xargs"),
            run("rm"),
            ask("xargs rm: paths come from input")
        ]
    );
}

#[test]
fn xargs_options_are_skipped() {
    // xargs: -n N, -P N, -I REPL, -d DELIM take values; -0 and -r are flags.
    assert_eq!(
        go(&["xargs", "-0", "-n", "1", "-P", "4", "rm"]),
        vec![
            run("xargs"),
            run("rm"),
            ask("xargs rm: paths come from input")
        ]
    );
    assert_eq!(
        go(&["xargs", "-I", "{}", "cp", "{}", "/d"]),
        vec![
            run("xargs"),
            run("cp"),
            ask("xargs cp: paths come from input")
        ]
    );
    assert_eq!(
        go(&["xargs", "-n1", "-d", "\\n", "touch"]),
        vec![
            run("xargs"),
            run("touch"),
            ask("xargs touch: paths come from input")
        ]
    );
}

#[test]
fn xargs_attached_optional_flags() {
    // GNU xargs: -i[R], -l[N], -e[EOF] take an attached value only.
    assert_eq!(
        go(&["xargs", "-i", "rm", "{}"]),
        vec![
            run("xargs"),
            run("rm"),
            ask("xargs rm: paths come from input")
        ]
    );
    assert_eq!(
        go(&["xargs", "-l", "grep", "x"]),
        vec![run("xargs"), run("grep")]
    );
}

#[test]
fn xargs_non_writer() {
    assert_eq!(
        go(&["xargs", "grep", "-l", "x"]),
        vec![run("xargs"), run("grep")]
    );
}

#[test]
fn xargs_without_command_is_echo() {
    assert_eq!(go(&["xargs"]), vec![run("xargs"), run("echo")]);
    assert_eq!(go(&["xargs", "-0"]), vec![run("xargs"), run("echo")]);
}

#[test]
fn xargs_double_dash() {
    assert_eq!(
        go(&["xargs", "--", "rm"]),
        vec![
            run("xargs"),
            run("rm"),
            ask("xargs rm: paths come from input")
        ]
    );
}

#[test]
fn xargs_through_wrapper() {
    assert_eq!(
        go(&["xargs", "env", "A=1", "rm"]),
        vec![
            run("xargs"),
            run("env"),
            run("rm"),
            ask("xargs env: paths come from input")
        ]
    );
}

#[test]
fn xargs_keeps_other_asks() {
    assert_eq!(
        go(&["xargs", "sudo", "rm"]),
        vec![
            run("xargs"),
            run("sudo"),
            ask("sudo runs as another user"),
            run("rm"),
            ask("xargs sudo: paths come from input")
        ]
    );
}

#[test]
fn xargs_missing_value_asks() {
    assert_eq!(
        go(&["xargs", "-n"]),
        vec![run("xargs"), ask("xargs: cannot read its arguments")]
    );
}

// ------------------------------------------------------------ sudo, doas

#[test]
fn sudo_asks_and_interprets_inner() {
    assert_eq!(
        go(&["sudo", "rm", "x"]),
        vec![
            run("sudo"),
            ask("sudo runs as another user"),
            run("rm"),
            w("x")
        ]
    );
}

#[test]
fn sudo_options_and_assignments() {
    assert_eq!(
        go(&["sudo", "-u", "bob", "-E", "FOO=1", "rm", "x"]),
        vec![
            run("sudo"),
            ask("sudo runs as another user"),
            run("rm"),
            w("x")
        ]
    );
    assert_eq!(
        go(&["sudo", "--user=bob", "--", "rm", "x"]),
        vec![
            run("sudo"),
            ask("sudo runs as another user"),
            run("rm"),
            w("x")
        ]
    );
}

#[test]
fn sudo_shell_mode_has_no_inner() {
    assert_eq!(
        go(&["sudo", "-i"]),
        vec![run("sudo"), ask("sudo runs as another user")]
    );
    assert_eq!(
        go(&["sudo", "-s"]),
        vec![run("sudo"), ask("sudo runs as another user")]
    );
}

#[test]
fn doas_asks_and_interprets_inner() {
    assert_eq!(
        go(&["doas", "-u", "bob", "rm", "x"]),
        vec![
            run("doas"),
            ask("doas runs as another user"),
            run("rm"),
            w("x")
        ]
    );
}

#[test]
fn sudo_without_command() {
    assert_eq!(
        go(&["sudo"]),
        vec![run("sudo"), ask("sudo runs as another user")]
    );
}
