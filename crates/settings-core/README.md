# Lumen settings core

UI-independent settings definitions and rules for the Lumen workbench.

- Each section owns its schema, defaults, control metadata and validation.
- `catalog()` and `SECTIONS` drive consumers without depending on a widget toolkit.
- `resolve()` implements Default → User → Project precedence for supplied layers.
- `validate_layers()` checks individual values and effective cross-field rules.
- `Setting::matches()` handles local search, aliases and scope filters.
- `parse()` and `serialize()` decode TOML and preserve comments/unknown fields
  when writing sparse overrides. Validate draft layers before serializing them.

This crate does not resolve home/workspace paths, read or write files, watch
configuration, own editable drafts, or render UI. Consumers supply the applicable
layers; document/project containment remains an application responsibility.

Run its tests independently with `cargo test -p lumen-settings-core`.
