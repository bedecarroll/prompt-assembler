use std::fs;
use std::io::{self, Write};
use std::process;
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use directories::BaseDirs;
use prompt_assembler::{
    ConfigIssue, LoadConfigError, PromptAssembler, PromptKind, PromptPart, PromptProfile,
    PromptSpec, PromptVariable, StructuredData,
};
use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SCHEMA_VERSION: u8 = 1;
const DEFAULT_CONFIG: &[u8] = include_bytes!("../../../assets/default_config.toml");

#[derive(Parser, Debug)]
#[command(
    name = "pa",
    version,
    about = "Assemble prompt snippets from your prompt library",
    arg_required_else_help = true,
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,
    #[arg(value_name = "ARG", trailing_var_arg = true)]
    prompt_args: Vec<String>,
}

#[derive(Args, Debug, Clone)]
struct ListArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct ShowArgs {
    #[arg(value_name = "PROMPT")]
    name: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct ValidateArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug, Clone)]
struct SelfUpdateArgs {
    #[arg(long, value_name = "TAG")]
    version: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List available prompts
    List(ListArgs),
    /// Show prompt metadata
    Show(ShowArgs),
    /// Validate configuration files
    Validate(ValidateArgs),
    /// Update pa to the latest released version
    SelfUpdate(SelfUpdateArgs),
    /// Generate shell completions
    Completions { shell: String },
    /// Concatenate raw prompt parts without placeholder substitution
    Parts {
        #[arg(value_name = "FILE", num_args = 1..)]
        files: Vec<String>,
    },
}

fn main() -> Result<()> {
    let Cli {
        command,
        prompt,
        prompt_args,
    } = Cli::parse();

    let config_dir = discover_config_dir()?;
    ensure_config_initialized(config_dir.as_ref())?;

    match command {
        Some(Commands::List(args)) => {
            handle_list(config_dir.as_ref(), &args)?;
        }
        Some(Commands::Show(args)) => {
            handle_show(config_dir.as_ref(), &args)?;
        }
        Some(Commands::Validate(args)) => {
            handle_validate(config_dir.as_ref(), &args)?;
        }
        Some(Commands::SelfUpdate(args)) => {
            handle_self_update(&args)?;
        }
        Some(Commands::Completions { shell }) => {
            let assembler = load_runtime_assembler(config_dir.as_ref())?;
            ensure_prompts_available(&assembler)?;
            let shell = parse_shell(&shell)?;
            generate_completions(shell, &assembler)?;
        }
        Some(Commands::Parts { files }) => {
            let assembler = load_runtime_assembler(config_dir.as_ref())?;
            run_parts(&assembler, &files)?;
        }
        None => {
            let assembler = load_runtime_assembler(config_dir.as_ref())?;
            ensure_prompts_available(&assembler)?;
            let prompt = prompt.ok_or_else(|| anyhow!("prompt name is required"))?;
            run_prompt(&assembler, &prompt, prompt_args)?;
        }
    }

    Ok(())
}

fn run_prompt(assembler: &PromptAssembler, prompt: &str, args: Vec<String>) -> Result<()> {
    let kind = assembler
        .prompt_kind(prompt)
        .ok_or_else(|| anyhow!("unknown prompt: {prompt}"))?;

    let stdin_arg = read_stdin_if_available()?;

    let output = match kind {
        PromptKind::Sequence { .. } => {
            let mut positional_args = args;
            if let Some(ref input) = stdin_arg {
                positional_args.insert(0, input.clone());
            }

            if positional_args
                .first()
                .is_some_and(|first| looks_like_data_file(first))
            {
                bail!("prompt '{prompt}' does not accept structured data");
            }
            assembler.render_prompt(prompt, &positional_args, None)?
        }
        PromptKind::Template { .. } => {
            let mut iter = args.into_iter();
            let data_arg = iter
                .next()
                .ok_or_else(|| anyhow!("prompt '{prompt}' requires a data file (JSON or TOML)"))?;
            let data = parse_data_argument(&data_arg)?;
            let mut remaining: Vec<String> = iter.collect();
            if let Some(ref input) = stdin_arg {
                remaining.insert(0, input.clone());
            }
            assembler.render_prompt(prompt, &remaining, Some(data))?
        }
    };

    print!("{output}");
    Ok(())
}

fn run_parts(assembler: &PromptAssembler, files: &[String]) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let cwd = Utf8PathBuf::from_path_buf(cwd)
        .map_err(|_| anyhow!("current directory is not valid UTF-8"))?;

    let output = assembler.assemble_parts(cwd.as_ref(), files)?;
    print!("{output}");
    Ok(())
}

fn load_runtime_assembler(config_dir: &Utf8Path) -> Result<PromptAssembler> {
    PromptAssembler::from_directory(config_dir)
        .with_context(|| format!("failed to load configuration from {config_dir}"))
}

