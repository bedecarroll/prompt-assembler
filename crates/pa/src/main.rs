use std::io::{self, Write};

use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use directories::BaseDirs;
use prompt_assembler::{PromptAssembler, PromptKind, StructuredData};

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

#[derive(Subcommand, Debug)]
enum Commands {
    /// List available prompts
    List,
    /// Generate shell completions
    Completions { shell: String },
    /// Concatenate raw prompt parts without placeholder substitution
    Parts {
        #[arg(value_name = "FILE", num_args = 1..)]
        files: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let config_dir = discover_config_dir()?;
    let assembler = PromptAssembler::from_directory(config_dir.as_ref())
        .with_context(|| format!("failed to load configuration from {config_dir}"))?;

    match cli.command {
        Some(Commands::List) => {
            ensure_prompts_available(&assembler)?;
            list_prompts(&assembler);
        }
        Some(Commands::Completions { shell }) => {
            ensure_prompts_available(&assembler)?;
            let shell = parse_shell(&shell)?;
            generate_completions(shell, &assembler)?;
        }
        Some(Commands::Parts { files }) => {
            run_parts(&assembler, &files)?;
        }
        None => {
            ensure_prompts_available(&assembler)?;
            let prompt = cli
                .prompt
                .ok_or_else(|| anyhow!("prompt name is required"))?;
            run_prompt(&assembler, &prompt, cli.prompt_args)?;
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
