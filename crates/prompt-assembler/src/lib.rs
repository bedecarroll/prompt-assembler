use std::collections::BTreeMap;
use std::fs;
use std::io::Read;

use anyhow::{Context, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use directories::BaseDirs;
use indexmap::IndexMap;
use minijinja::Environment;
use serde::Deserialize;

pub type Result<T> = std::result::Result<T, anyhow::Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub root: Utf8PathBuf,
    pub default_prompt_path: Option<Utf8PathBuf>,
    pub prompts: IndexMap<String, PromptSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSpec {
    pub prompt_path_override: Option<Utf8PathBuf>,
    pub kind: PromptKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    Sequence { files: Vec<Utf8PathBuf> },
    Template { template: Utf8PathBuf },
}

#[derive(Debug, Clone)]
pub struct PromptAssembler {
    config: Config,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredData {
    Json(Utf8PathBuf),
    Toml(Utf8PathBuf),
}

impl StructuredData {
    fn path(&self) -> &Utf8Path {
        match self {
            StructuredData::Json(path) | StructuredData::Toml(path) => path.as_ref(),
        }
    }
}

impl PromptAssembler {
    /// Construct an assembler by loading configuration from `dir`.
    ///
    /// # Errors
    /// Returns an error if configuration files are missing, unreadable, or invalid.
    pub fn from_directory(dir: &Utf8Path) -> Result<Self> {
        let config = load_config(dir)?;
        Ok(Self { config })
    }

    /// Assemble the prompt identified by `name` using provided arguments and optional data.
    ///
    /// # Errors
    /// Returns an error when the prompt is unknown, configuration is incomplete, or
    /// required files and data cannot be read or parsed.
    pub fn render_prompt(
        &self,
        name: &str,
        args: &[String],
        data: Option<StructuredData>,
    ) -> Result<String> {
        let spec = self
            .config
            .prompts
            .get(name)
            .ok_or_else(|| anyhow!("unknown prompt: {name}"))?;

        match &spec.kind {
            PromptKind::Sequence { files } => {
                if data.is_some() {
                    bail!("prompt '{name}' does not accept structured data");
                }

                let base = self
                    .resolve_prompt_path(spec)
                    .context("sequence prompt missing prompt_path")?;

                let mut rendered = String::new();
                for file in files {
                    let full_path = base.join(file);
                    let content = read_utf8(&full_path).with_context(|| {
                        format!("failed to read fragment '{file}' for prompt '{name}'")
                    })?;
                    let substituted = substitute_placeholders(&content, args)?;
                    rendered.push_str(&substituted);
                }
                Ok(rendered)
            }
            PromptKind::Template { template } => {
                let data = data.ok_or_else(|| {
                    anyhow!("prompt '{name}' requires a data file for structured context")
                })?;

                let base = self
                    .resolve_prompt_path(spec)
                    .context("template prompt missing prompt_path")?;

                render_template(name, &base, template, &data, args)
            }
        }
    }

    #[must_use]
    pub fn available_prompts(&self) -> BTreeMap<String, PromptKind> {
        self.config
            .prompts
            .iter()
            .map(|(name, spec)| (name.clone(), spec.kind.clone()))
            .collect()
    }

    #[must_use]
    pub fn has_prompts(&self) -> bool {
        !self.config.prompts.is_empty()
    }

    fn resolve_prompt_path(&self, spec: &PromptSpec) -> Option<Utf8PathBuf> {
        spec.prompt_path_override
            .clone()
            .or_else(|| self.config.default_prompt_path.clone())
    }

    #[must_use]
    pub fn prompt_kind(&self, name: &str) -> Option<&PromptKind> {
        self.config.prompts.get(name).map(|spec| &spec.kind)
    }

    /// Assemble a sequence of raw prompt parts by name without placeholder substitution.
    ///
    /// # Errors
    /// Returns an error when a part cannot be located or read.
    pub fn assemble_parts(&self, working_dir: &Utf8Path, part_names: &[String]) -> Result<String> {
        if part_names.is_empty() {
            bail!("no parts provided");
        }

        let mut output = String::new();
        for name in part_names {
            let resolved = self.resolve_part_path(working_dir, name)?;
            let contents = read_utf8(resolved.as_path())
                .with_context(|| format!("failed to read part '{name}' at {resolved}"))?;
            output.push_str(&contents);
        }

        Ok(output)
    }

    fn resolve_part_path(&self, working_dir: &Utf8Path, raw: &str) -> Result<Utf8PathBuf> {
        let candidate = Utf8PathBuf::from(raw);

        if candidate.is_absolute() {
            if candidate.exists() {
                return Ok(candidate);
            }
            bail!("missing part '{raw}'");
        }

        let cwd_candidate = working_dir.join(&candidate);
        if cwd_candidate.exists() {
            return Ok(cwd_candidate);
        }

        if let Some(base) = &self.config.default_prompt_path {
            let prompt_candidate = base.join(&candidate);
            if prompt_candidate.exists() {
                return Ok(prompt_candidate);
            }
        }

        bail!("missing part '{raw}'")
    }
}

fn load_config(root: &Utf8Path) -> Result<Config> {
    let mut prompts: IndexMap<String, PromptSpec> = IndexMap::new();
    let mut default_prompt_path: Option<Utf8PathBuf> = Some(root.to_owned());

    // Load primary config
    let main_config = root.join("config.toml");
    if main_config.exists() {
        let bytes = read_utf8(&main_config)?;
        merge_config(root, &bytes, &mut prompts, &mut default_prompt_path)?;
    }

    // Load conf.d entries in lexical order
    let conf_d = root.join("conf.d");
    if conf_d.exists() {
        let mut entries: Vec<Utf8PathBuf> = fs::read_dir(conf_d.as_std_path())
            .with_context(|| format!("failed to read {conf_d}"))?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "toml") {
                    Some(path)
                } else {
                    None
                }
            })
            .map(|path| {
                Utf8PathBuf::from_path_buf(path)
                    .map_err(|_| anyhow!("configuration paths must be valid UTF-8"))
            })
            .collect::<Result<_>>()?;

        entries.sort();

        for entry in entries {
            let content = read_utf8(&entry)?;
            merge_config(root, &content, &mut prompts, &mut default_prompt_path)?;
        }
    }

    Ok(Config {
        root: root.to_owned(),
        default_prompt_path,
        prompts,
    })
}