fn ensure_prompts_available(assembler: &PromptAssembler) -> Result<()> {
    if assembler.has_prompts() {
        Ok(())
    } else {
        bail!("no prompts defined; ensure config.toml exists with prompt entries");
    }
}

fn list_prompts(assembler: &PromptAssembler) {
    for name in assembler.available_prompts().keys() {
        println!("{name}");
    }
}

fn handle_list(config_dir: &Utf8Path, args: &ListArgs) -> Result<()> {
    match PromptAssembler::load_with_diagnostics(config_dir) {
        Ok(assembler) => {
            if args.json {
                print_list_json(&assembler)?;
            } else {
                ensure_prompts_available(&assembler)?;
                list_prompts(&assembler);
            }
        }
        Err(LoadConfigError::Invalid { diagnostics }) => {
            emit_human_diagnostics("error", &diagnostics.errors);
            emit_human_diagnostics("warning", &diagnostics.warnings);
            process::exit(2);
        }
        Err(other) => exit_with_load_error(other),
    }

    Ok(())
}

fn handle_show(config_dir: &Utf8Path, args: &ShowArgs) -> Result<()> {
    match PromptAssembler::load_with_diagnostics(config_dir) {
        Ok(assembler) => {
            let Some(spec) = assembler.prompt_spec(&args.name) else {
                eprintln!("error: unknown prompt '{}'", args.name);
                process::exit(1);
            };

            if args.json {
                let profile = assembler.prompt_profile(&args.name)?;
                let profile = Some(profile_to_json(profile));
                print_prompt_json(&args.name, spec, profile)?;
            } else {
                print_prompt_human(&args.name, spec);
            }
        }
        Err(LoadConfigError::Invalid { diagnostics }) => {
            emit_human_diagnostics("error", &diagnostics.errors);
            emit_human_diagnostics("warning", &diagnostics.warnings);
            process::exit(2);
        }
        Err(other) => exit_with_load_error(other),
    }

    Ok(())
}

fn handle_validate(config_dir: &Utf8Path, args: &ValidateArgs) -> Result<()> {
    match PromptAssembler::load_with_diagnostics(config_dir) {
        Ok(assembler) => {
            let warnings: Vec<ConfigIssue> = assembler.config_warnings().to_vec();
            if args.json {
                print_validate_json(&[], &warnings)?;
            } else {
                if !warnings.is_empty() {
                    emit_human_diagnostics("warning", &warnings);
                }
                println!("configuration is valid");
            }
        }
        Err(LoadConfigError::Invalid { diagnostics }) => {
            if args.json {
                print_validate_json(&diagnostics.errors, &diagnostics.warnings)?;
            } else {
                emit_human_diagnostics("error", &diagnostics.errors);
                emit_human_diagnostics("warning", &diagnostics.warnings);
            }
            process::exit(2);
        }
        Err(other) => exit_with_load_error(other),
    }

    Ok(())
}

fn handle_self_update(args: &SelfUpdateArgs) -> Result<()> {
    use self_update::backends::github::Update;

    const REPO_OWNER: &str = "bedecarroll";
    const REPO_NAME: &str = "prompt-assembler";
    const BIN_NAME: &str = "pa";

    let mut builder = Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(env!("CARGO_PKG_VERSION"))
        .show_download_progress(true);

    if let Some(version) = args.version.as_ref() {
        let normalized = if version.starts_with('v') {
            version.clone()
        } else {
            format!("v{version}")
        };
        builder.target_version_tag(&normalized);
    }

    if let Ok(token) = std::env::var("PA_GITHUB_TOKEN") {
        if !token.trim().is_empty() {
            builder.auth_token(token.trim());
        }
    }

    let status = builder
        .build()
        .context("failed to configure self-updater")?
        .update()
        .context("failed to apply update")?;

    if status.updated() {
        println!("Updated pa to {}", status.version());
    } else {
        println!("pa is already up to date ({})", status.version());
    }

    Ok(())
}

fn generate_completions(shell: Shell, assembler: &PromptAssembler) -> Result<()> {
    let mut cmd = Cli::command();
    let mut buffer = Vec::new();
    generate(shell, &mut cmd, "pa", &mut buffer);

    let prompts: Vec<String> = assembler.available_prompts().keys().cloned().collect();

    let mut stdout = io::stdout();
    stdout.write_all(&buffer)?;

    if !prompts.is_empty() {
        match shell {
            Shell::Bash | Shell::Zsh | Shell::Fish => {
                writeln!(
                    stdout,
                    "\n# prompt-assembler dynamic prompt list\n_pa_prompt_list=\"{}\"",
                    prompts.join(" ")
                )?;
            }
            _ => {
                writeln!(stdout, "\n# prompts: {}", prompts.join(" "))?;
            }
        }
    }

    Ok(())
}

