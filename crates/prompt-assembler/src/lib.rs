use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::time::SystemTime;

use anyhow::{Context, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use directories::BaseDirs;
use indexmap::IndexMap;
use minijinja::Environment;
use serde::Deserialize;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, anyhow::Error>;

#[derive(Debug, Clone)]
pub struct Config {
    pub root: Utf8PathBuf,
    pub default_prompt_path: Option<Utf8PathBuf>,
    pub prompts: IndexMap<String, PromptSpec>,
}

#[derive(Debug, Clone)]
pub struct PromptSpec {
    pub prompt_path_override: Option<Utf8PathBuf>,
    pub kind: PromptKind,
    pub metadata: PromptMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    Sequence { files: Vec<Utf8PathBuf> },
    Template { template: Utf8PathBuf },
}

#[derive(Debug, Clone)]
pub struct PromptMetadata {
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub vars: Vec<PromptVariable>,
    pub stdin_supported: Option<bool>,
    pub source: PromptSource,
}

#[derive(Debug, Clone)]
pub struct PromptSource {
    pub path: Utf8PathBuf,
    pub last_modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptVariable {
    pub name: String,
    pub required: bool,
    pub kind: PromptVariableKind,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptVariableKind {
    String,
    Path,
    Number,
    Boolean,
}

impl PromptVariableKind {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            PromptVariableKind::String => "string",
            PromptVariableKind::Path => "path",
            PromptVariableKind::Number => "number",
            PromptVariableKind::Boolean => "boolean",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigIssueCode {
    DuplicateVar,
    Override,
    InvalidPrompt,
    ParseError,
}

impl ConfigIssueCode {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigIssueCode::DuplicateVar => "duplicate_var",
            ConfigIssueCode::Override => "override",
            ConfigIssueCode::InvalidPrompt => "invalid_prompt",
            ConfigIssueCode::ParseError => "parse_error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigIssue {
    pub code: ConfigIssueCode,
    pub message: String,
    pub path: Utf8PathBuf,
    pub line: Option<u32>,
}

impl ConfigIssue {
    fn new(
        code: ConfigIssueCode,
        path: Utf8PathBuf,
        line: Option<u32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            path,
            line,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigDiagnostics {
    pub errors: Vec<ConfigIssue>,
    pub warnings: Vec<ConfigIssue>,
}

#[derive(Debug, Error)]
pub enum LoadConfigError {
    #[error("failed to enumerate configuration directory {path}: {source}")]
    ReadDir {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("configuration is invalid")]
    Invalid { diagnostics: ConfigDiagnostics },
    #[error("failed to read configuration file {path}: {source}")]
    Io {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct PromptAssembler {
    config: Config,
    warnings: Vec<ConfigIssue>,
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
        Self::load_with_diagnostics(dir).map_err(anyhow::Error::from)
    }

    /// Construct an assembler while retaining structured diagnostics.
    ///
    /// # Errors
    /// Returns a [`LoadConfigError`] when configuration files cannot be read or contain
    /// invalid definitions.
    pub fn load_with_diagnostics(dir: &Utf8Path) -> std::result::Result<Self, LoadConfigError> {
        let ConfigLoad { config, warnings } = load_config(dir)?;
        Ok(Self { config, warnings })
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
                    if !rendered.ends_with('\n') {
                        rendered.push('\n');
                    }
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
    pub fn prompt_specs(&self) -> &IndexMap<String, PromptSpec> {
        &self.config.prompts
    }

    #[must_use]
    pub fn prompt_spec(&self, name: &str) -> Option<&PromptSpec> {
        self.config.prompts.get(name)
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

    #[must_use]
    pub fn config_warnings(&self) -> &[ConfigIssue] {
        &self.warnings
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

struct ConfigLoad {
    config: Config,
    warnings: Vec<ConfigIssue>,
}

fn load_config(root: &Utf8Path) -> std::result::Result<ConfigLoad, LoadConfigError> {
    let mut prompts: IndexMap<String, PromptSpec> = IndexMap::new();
    let mut default_prompt_path: Option<Utf8PathBuf> = Some(root.to_owned());
    let mut warnings: Vec<ConfigIssue> = Vec::new();
    let mut errors: Vec<ConfigIssue> = Vec::new();

    let main_config = root.join("config.toml");
    if main_config.exists() {
        process_config_file(
            root,
            main_config.as_ref(),
            &mut prompts,
            &mut default_prompt_path,
            &mut warnings,
            &mut errors,
        )?;
    }

    let conf_d = root.join("conf.d");
    if conf_d.exists() {
        let mut entries: Vec<Utf8PathBuf> = Vec::new();
        let read_dir =
            fs::read_dir(conf_d.as_std_path()).map_err(|source| LoadConfigError::ReadDir {
                path: conf_d.clone(),
                source,
            })?;

        for entry in read_dir {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    errors.push(ConfigIssue::new(
                        ConfigIssueCode::ParseError,
                        conf_d.clone(),
                        None,
                        format!("failed to read entry in {conf_d}: {err}"),
                    ));
                    continue;
                }
            };

            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                match Utf8PathBuf::from_path_buf(path) {
                    Ok(path) => entries.push(path),
                    Err(_) => errors.push(ConfigIssue::new(
                        ConfigIssueCode::ParseError,
                        conf_d.clone(),
                        None,
                        "configuration paths must be valid UTF-8",
                    )),
                }
            }
        }

        entries.sort();

        for entry in entries {
            process_config_file(
                root,
                entry.as_ref(),
                &mut prompts,
                &mut default_prompt_path,
                &mut warnings,
                &mut errors,
            )?;
        }
    }

    if errors.is_empty() {
        Ok(ConfigLoad {
            config: Config {
                root: root.to_owned(),
                default_prompt_path,
                prompts,
            },
            warnings,
        })
    } else {
        Err(LoadConfigError::Invalid {
            diagnostics: ConfigDiagnostics { errors, warnings },
        })
    }
}

fn process_config_file(
    root: &Utf8Path,
    path: &Utf8Path,
    prompts: &mut IndexMap<String, PromptSpec>,
    default_prompt_path: &mut Option<Utf8PathBuf>,
    warnings: &mut Vec<ConfigIssue>,
    errors: &mut Vec<ConfigIssue>,
) -> std::result::Result<(), LoadConfigError> {
    let content = read_config_file(path)?;
    let raw: RawFile = match toml::from_str(&content) {
        Ok(raw) => raw,
        Err(err) => {
            let line = None;
            errors.push(ConfigIssue::new(
                ConfigIssueCode::ParseError,
                path.to_owned(),
                line,
                err.to_string(),
            ));
            return Ok(());
        }
    };

    if let Some(path_str) = raw.prompt_path {
        match resolve_path(root, &path_str) {
            Ok(resolved) => *default_prompt_path = Some(resolved),
            Err(err) => {
                errors.push(ConfigIssue::new(
                    ConfigIssueCode::InvalidPrompt,
                    path.to_owned(),
                    None,
                    format!("invalid prompt_path '{path_str}': {err}"),
                ));
                return Ok(());
            }
        }
    }

    let source = PromptSource {
        path: path.to_owned(),
        last_modified: fs::metadata(path.as_std_path())
            .and_then(|meta| meta.modified())
            .ok(),
    };

    for (name, prompt) in raw.prompt {
        match build_prompt_spec(root, &name, prompt, &source) {
            Ok(spec) => {
                if let Some(previous) = prompts.insert(name.clone(), spec) {
                    warnings.push(ConfigIssue::new(
                        ConfigIssueCode::Override,
                        source.path.clone(),
                        None,
                        format!(
                            "prompt '{name}' overrides definition from {}",
                            previous.metadata.source.path
                        ),
                    ));
                }
            }
            Err(issue) => errors.push(issue),
        }
    }

    Ok(())
}

fn read_config_file(path: &Utf8Path) -> std::result::Result<String, LoadConfigError> {
    let mut file = fs::File::open(path.as_std_path()).map_err(|source| LoadConfigError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|source| LoadConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
    Ok(buf)
}

fn build_prompt_spec(
    root: &Utf8Path,
    prompt_name: &str,
    prompt: RawPrompt,
    source: &PromptSource,
) -> std::result::Result<PromptSpec, ConfigIssue> {
    let prompt_path_override = match prompt.prompt_path {
        Some(path) => match resolve_path(root, &path) {
            Ok(resolved) => Some(resolved),
            Err(err) => {
                return Err(ConfigIssue::new(
                    ConfigIssueCode::InvalidPrompt,
                    source.path.clone(),
                    None,
                    format!("prompt '{prompt_name}' has invalid prompt_path '{path}': {err}"),
                ));
            }
        },
        None => None,
    };

    let kind = match (prompt.prompts, prompt.template) {
        (Some(files), None) => {
            if files.is_empty() {
                return Err(ConfigIssue::new(
                    ConfigIssueCode::InvalidPrompt,
                    source.path.clone(),
                    None,
                    "prompt sequence cannot be empty",
                ));
            }
            PromptKind::Sequence {
                files: files.into_iter().map(Utf8PathBuf::from).collect(),
            }
        }
        (None, Some(template)) => PromptKind::Template {
            template: Utf8PathBuf::from(template),
        },
        (Some(_), Some(_)) => {
            return Err(ConfigIssue::new(
                ConfigIssueCode::InvalidPrompt,
                source.path.clone(),
                None,
                "prompts and template are exclusive options",
            ));
        }
        (None, None) => {
            return Err(ConfigIssue::new(
                ConfigIssueCode::InvalidPrompt,
                source.path.clone(),
                None,
                "prompt must define either 'prompts' or 'template'",
            ));
        }
    };

    let vars = parse_prompt_vars(prompt_name, prompt.vars, source)?;

    let metadata = PromptMetadata {
        description: prompt.description,
        tags: prompt.tags,
        vars,
        stdin_supported: prompt.stdin_supported,
        source: source.clone(),
    };

    Ok(PromptSpec {
        prompt_path_override,
        kind,
        metadata,
    })
}

fn parse_prompt_vars(
    prompt_name: &str,
    vars: Vec<RawPromptVar>,
    source: &PromptSource,
) -> std::result::Result<Vec<PromptVariable>, ConfigIssue> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut parsed: Vec<PromptVariable> = Vec::with_capacity(vars.len());

    for raw in vars {
        if !seen.insert(raw.name.clone()) {
            return Err(ConfigIssue::new(
                ConfigIssueCode::DuplicateVar,
                source.path.clone(),
                None,
                format!("var '{}' declared twice", raw.name),
            ));
        }

        let raw_kind = raw.kind.unwrap_or_else(|| "string".to_owned());
        let kind = parse_var_kind(&raw_kind).ok_or_else(|| {
            ConfigIssue::new(
                ConfigIssueCode::InvalidPrompt,
                source.path.clone(),
                None,
                format!("unknown var type '{raw_kind}' for prompt '{prompt_name}'"),
            )
        })?;

        parsed.push(PromptVariable {
            name: raw.name,
            required: raw.required,
            kind,
            description: raw.description,
        });
    }

    Ok(parsed)
}

fn parse_var_kind(raw: &str) -> Option<PromptVariableKind> {
    match raw {
        "string" => Some(PromptVariableKind::String),
        "path" => Some(PromptVariableKind::Path),
        "number" => Some(PromptVariableKind::Number),
        "boolean" => Some(PromptVariableKind::Boolean),
        _ => None,
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
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    vars: Vec<RawPromptVar>,
    #[serde(default)]
    #[serde(rename = "stdin")]
    stdin_supported: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPromptVar {
    name: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    description: Option<String>,
}