fn merge_config(
    root: &Utf8Path,
    raw: &str,
    prompts: &mut IndexMap<String, PromptSpec>,
    default_prompt_path: &mut Option<Utf8PathBuf>,
) -> Result<()> {
    let file: RawFile = toml::from_str(raw)?;

    if let Some(path) = file.prompt_path {
        let resolved = resolve_path(root, &path)?;
        *default_prompt_path = Some(resolved);
    }

    for (name, prompt) in file.prompt {
        let prompt_spec = build_prompt_spec(root, prompt)?;
        prompts.insert(name, prompt_spec);
    }

    Ok(())
}

fn build_prompt_spec(root: &Utf8Path, prompt: RawPrompt) -> Result<PromptSpec> {
    let prompt_path = match prompt.prompt_path {
        Some(path) => Some(resolve_path(root, &path)?),
        None => None,
    };

    match (prompt.prompts, prompt.template) {
        (Some(files), None) => {
            if files.is_empty() {
                bail!("prompt sequence cannot be empty");
            }
            let resolved_files = files.into_iter().map(Utf8PathBuf::from).collect();

            Ok(PromptSpec {
                prompt_path_override: prompt_path,
                kind: PromptKind::Sequence {
                    files: resolved_files,
                },
            })
        }
        (None, Some(template)) => Ok(PromptSpec {
            prompt_path_override: prompt_path,
            kind: PromptKind::Template {
                template: Utf8PathBuf::from(template),
            },
        }),
        (Some(_), Some(_)) => bail!("prompts and template are exclusive options"),
        (None, None) => bail!("prompt must define either 'prompts' or 'template'"),
    }
}

fn resolve_path(root: &Utf8Path, path: &str) -> Result<Utf8PathBuf> {
    if let Some(stripped) = path.strip_prefix("~/") {
        let base_dirs =
            BaseDirs::new().ok_or_else(|| anyhow!("cannot resolve '~' without home directory"))?;
        let mut buf = Utf8PathBuf::from_path_buf(base_dirs.home_dir().to_path_buf())
            .map_err(|_| anyhow!("home directory is not valid UTF-8"))?;
        buf.push(stripped);
        Ok(buf)
    } else {
        let candidate = Utf8PathBuf::from(path);
        if candidate.is_absolute() {
            Ok(candidate)
        } else {
            Ok(root.join(candidate))
        }
    }
}

