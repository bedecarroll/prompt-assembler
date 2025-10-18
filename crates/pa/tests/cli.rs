use std::fs;
use std::io::Write;

use assert_cmd::Command;
use camino::{Utf8Path, Utf8PathBuf};
use predicates::prelude::*;
use tempfile::TempDir;

fn utf8_path(path: &std::path::Path) -> &Utf8Path {
    Utf8Path::from_path(path).expect("valid utf-8 path")
}

fn write_file(dir: &Utf8Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent.as_std_path()).unwrap();
    }
    let mut file = fs::File::create(path.as_std_path()).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
}

fn prepare_config(temp: &TempDir) -> (Utf8PathBuf, Utf8PathBuf) {
    let root = utf8_path(temp.path());
    let xdg_config_home = root.join("xdg-config");
    let library_dir = xdg_config_home.join("prompt-assembler");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    (xdg_config_home, library_dir)
}

fn command_with_xdg(temp: &TempDir, xdg_config_home: &Utf8Path) -> Command {
    let mut cmd = Command::cargo_bin("pa").unwrap();
    cmd.env("XDG_CONFIG_HOME", xdg_config_home);
    cmd.current_dir(temp.path());
    cmd
}

#[test]
fn prints_sequence_prompt_output() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        r#"[prompt.simple]
prompts = ["body.md"]
"#,
    )
    .unwrap();
    write_file(&library_dir, "body.md", "Hello {0}!\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("simple").arg("World");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Hello World!"));
}

#[test]
fn stdin_provides_first_argument() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        r#"[prompt.echo]
prompts = ["echo.md"]
"#,
    )
    .unwrap();
    write_file(&library_dir, "echo.md", "Echo {0}\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("echo").write_stdin("piped text\n");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Echo piped text"));
}

#[test]
fn prints_template_prompt_with_json_data() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        r#"[prompt.troubleshoot]
template = "troubleshoot.j2"
"#,
    )
    .unwrap();
    write_file(&library_dir, "troubleshoot.j2", "Issue: {{ issue }}\n");

    let data_path = library_dir.join("vars.json");
    fs::write(data_path.as_std_path(), r#"{"issue": "network"}"#).unwrap();

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("troubleshoot").arg(data_path.as_str());

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Issue: network"));
}

#[test]
fn errors_when_prompt_missing_arguments() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        r#"[prompt.warning]
prompts = ["warn.md"]
"#,
    )
    .unwrap();
    write_file(&library_dir, "warn.md", "Warn {0} {1}\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("warning").arg("only-one");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("missing argument"));
}

#[test]
fn list_command_prints_available_prompts() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.alpha]\nprompts = [\"a.md\"]\n[prompt.bravo]\nprompts = [\"b.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "a.md", "A\n");
    write_file(&library_dir, "b.md", "B\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("list");

    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines, vec!["alpha", "bravo"]);
}

#[test]
fn completions_include_prompt_names() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.sample]\nprompts = [\"sample.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "sample.md", "Sample\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.args(["completions", "bash"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("sample"));
}

#[test]
fn completions_include_prompts_from_conf_d() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.alpha]\nprompts = [\"alpha.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "alpha.md", "Alpha\n");
    let conf_d = library_dir.join("conf.d");
    fs::create_dir_all(conf_d.as_std_path()).unwrap();
    fs::write(
        conf_d.join("10-extra.toml").as_std_path(),
        "[prompt.extra]\nprompts = [\"extra.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "extra.md", "Extra\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.args(["completions", "zsh"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("extra"));
}

#[test]
fn completions_error_on_unsupported_shell() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.alpha]\nprompts = [\"alpha.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "alpha.md", "Alpha\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.args(["completions", "unknown-shell"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unsupported shell"));
}

#[test]
fn parts_command_concatenates_files_from_cwd_and_prompt_path() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        r#"
prompt_path = "snippets"

[prompt.placeholder]
prompts = ["placeholder.md"]
"#,
    )
    .unwrap();
    write_file(&library_dir, "snippets/placeholder.md", "unused\n");
    write_file(&library_dir, "snippets/library.md", "Library keeps {0}\n");
    write_file(root, "local.md", "Local holds {0}\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.args(["parts", "local.md", "library.md"]);

    cmd.assert().success().stdout(predicate::str::contains(
        "Local holds {0}\nLibrary keeps {0}\n",
    ));
}

#[test]
fn parts_command_errors_when_file_missing() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        r#"
prompt_path = "snippets"

[prompt.placeholder]
prompts = ["placeholder.md"]
"#,
    )
    .unwrap();
    write_file(&library_dir, "snippets/placeholder.md", "unused\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.args(["parts", "missing.md"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("missing part"));
}

#[test]
fn errors_for_unknown_prompt_name() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.alpha]\nprompts = [\"alpha.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "alpha.md", "Alpha\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("missing");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unknown prompt"));
}

#[test]
fn errors_when_template_missing_data_cli() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.tmpl]\ntemplate = \"tmpl.j2\"\n",
    )
    .unwrap();
    write_file(&library_dir, "tmpl.j2", "{{ value }}\n");

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("tmpl");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("data file"));
}

#[test]
fn errors_when_sequence_prompt_passed_data_file_cli() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.seq]\nprompts = [\"seq.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "seq.md", "Seq\n");
    let data_path = library_dir.join("data.toml");
    fs::write(data_path.as_std_path(), "value = \"v\"\n").unwrap();

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("seq").arg(data_path.as_str());

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("structured data"));
}

#[test]
fn cli_uses_conf_d_override() {
    let temp = TempDir::new().unwrap();
    let (xdg_home, library_dir) = prepare_config(&temp);

    fs::write(
        library_dir.join("config.toml").as_std_path(),
        "[prompt.note]\nprompts = [\"note.md\"]\n",
    )
    .unwrap();
    write_file(&library_dir, "note.md", "Base\n");

    let conf_d = library_dir.join("conf.d");
    fs::create_dir_all(conf_d.as_std_path()).unwrap();
    fs::write(
        conf_d.join("20-override.toml").as_std_path(),
        "[prompt.note]\ntemplate = \"note.j2\"\n",
    )
    .unwrap();
    write_file(&library_dir, "note.j2", "Override {{ val }}\n");
    let data_path = library_dir.join("vars.json");
    fs::write(data_path.as_std_path(), r#"{"val": "yes"}"#).unwrap();

    let mut cmd = command_with_xdg(&temp, xdg_home.as_ref());
    cmd.arg("note").arg(data_path.as_str());

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Override yes"));
}
