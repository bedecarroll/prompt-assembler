use std::fs;
use std::io::Write;

use camino::Utf8Path;
use prompt_assembler::{LoadConfigError, PromptAssembler, StructuredData};
use tempfile::TempDir;

fn utf8_path(path: &std::path::Path) -> &Utf8Path {
    Utf8Path::from_path(path).expect("path is valid UTF-8")
}

fn write_file(dir: &Utf8Path, relative: &str, contents: &str) {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    let mut file = std::fs::File::create(path.as_std_path()).expect("create file");
    file.write_all(contents.as_bytes()).expect("write file");
}

fn write_config(root: &Utf8Path, contents: &str) {
    fs::write(root.join("config.toml").as_std_path(), contents).unwrap();
}

#[test]
fn lists_prompts_from_base_and_conf_d() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    let config = format!(
        r#"
        prompt_path = "{library_dir}"

        [prompt.alpha]
        prompts = ["a.md"]
        "#
    );

    fs::write(root.join("config.toml").as_std_path(), config).unwrap();

    let conf_d = root.join("conf.d");
    fs::create_dir_all(conf_d.as_std_path()).unwrap();
    fs::write(
        conf_d.join("10-beta.toml").as_std_path(),
        "[prompt.beta]\nprompts = [\"b.md\"]\n",
    )
    .unwrap();

    write_file(&library_dir, "a.md", "Alpha\n");
    write_file(&library_dir, "b.md", "Beta\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");
    let names: Vec<_> = assembler.available_prompts().keys().cloned().collect();

    assert_eq!(names, vec!["alpha", "beta"]);
}

#[test]
fn renders_sequence_prompt_with_arguments() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    let config = format!(
        r#"
        prompt_path = "{library_dir}"

        [prompt.ticket]
        prompts = [
            "intro.md",
            "details.md"
        ]
        "#
    );
    fs::write(root.join("config.toml").as_std_path(), config).unwrap();

    write_file(&library_dir, "intro.md", "Ticket {0}\n");
    write_file(&library_dir, "details.md", "Details {{ {1} }}\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let rendered = assembler
        .render_prompt("ticket", &["ABC-123".into(), "Check logs".into()], None)
        .expect("render prompt");

    assert_eq!(rendered, "Ticket ABC-123\nDetails { Check logs }\n");
}

#[test]
fn renders_template_prompt_with_json_data() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    let config = format!(
        r#"
        prompt_path = "{library_dir}"

        [prompt.greeting]
        template = "greet.j2"
        "#
    );
    fs::write(root.join("config.toml").as_std_path(), config).unwrap();

    write_file(&library_dir, "greet.j2", "Hello {{ name }}!\n");

    let data_path = library_dir.join("data.json");
    fs::write(data_path.as_std_path(), r#"{"name": "World"}"#).unwrap();

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let rendered = assembler
        .render_prompt(
            "greeting",
            &[],
            Some(StructuredData::Json(data_path.clone())),
        )
        .expect("render template");

    assert_eq!(rendered, "Hello World!\n");
}

#[test]
fn renders_template_prompt_with_toml_data() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    let config = format!(
        r#"
        prompt_path = "{library_dir}"

        [prompt.system]
        template = "system.j2"
        "#
    );
    fs::write(root.join("config.toml").as_std_path(), config).unwrap();

    write_file(&library_dir, "system.j2", "Role: {{ role }}\n");

    let data_path = library_dir.join("data.toml");
    fs::write(data_path.as_std_path(), "role = \"admin\"\n").unwrap();

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let rendered = assembler
        .render_prompt("system", &[], Some(StructuredData::Toml(data_path.clone())))
        .expect("render template");

    assert_eq!(rendered, "Role: admin\n");
}

#[test]
fn fails_when_arguments_missing() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    let config = format!(
        r#"
        prompt_path = "{library_dir}"

        [prompt.partial]
        prompts = ["only.md"]
        "#
    );
    fs::write(root.join("config.toml").as_std_path(), config).unwrap();

    write_file(&library_dir, "only.md", "Value {0} and {1}\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let err = assembler
        .render_prompt("partial", &["one".into()], None)
        .expect_err("expected missing argument error");

    assert!(format!("{err}").contains("missing argument"));
}