fn read_utf8(path: &Utf8Path) -> Result<String> {
    let mut file =
        fs::File::open(path.as_std_path()).with_context(|| format!("failed to open {path}"))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .with_context(|| format!("failed to read {path}"))?;
    Ok(buf)
}

fn substitute_placeholders(template: &str, args: &[String]) -> Result<String> {
    let mut output = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '{' => match chars.peek() {
                Some('{') => {
                    chars.next();
                    output.push('{');
                }
                Some(_) => {
                    let mut digits = String::new();
                    while let Some(peek) = chars.peek() {
                        if peek.is_ascii_digit() {
                            digits.push(*peek);
                            chars.next();
                        } else {
                            break;
                        }
                    }

                    if digits.is_empty() {
                        bail!("empty placeholder braces are not allowed");
                    }

                    let index = digits
                        .parse::<usize>()
                        .map_err(|_| anyhow!("invalid placeholder index '{digits}'"))?;

                    match chars.next() {
                        Some('}') => {}
                        _ => bail!("unterminated placeholder '{{{digits}'"),
                    }

                    if index > 9 {
                        bail!("positional placeholders support up to 9 arguments");
                    }
                    let value = args
                        .get(index)
                        .ok_or_else(|| anyhow!("missing argument for placeholder {{{index}}}"))?;
                    output.push_str(value);
                }
                None => bail!("unterminated placeholder at end of template"),
            },
            '}' => match chars.peek() {
                Some('}') => {
                    chars.next();
                    output.push('}');
                }
                _ => bail!("unmatched closing brace '}}'"),
            },
            other => output.push(other),
        }
    }

    Ok(output)
}

fn render_template(
    prompt_name: &str,
    base: &Utf8Path,
    template: &Utf8Path,
    data: &StructuredData,
    args: &[String],
) -> Result<String> {
    let mut env = Environment::new();
    env.set_keep_trailing_newline(true);
    env.set_loader(minijinja::path_loader(base.as_std_path()));

    let template_name = template.as_str();
    let template_ref = env
        .get_template(template_name)
        .with_context(|| format!("prompt '{prompt_name}' template '{template}' not found"))?;

    let data_path = data.path();
    let data_value = load_structured_data(data).with_context(|| {
        format!("failed to load data file {data_path} for prompt '{prompt_name}'")
    })?;
    let mut map = match data_value {
        serde_json::Value::Object(obj) => obj,
        other => {
            let mut obj = serde_json::Map::new();
            obj.insert("value".into(), other);
            obj
        }
    };

    if !args.is_empty() {
        let positional = serde_json::Value::Array(
            args.iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        );
        map.insert("_args".into(), positional);
    }

    let context_value = serde_json::Value::Object(map);
    let rendered = template_ref
        .render(minijinja::value::Value::from_serialize(&context_value))
        .with_context(|| {
            format!("rendering template '{template_name}' for prompt '{prompt_name}'")
        })?;
    Ok(rendered)
}

fn load_structured_data(data: &StructuredData) -> Result<serde_json::Value> {
    match data {
        StructuredData::Json(path) => {
            let content = read_utf8(path)?;
            Ok(serde_json::from_str(&content)
                .with_context(|| format!("failed to parse JSON data from {path}"))?)
        }
        StructuredData::Toml(path) => {
            let content = read_utf8(path)?;
            let toml_value: toml::Value = toml::from_str(&content)
                .with_context(|| format!("failed to parse TOML data from {path}"))?;
            serde_json::to_value(toml_value)
                .map_err(|err| anyhow!("failed to convert TOML to JSON: {err}"))
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    prompt_path: Option<String>,
    #[serde(default)]
    prompt: IndexMap<String, RawPrompt>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrompt {
    #[serde(default)]
    prompt_path: Option<String>,
    #[serde(default)]
    prompts: Option<Vec<String>>,
    #[serde(default)]
    template: Option<String>,
}
