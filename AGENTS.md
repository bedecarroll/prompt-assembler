# Development Playbook

## Core Principles
- Favor readable, direct solutions; skip speculative abstractions or "future-proofing".
- Follow the red-green-refactor TDD loop. Start every change by writing failing tests, then implement the minimal code to pass, and finally clean up.
- Target Rust 2024 edition with `clippy::pedantic` and `-D warnings` enforced in CI and local workflows.
- Use `mise` for repeatable tooling and tasks; avoid `make`, `just`, or bespoke scripts unless coordinated through `mise`.
- Release automation relies on `cargo-dist`; ensure release tasks are defined via `mise`.
- Prefer small, composable modules; pull complexity into isolated components with clear tests.
- Comments only when intent would otherwise be unclear; rely on expressive naming and structure first.

## Style & Practices
- Adhere to idiomatic Rust patterns (ownership clarity, explicit error handling, minimal `unwrap`).
- Maintain deterministic behavior, especially when processing configuration files or directory listings.
- Configuration supports XDG paths, TOML format for metadata, and prompt inputs that may be JSON or TOML.
- Prompt placeholders begin at `{0}`; escape literal braces with `{{`.
- Templates use Minijinja; ensure template search paths are explicit, and context data is validated.
- Structured prompt data accepts JSON or TOML, mirroring Minijinja CLI format handling.

## Quality Gates
- Every feature addition or bug fix requires: failing tests → implementation → passing tests → refactor.
- Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings -D clippy::pedantic`, `cargo test`, and corresponding `mise` tasks before completion.
- Prefer integration tests for CLI behavior and unit tests for internal modules; cover edge cases (missing files, bad data, argument mismatches).
- Document any deviations from these rules in PR notes and update this guide when practices evolve.