#[test]
fn prompt_path_override_applies_per_prompt() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let shared_dir = root.join("shared");
    let overrides_dir = root.join("overrides");
    fs::create_dir_all(shared_dir.as_std_path()).unwrap();
    fs::create_dir_all(overrides_dir.as_std_path()).unwrap();

    let config = format!(
        r#"
        prompt_path = "{shared_dir}"

        [prompt.base]
        prompts = ["base.md"]

        [prompt.override]
        prompt_path = "{overrides_dir}"
        prompts = ["special.md"]
        "#
    );
    fs::write(root.join("config.toml").as_std_path(), config).unwrap();

    write_file(&shared_dir, "base.md", "BASE\n");
    write_file(&overrides_dir, "special.md", "OVERRIDE\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let base = assembler
        .render_prompt("base", &[], None)
        .expect("render base");
    let special = assembler
        .render_prompt("override", &[], None)
        .expect("render override");

    assert_eq!(base, "BASE\n");
    assert_eq!(special, "OVERRIDE\n");
}

#[test]
fn loads_without_prompt_definitions() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());

    let assembler = PromptAssembler::from_directory(root).expect("load assembler without prompts");

    assert!(!assembler.has_prompts());

    write_file(root, "standalone.md", "standalone content\n");

    let assembled_parts = assembler
        .assemble_parts(root, &["standalone.md".to_string()])
        .expect("assemble parts without prompt definitions");

    assert_eq!(assembled_parts, "standalone content\n");
}

#[test]
fn default_prompt_path_is_config_directory() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    fs::create_dir_all(root.as_std_path()).unwrap();

    write_config(
        root,
        r#"
        [prompt.default]
        prompts = ["default.md"]
        "#,
    );

    write_file(root, "default.md", "Default\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");
    let rendered = assembler
        .render_prompt("default", &[], None)
        .expect("render default prompt");

    assert_eq!(rendered, "Default\n");
}

#[test]
fn reports_missing_prompt() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    fs::write(
        root.join("config.toml").as_std_path(),
        format!(
            r#"prompt_path = "{library_dir}"
[prompt.alpha]
prompts = ["a.md"]
"#
        ),
    )
    .unwrap();
    write_file(&library_dir, "a.md", "A\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let err = assembler
        .render_prompt("missing", &[], None)
        .expect_err("prompt should be missing");

    assert!(format!("{err}").contains("unknown prompt"));
}

