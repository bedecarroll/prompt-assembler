# Changelog

All notable changes to this project will be documented in this file.

## 0.3.0 - 2025-10-30

- add prompt profile details to `pa show --json`, including each fragment and the combined content
- expose prompt fragments in the library with `PromptAssembler::prompt_profile` for tools that need raw parts

## 0.2.0 - 2025-10-18

- add a `pa parts` subcommand to concatenate prompt fragments from the working directory or configured `prompt_path`
- document the new workflow and update install snippets

## 0.1.1 - 2025-10-18

- add shell and PowerShell installer scripts to the release artifacts
- document installer-based setup in the README
- start maintaining this changelog

## 0.1.0 - 2025-10-18

- initial release of the `pa` CLI and supporting library
