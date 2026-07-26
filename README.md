# RepoScribe

RepoScribe is a local-first repository understanding CLI written in Rust.

The project is under active development. The first working slice provides:

- `reposcribe --v` and `reposcribe --help`
- `reposcribe doctor`
- `reposcribe config ai` and `reposcribe config show`
- live Anthropic/OpenAI model discovery using environment-only API keys
- monorepo-aware project detection with a private Git cache
- static Rails schema, Prisma, and SQL database analysis with ERD generation
- PDF output by default, with HTML and Markdown alternatives
- automated unit and CLI integration tests

RepoScribe reads the current local checkout. It does not execute project code,
run framework commands, install dependencies, or fetch remote source.

## Development

```bash
cargo test --workspace
cargo run -p reposcribe-cli -- --help
```

Generate an ERD from the current local checkout:

```bash
cargo run -p reposcribe-cli -- erd
cargo run -p reposcribe-cli -- erd --output html --out reports/database
cargo run -p reposcribe-cli -- erd --project apps/api --output markdown
```

When a monorepo contains multiple databases, RepoScribe presents a selector in
an interactive terminal. Scripts can select one explicitly with `--project`.