#[test]
fn config_errors_when_prompt_defines_sequence_and_template() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    write_config(
        root,
        r#"
        prompt_path = "~/.config/pa/"

        [prompt.invalid]
        prompts = ["a.md"]
        template = "bad.j2"
        "#,
    );

    let err = PromptAssembler::from_directory(root).expect_err("config should fail");
    let load_err = err.downcast::<LoadConfigError>().expect("load error");
    match load_err {
        LoadConfigError::Invalid { diagnostics } => {
            assert!(
                diagnostics
                    .errors
                    .iter()
                    .any(|issue| issue.message.contains("exclusive"))
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn config_errors_when_prompt_sequence_is_empty() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());

    write_config(
        root,
        r#"
        prompt_path = "~/.config/pa/"

        [prompt.empty]
        prompts = []
        "#,
    );

    let err = PromptAssembler::from_directory(root).expect_err("config should fail");
    let load_err = err.downcast::<LoadConfigError>().expect("load error");
    match load_err {
        LoadConfigError::Invalid { diagnostics } => {
            assert!(
                diagnostics
                    .errors
                    .iter()
                    .any(|issue| issue.message.contains("prompt sequence cannot be empty"))
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn later_conf_d_entries_override_base_definition() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let base_dir = root.join("library");
    fs::create_dir_all(base_dir.as_std_path()).unwrap();

    write_config(
        root,
        format!(
            r#"
            prompt_path = "{base_dir}"

            [prompt.note]
            prompts = ["base.md"]
            "#
        )
        .as_str(),
    );

    let conf_d = root.join("conf.d");
    fs::create_dir_all(conf_d.as_std_path()).unwrap();
    fs::write(
        conf_d.join("20-override.toml").as_std_path(),
        "[prompt.note]\ntemplate = \"note.j2\"\n",
    )
    .unwrap();

    write_file(&base_dir, "base.md", "Base\n");
    write_file(&base_dir, "note.j2", "Override {{ value }}\n");
    let data_path = base_dir.join("data.json");
    fs::write(data_path.as_std_path(), r#"{"value": "yes"}"#).unwrap();

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let rendered = assembler
        .render_prompt("note", &[], Some(StructuredData::Json(data_path)))
        .expect("render template");

    assert_eq!(rendered, "Override yes\n");
}

#[test]
fn config_errors_on_unknown_prompt_key() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());

    write_config(
        root,
        r#"
        prompt_path = "~/.config/pa/"

        [prompt.alpha]
        prompts = ["alpha.md"]
        unexpected = true
        "#,
    );

    let err = PromptAssembler::from_directory(root).expect_err("unknown key should fail");
    let load_err = err.downcast::<LoadConfigError>().expect("load error");
    match load_err {
        LoadConfigError::Invalid { diagnostics } => {
            assert!(
                diagnostics
                    .errors
                    .iter()
                    .any(|issue| issue.message.contains("unexpected"))
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn errors_on_non_sequential_placeholder_index() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    write_config(
        root,
        format!(
            r#"
            prompt_path = "{library_dir}"

            [prompt.skip]
            prompts = ["skip.md"]
            "#
        )
        .as_str(),
    );
    write_file(&library_dir, "skip.md", "First {0}, third {2}\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let err = assembler
        .render_prompt("skip", &["one".into()], None)
        .expect_err("missing {1} should error");

    assert!(format!("{err}").contains("placeholder"));
}

#[test]
fn errors_on_placeholder_index_above_nine() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    write_config(
        root,
        format!(
            r#"
            prompt_path = "{library_dir}"

            [prompt.ten]
            prompts = ["ten.md"]
            "#
        )
        .as_str(),
    );
    write_file(&library_dir, "ten.md", "Value {10}\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let err = assembler
        .render_prompt("ten", &["one".into()], None)
        .expect_err("placeholder above nine should fail");

    assert!(format!("{err}").contains("up to 9"));
}

#[test]
fn errors_when_prompt_fragment_missing() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    write_config(
        root,
        format!(
            r#"
            prompt_path = "{library_dir}"

            [prompt.missing]
            prompts = ["missing.md"]
            "#
        )
        .as_str(),
    );

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");
    let err = assembler
        .render_prompt("missing", &[], None)
        .expect_err("missing file should error");

    assert!(format!("{err}").contains("missing.md"));
}

#[test]
fn errors_when_data_file_missing() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    write_config(
        root,
        format!(
            r#"
            prompt_path = "{library_dir}"

            [prompt.template]
            template = "tpl.j2"
            "#
        )
        .as_str(),
    );
    write_file(&library_dir, "tpl.j2", "{{ value }}\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");
    let data_path = library_dir.join("missing.json");

    let err = assembler
        .render_prompt("template", &[], Some(StructuredData::Json(data_path)))
        .expect_err("missing data file should error");

    assert!(format!("{err}").contains("missing.json"));
}

#[test]
fn errors_when_data_given_for_sequence_prompt() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    write_config(
        root,
        format!(
            r#"
            prompt_path = "{library_dir}"

            [prompt.sequence]
            prompts = ["seq.md"]
            "#
        )
        .as_str(),
    );
    write_file(&library_dir, "seq.md", "Only text\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");
    let data_path = library_dir.join("vars.json");
    fs::write(data_path.as_std_path(), "{}").unwrap();

    let err = assembler
        .render_prompt(
            "sequence",
            &[],
            Some(StructuredData::Json(data_path.clone())),
        )
        .expect_err("sequence prompt should reject data");

    assert!(format!("{err}").contains("does not accept structured data"));
}

#[test]
fn errors_when_template_prompt_missing_data() {
    let temp = TempDir::new().unwrap();
    let root = utf8_path(temp.path());
    let library_dir = root.join("library");
    fs::create_dir_all(library_dir.as_std_path()).unwrap();

    write_config(
        root,
        format!(
            r#"
            prompt_path = "{library_dir}"

            [prompt.template]
            template = "should-need-data.j2"
            "#
        )
        .as_str(),
    );
    write_file(&library_dir, "should-need-data.j2", "{{ value }}\n");

    let assembler = PromptAssembler::from_directory(root).expect("load assembler");

    let err = assembler
        .render_prompt("template", &[], None)
        .expect_err("template without data should error");

    assert!(format!("{err}").contains("data file"));
}