fn print_list_json(assembler: &PromptAssembler) -> Result<()> {
    let prompts: Vec<JsonPrompt> = assembler
        .prompt_specs()
        .iter()
        .map(|(name, spec)| prompt_to_json(name, spec, None))
        .collect();

    let payload = ListEnvelope {
        schema_version: SCHEMA_VERSION,
        generated_at: current_timestamp(),
        prompts,
    };

    let rendered = serde_json::to_string_pretty(&payload)?;
    println!("{rendered}");
    Ok(())
}

fn print_prompt_json(
    name: &str,
    spec: &PromptSpec,
    profile: Option<JsonPromptProfile>,
) -> Result<()> {
    let payload = prompt_to_json(name, spec, profile);
    let rendered = serde_json::to_string_pretty(&payload)?;
    println!("{rendered}");
    Ok(())
}

fn print_prompt_human(name: &str, spec: &PromptSpec) {
    println!("name: {name}");

    match spec.kind {
        PromptKind::Sequence { .. } => println!("kind: sequence"),
        PromptKind::Template { .. } => println!("kind: template"),
    }

    if let Some(description) = &spec.metadata.description {
        println!("description: {description}");
    }

    if !spec.metadata.tags.is_empty() {
        println!("tags: {}", spec.metadata.tags.join(", "));
    }

    println!(
        "stdin supported: {}",
        if effective_stdin_supported(spec) {
            "yes"
        } else {
            "no"
        }
    );

    if let Some(last_modified) = format_system_time(spec.metadata.source.last_modified) {
        println!("last modified: {last_modified}");
    }

    println!("source: {}", spec.metadata.source.path);

    if !spec.metadata.vars.is_empty() {
        println!("vars:");
        for var in &spec.metadata.vars {
            let mut details = format!("  - {} ({})", var.name, var.kind.as_str());
            if var.required {
                details.push_str(" [required]");
            }
            if let Some(description) = &var.description {
                details.push_str(" — ");
                details.push_str(description);
            }
            println!("{details}");
        }
    }
}

fn print_validate_json(errors: &[ConfigIssue], warnings: &[ConfigIssue]) -> Result<()> {
    let payload = ValidateEnvelope {
        schema_version: SCHEMA_VERSION,
        generated_at: current_timestamp(),
        errors: errors.iter().map(JsonDiagnostic::from).collect(),
        warnings: warnings.iter().map(JsonDiagnostic::from).collect(),
    };

    let rendered = serde_json::to_string_pretty(&payload)?;
    println!("{rendered}");
    Ok(())
}

fn prompt_to_json(name: &str, spec: &PromptSpec, profile: Option<JsonPromptProfile>) -> JsonPrompt {
    JsonPrompt {
        name: name.to_string(),
        description: spec.metadata.description.clone(),
        tags: spec.metadata.tags.clone(),
        vars: convert_vars(&spec.metadata.vars),
        stdin_supported: effective_stdin_supported(spec),
        last_modified: format_system_time(spec.metadata.source.last_modified),
        source_path: spec.metadata.source.path.as_str().to_owned(),
        profile,
    }
}

fn convert_vars(vars: &[PromptVariable]) -> Vec<JsonPromptVar> {
    vars.iter()
        .map(|var| JsonPromptVar {
            name: var.name.clone(),
            required: var.required,
            kind: var.kind.as_str().to_owned(),
            description: var.description.clone(),
        })
        .collect()
}

fn profile_to_json(profile: PromptProfile) -> JsonPromptProfile {
    match profile {
        PromptProfile::Sequence { parts, combined } => JsonPromptProfile {
            kind: "sequence".to_string(),
            parts: parts.into_iter().map(JsonPromptPart::from).collect(),
            template: None,
            content: combined,
        },
        PromptProfile::Template { template } => {
            let part = JsonPromptPart::from(template);
            let content = part.content.clone();
            JsonPromptProfile {
                kind: "template".to_string(),
                parts: Vec::new(),
                template: Some(part),
                content,
            }
        }
    }
}

impl From<PromptPart> for JsonPromptPart {
    fn from(part: PromptPart) -> Self {
        Self {
            path: part.path.into_string(),
            content: part.content,
        }
    }
}

fn emit_human_diagnostics(level: &str, issues: &[ConfigIssue]) {
    for issue in issues {
        let detail = format_issue(issue);
        eprintln!("{level}: {detail} ({})", issue.code.as_str());
    }
}

fn format_issue(issue: &ConfigIssue) -> String {
    match issue.line {
        Some(line) => format!("{}:{}: {}", issue.path, line, issue.message),
        None => format!("{}: {}", issue.path, issue.message),
    }
}

