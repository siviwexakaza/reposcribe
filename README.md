# RepoScribe

RepoScribe is a local-first repository understanding CLI written in Rust.

The project is under active development. The first working slice provides:

- `reposcribe --v` and `reposcribe --help`
- `reposcribe doctor`
- `reposcribe config ai` and `reposcribe config show`
- live Anthropic/OpenAI model discovery using environment-only API keys
- monorepo-aware project detection with a private Git cache
- AI-driven, framework-independent database analysis and ERD generation
- AI-driven sequence and flow diagrams with smart entry resolution
- detailed Mermaid as the canonical format for every diagram, rendered consistently
  into PDF, HTML, or editable Markdown
- PDF output by default, with HTML and Markdown alternatives
- automated unit and CLI integration tests

RepoScribe reads the current local checkout. It does not execute project code,
run framework commands, install dependencies, or fetch remote source.

For ERDs, you explicitly provide the schema file or model directory with
`--source`. The configured AI reads that selected location through restricted,
read-only directory-listing and file-reading operations, then identifies the
tables and relationships defined there. RepoScribe blocks
secret files, build output, Git internals, writes, and paths outside the
repository. File contents requested by the AI are sent to the selected provider.

## Development

```bash
cargo test --workspace
cargo run -p reposcribe-cli -- --help
```

Generate an ERD from the current local checkout:

```bash
cargo run -p reposcribe-cli -- config ai
cargo run -p reposcribe-cli -- erd --source src/db/schema.ts
cargo run -p reposcribe-cli -- erd --source db/schema.rb --output html --out reports/database
cargo run -p reposcribe-cli -- erd --source app/models --output markdown
cargo run -p reposcribe-cli -- sequence --entry "POST /orders"
cargo run -p reposcribe-cli -- flow --entry create_order --output html
```

`--source` is required for every ERD. Pass a single schema file when the database
is defined centrally, or a model directory when each model represents a table.
In a monorepo, the path is relative to the repository root; `--project` remains
available if the selected source produces more than one project result.

For `sequence` and `flow`, `--entry` can describe a path, symbol, class,
function, route, endpoint, command, event, or feature. The AI resolves it by
inspecting the repository and traces only the relevant source files.
