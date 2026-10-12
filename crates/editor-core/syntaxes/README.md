# Embedded syntax definitions

Each `*.syntax` file supplies one language's display label, filename matches,
extension matches, and keyword list. `include_str!` in `src/syntax.rs` embeds
these UTF-8 files in the binary; no syntax directory is needed at runtime.

Fields are `key=value`, one per line. Lists use spaces as separators; filenames
and extensions are case-sensitive. `label` and `lexer` are required. Supported
lexer names: `code`, `markdown`, `html`, `makefile`, `dockerfile`, `ignore`, and
`key_value`. Optional fields are `filenames`, `prefixes`, `extensions`,
`keywords`, `line_comment`, `block_comment=true`, and
`case_insensitive_keywords=true`. Exact filenames win over prefixes, which win
over extensions. A leading `#` line is a definition-file comment.

To add a language using an existing lexer, add a definition, add its typed
`Language` variant and `include_str!` registration in `src/syntax.rs`, then add
filename and token tests. New syntax structures still require a focused scanner;
these files are not full parser grammars. Keep color assignments in the shared
workbench theme, not in the syntax definitions. Tests reject duplicate matches
and malformed fields.