fn exit_with_load_error(err: LoadConfigError) -> ! {
    match err {
        LoadConfigError::Io { path, source } => {
            eprintln!("error: failed to read {path}: {source}");
            process::exit(127);
        }
        LoadConfigError::ReadDir { path, source } => {
            eprintln!("error: failed to enumerate {path}: {source}");
            process::exit(127);
        }
        LoadConfigError::Invalid { diagnostics } => {
            emit_human_diagnostics("error", &diagnostics.errors);
            emit_human_diagnostics("warning", &diagnostics.warnings);
            process::exit(2);
        }
    }
}

fn effective_stdin_supported(spec: &PromptSpec) -> bool {
    spec.metadata
        .stdin_supported
        .unwrap_or(matches!(spec.kind, PromptKind::Sequence { .. }))
}

fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn format_system_time(value: Option<SystemTime>) -> Option<String> {
    value.and_then(|time| OffsetDateTime::from(time).format(&Rfc3339).ok())
}

#[derive(Serialize)]
struct ListEnvelope {
    schema_version: u8,
    generated_at: String,
    prompts: Vec<JsonPrompt>,
}

#[derive(Serialize)]
struct JsonPrompt {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    vars: Vec<JsonPromptVar>,
    stdin_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
    source_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<JsonPromptProfile>,
}

#[derive(Serialize)]
struct JsonPromptProfile {
    kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parts: Vec<JsonPromptPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template: Option<JsonPromptPart>,
    content: String,
}

#[derive(Serialize, Clone)]
struct JsonPromptPart {
    path: String,
    content: String,
}

#[derive(Serialize)]
struct JsonPromptVar {
    name: String,
    required: bool,
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Serialize)]
struct ValidateEnvelope {
    schema_version: u8,
    generated_at: String,
    errors: Vec<JsonDiagnostic>,
    warnings: Vec<JsonDiagnostic>,
}

#[derive(Serialize)]
struct JsonDiagnostic {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u32>,
    code: String,
    message: String,
}

impl From<&ConfigIssue> for JsonDiagnostic {
    fn from(issue: &ConfigIssue) -> Self {
        Self {
            file: issue.path.as_str().to_owned(),
            line: issue.line,
            code: issue.code.as_str().to_owned(),
            message: issue.message.clone(),
        }
    }
}

fn parse_data_argument(raw: &str) -> Result<StructuredData> {
    if !looks_like_data_file(raw) {
        bail!("data file must use JSON or TOML format");
    }
    let path = Utf8PathBuf::from(raw);
    match path.extension().map(str::to_ascii_lowercase).as_deref() {
        Some("json") => Ok(StructuredData::Json(path)),
        Some("toml") => Ok(StructuredData::Toml(path)),
        _ => bail!("data file must use JSON or TOML format"),
    }
}

fn discover_config_dir() -> Result<Utf8PathBuf> {
    if let Ok(xdg_home) = std::env::var("XDG_CONFIG_HOME") {
        let base = Utf8PathBuf::from(xdg_home);
        return Ok(base.join("prompt-assembler"));
    }

    let base_dirs =
        BaseDirs::new().ok_or_else(|| anyhow!("unable to locate XDG config directory"))?;
    let path = base_dirs.config_dir().join("prompt-assembler");
    Utf8PathBuf::from_path_buf(path).map_err(|_| anyhow!("config path is not valid UTF-8"))
}

fn ensure_config_initialized(config_dir: &Utf8Path) -> Result<()> {
    fs::create_dir_all(config_dir.as_std_path())
        .with_context(|| format!("failed to create config directory {config_dir}"))?;

    let config_path = config_dir.join("config.toml");
    if !config_path.exists() {
        fs::write(config_path.as_std_path(), DEFAULT_CONFIG)
            .with_context(|| format!("failed to write default config at {config_path}"))?;
    }

    Ok(())
}

fn looks_like_data_file(value: &str) -> bool {
    Utf8Path::new(value)
        .extension()
        .map(str::to_ascii_lowercase)
        .as_deref()
        .is_some_and(|ext| ext == "json" || ext == "toml")
}

fn parse_shell(raw: &str) -> Result<Shell> {
    let normalized = raw.to_ascii_lowercase();
    normalized
        .parse::<Shell>()
        .map_err(|_| anyhow!("unsupported shell '{raw}'"))
}

fn read_stdin_if_available() -> Result<Option<String>> {
    use std::io::Read;

    let stdin = io::stdin();
    let mut handle = stdin.lock();

    if atty::is(atty::Stream::Stdin) {
        return Ok(None);
    }

    let mut buffer = String::new();
    handle.read_to_string(&mut buffer)?;

    if buffer.is_empty() {
        Ok(None)
    } else {
        if buffer.ends_with('\n') {
            buffer.pop();
            if buffer.ends_with('\r') {
                buffer.pop();
            }
        }
        Ok(Some(buffer))
    }
}
