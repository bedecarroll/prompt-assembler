# prompt-assembler

Create your own library of snippets to assemble prompts.

## Table of contents

- [Features](#features)
- [Installation](#installation)
- [Configuration](#configuration)
- [Usage](#usage)
  - [Simple prompt](#simple-prompt)
  - [Piping input](#piping-input)
  - [Multiple prompts with variables](#multiple-prompts-with-variables)
  - [Ad-hoc parts](#ad-hoc-parts)
  - [Jinja templates](#jinja-template)
  - [JSON API](#json-api)
  - [Shell completions](#shell-completions)
- [Development](#development)
- [Releasing](#releasing)
- [License](#license)

## Features

- Uses XDG
  - ~/.config/prompt-assembler/
- Supports splitting your configuration
  - ~/.config/prompt-assembler/conf.d/
  - uses lexical order
- Uses TOML for config
- `prompt_path` is optional; when omitted, prompt files are resolved relative to the directory containing `config.toml`
- You can add variables to your prompts, up to 9 arguments starting at `{0}`
  - Use `{{` for literal curly braces in fragments
  - Beware of making overly long prompts however as you might run into shell limitations
- Concatenate raw parts on demand with `pa parts`, which skips placeholder substitution so braces like `{0}` remain literal
- Sequence prompts can consume piped stdin as their first argument (`{0}`)
- Can use Jinja templates (using minijinja)
  - Allows you to create parameterized templates
- Template data files support JSON or TOML formats (auto-detected by extension)
- Shell completions include your prompts
- Prints your completed prompts on stdout

## Installation

Prebuilt installers are available for macOS, Linux, and Windows once a release is tagged.

### Shell (macOS and Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/bedecarroll/prompt-assembler/releases/download/v0.2.1/pa-installer.sh | sh
```

### PowerShell (Windows)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/bedecarroll/prompt-assembler/releases/download/v0.2.1/pa-installer.ps1 | iex"
```

### Cargo (alternative)

```bash
cargo install prompt-assembler --version 0.2.1
```

> **Note**  
> Update the version in the commands above if a newer release is available.

#### Installer destination

The installer places the `pa` binary in the first writable directory from:

1. `$PA_INSTALL_DIR/bin` (if the environment variable is set)
2. `~/.local/bin`

On Unix-like systems, make sure one of those directories is on your `PATH`. On Windows, set `PA_INSTALL_DIR` to `%USERPROFILE%\.cargo` before running the installer so the binary lands in the default Cargo location (`%USERPROFILE%\.cargo\bin`), which is already on `PATH` when installed via `rustup`.

## Config file

```toml
prompt_path = "~/.config/prompt-assembler/"

[prompt.create-ticket]
prompt_path = "~/.config/prompt-assembler/"
# Paths, in order
prompts = [
  "ticket.md"
]

[prompt.update-ticket]
prompts = [
  "get-ticket.md",
  "update-ticket.md"
]

[prompt.troubleshooting]
template = "troubleshooting.j2"

[prompt.echo]
prompts = ["echo.md"]
```

### Configuration layout

Configuration follows the XDG base directory spec:

- Base directory: `~/.config/prompt-assembler/`
- Optional fragments: any `*.toml` file inside `~/.config/prompt-assembler/conf.d/` are loaded in lexical order.
- If a prompt omits `prompt_path`, prompt fragments are resolved relative to the directory that contained the TOML file where the prompt was defined.

## Examples

### Simple prompt

```bash
$ cat ticket.md
# New ticket prompt

Create a new ticket
$ pa create-ticket
# New ticket prompt

Create a new ticket
```

### Piping input

```bash
$ cat echo.md
Echo {0}
$ echo "piped text" | pa echo
Echo piped text
```

### Multiple prompts with variables

```bash
$ cat get-ticket.md update-ticket.md
# Get ticket markdown

Search for ticket {0} using your MCP
# Update ticket markdown

Update the ticket with:
{1}
$ pa update-ticket tic-123 "working on ticket now"
# Get ticket markdown

Search for ticket tic-123 using your MCP
# Update ticket markdown

Update the ticket with:
working on ticket now
```

### Ad-hoc parts

Use `pa parts` when you want to stitch a few fragments together without defining a prompt first. Each filename is searched relative to your current working directory and then the library `prompt_path`.

```bash
$ ls
intro.md outro.md
$ cat intro.md outro.md
Intro with literal {0}
Outro with literal {1}
$ pa parts intro.md outro.md
Intro with literal {0}
Outro with literal {1}
```

The command prints files verbatim—placeholders such as `{0}` are *not* substituted, which makes it safe for assembling fragments that intentionally contain curly braces.

### Jinja template

```bash
$ cat troubleshooting.j2 always.j2 vars.json
Hello {{ var }}!
{% include 'always.j2' %}
How are you {{ name }}?
{
  "var": "World",
  "name": "Bede"
}
$ pa troubleshooting vars.json
Hello World!
How are you Bede?
```

Template prompts require a structured data file. The CLI infers the format from the extension:

- `.json` → JSON
- `.toml` → TOML

Sequence prompts reject structured data.

### JSON API

`pa` exposes machine-readable output for launchers or automation that need prompt metadata:

- `pa list --json` emits an envelope with `schema_version`, an ISO-8601 `generated_at` timestamp, and a `prompts` array. Each prompt object includes `name`, optional `description`, `tags`, `vars`, `stdin_supported`, `last_modified`, and the absolute `source_path` of the TOML definition.
- `pa show <prompt> --json` returns the same prompt object for a single entry and exits with code `1` when the prompt is unknown.
- `pa validate [--json]` checks configuration integrity. It exits `0` when valid, `2` when invalid, and prints diagnostics. The JSON envelope contains `errors` and `warnings`, each with `file`, optional `line`, `code`, and `message` fields.

All JSON responses currently use `schema_version = 1`. If configuration files are unreadable (for example, the config directory is missing), commands exit with code `127`.

### Shell completions

Generate completions for your shell at runtime:

```bash
$ pa completions bash > ~/.local/share/bash-completion/pa
$ source ~/.local/share/bash-completion/pa
```

`pa` inspects your configuration at generation time, so completions stay in sync with your prompt names. Regenerate the script after adding or removing prompts.

## Flags

- -h help
- -V version

## Development

Use `mise` tasks for local workflows:

- `mise run fmt`
- `mise run clippy`
- `mise run unit`
- `mise run dist` (wraps `cargo dist` to build release artifacts)
- `mise run lint` (spell-checks with `typos`)

Running these commands directly with Cargo works too:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings -D clippy::pedantic
cargo test
```

## Releasing

Release automation is powered by [`cargo dist`](https://github.com/axodotdev/cargo-dist) and a GitHub Actions workflow.

1. Update `Cargo.toml` with the desired version.
2. Tag the commit: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. The `Release` workflow builds artifacts and publishes a GitHub Release automatically.

You can preview the artifacts locally with:

```bash
cargo dist build
```

## License

MIT
