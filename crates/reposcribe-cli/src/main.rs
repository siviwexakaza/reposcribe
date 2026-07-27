use std::{env, io::IsTerminal, path::PathBuf, time::Duration};

use clap::{ArgAction, Parser, Subcommand};
use console::style;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use inquire::Select;
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use reposcribe_ai::{AiClient, AiError, ProviderModel};
use reposcribe_analyzers::{find_repository_root, load_or_detect_project};
use reposcribe_core::{
    AiConfiguration, AiProvider, CacheState, ConfigStore, DatabaseProjectAnalysis, ModuleProfile,
    OutputFormat,
};

#[derive(Debug, Parser)]
#[command(
    name = "reposcribe",
    version,
    about = "Understand a local repository through clear, framework-aware reports",
    long_about = None,
    propagate_version = true
)]
struct Cli {
    /// Print the RepoScribe version.
    #[arg(long = "v", action = ArgAction::Version)]
    version_alias: Option<bool>,

    /// Repository path. Defaults to the current directory.
    #[arg(long, global = true, value_name = "PATH")]
    repo: Option<PathBuf>,

    /// Hide non-essential status messages.
    #[arg(long, global = true)]
    quiet: bool,

    /// Disable terminal colors.
    #[arg(long, global = true)]
    no_color: bool,

    /// Show additional diagnostic information.
    #[arg(short, long, global = true, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Configure RepoScribe without storing any API keys.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Check repository access and credential configuration.
    Doctor,
    /// Generate an entity-relationship diagram from local database definition files.
    Erd {
        /// Schema file or model directory containing the database definitions.
        #[arg(long, value_name = "FILE_OR_DIRECTORY")]
        source: PathBuf,
        /// Output type: pdf (default), html, or markdown.
        #[arg(long, default_value_t = OutputFormat::default(), value_name = "TYPE")]
        output: OutputFormat,
        /// Destination path. The selected output extension is added automatically.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Project name or path to use when a monorepo has multiple databases.
        #[arg(long, value_name = "NAME_OR_PATH")]
        project: Option<String>,
    },
    /// Generate a control/data-flow diagram from a file, symbol, route, or feature.
    Flow {
        /// Starting file, symbol, class, function, route, endpoint, event, or feature.
        #[arg(long, value_name = "ENTRY")]
        entry: String,
        /// Output type: pdf (default), html, or markdown.
        #[arg(long, default_value_t = OutputFormat::default(), value_name = "TYPE")]
        output: OutputFormat,
        /// Destination path. The selected output extension is added automatically.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// List text-generation models available for an AI provider.
    Models {
        /// Provider to query. Defaults to the configured provider.
        #[arg(long, value_name = "PROVIDER")]
        provider: Option<AiProvider>,
    },
    /// Generate a sequence diagram from a file, symbol, route, or feature.
    Sequence {
        /// Starting file, symbol, class, function, route, endpoint, event, or feature.
        #[arg(long, value_name = "ENTRY")]
        entry: String,
        /// Output type: pdf (default), html, or markdown.
        #[arg(long, default_value_t = OutputFormat::default(), value_name = "TYPE")]
        output: OutputFormat,
        /// Destination path. The selected output extension is added automatically.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Detect and inspect the projects in the current repository.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Select the AI provider and model.
    Ai {
        /// Anthropic or OpenAI. Omit in a terminal to select interactively.
        #[arg(long, value_name = "PROVIDER")]
        provider: Option<AiProvider>,
        /// Model ID. Omit in a terminal to select from the provider's live model list.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
    },
    /// Show the saved provider and model without exposing credentials.
    Show,
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Show the detected project profile, using the private cache when valid.
    Show,
    /// Force project and framework detection to run again.
    Refresh,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.no_color || env::var_os("NO_COLOR").is_some() {
        console::set_colors_enabled(false);
        console::set_colors_enabled_stderr(false);
    }

    match &cli.command {
        Command::Config { command } => match command {
            ConfigCommand::Ai { provider, model } => {
                configure_ai(&cli, *provider, model.clone()).await
            }
            ConfigCommand::Show => show_configuration(),
        },
        Command::Doctor => doctor(&cli),
        Command::Erd {
            source,
            output,
            out,
            project,
        } => generate_erd(&cli, source, *output, out.clone(), project.as_deref()).await,
        Command::Flow { entry, output, out } => {
            generate_flow(&cli, entry, *output, out.clone()).await
        }
        Command::Models { provider } => list_models(&cli, *provider).await,
        Command::Project { command } => match command {
            ProjectCommand::Show => show_project(&cli, false),
            ProjectCommand::Refresh => show_project(&cli, true),
        },
        Command::Sequence { entry, output, out } => {
            generate_sequence(&cli, entry, *output, out.clone()).await
        }
    }
}

async fn generate_erd(
    cli: &Cli,
    source: &std::path::Path,
    output: OutputFormat,
    destination: Option<PathBuf>,
    project: Option<&str>,
) -> Result<()> {
    let root = repository_root(cli)?;
    let source = resolve_erd_source(&root, source)?;
    let (ai, client) = configured_ai_client()?;
    let analysis_spinner = spinner("AI is inspecting the repository", cli.quiet);
    let analysis = client
        .analyze_repository_database(&root, &ai.model, &source, project)
        .await
        .map_err(|error| friendly_ai_error(ai.provider, error))?;
    analysis_spinner.finish_and_clear();
    let selected = select_database_project(analysis.projects, project)?;
    let schema = selected.schema.clone();

    let destination = destination.unwrap_or_else(|| root.join("reposcribe-erd"));
    let render_spinner = spinner(&format!("Creating {output} diagram"), cli.quiet);
    let rendered = reposcribe_render::render_erd(&schema, output, &destination)
        .map_err(|error| miette!("diagram generation failed: {error}"))?;
    render_spinner.finish_and_clear();

    println!("{} ERD created", style("✓").green());
    println!("  Source         {}", source.display());
    println!("  Project        {}", selected.name);
    println!(
        "  Framework      {}",
        selected.framework.as_deref().unwrap_or("not identified")
    );
    println!(
        "  Database       {}",
        selected
            .database_technology
            .as_deref()
            .unwrap_or("not identified")
    );
    println!("  Entities       {}", schema.entities.len());
    println!("  Relationships  {}", schema.relationships.len());
    println!("  Output         {}", rendered.display());
    Ok(())
}

fn resolve_erd_source(root: &std::path::Path, source: &std::path::Path) -> Result<PathBuf> {
    let requested = if source.is_absolute() {
        source.to_path_buf()
    } else {
        root.join(source)
    };
    let canonical = requested.canonicalize().map_err(|error| {
        miette!(
            help = "Pass an existing schema file or model directory, for example `--source src/db/schema.ts` or `--source app/models`.",
            "could not access ERD source '{}': {error}",
            source.display()
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(miette!(
            "ERD source '{}' must be inside the repository",
            source.display()
        ));
    }
    if !canonical.is_file() && !canonical.is_dir() {
        return Err(miette!(
            "ERD source '{}' is not a file or directory",
            source.display()
        ));
    }
    canonical
        .strip_prefix(root)
        .map(PathBuf::from)
        .map_err(|_| miette!("could not resolve the ERD source inside the repository"))
}

async fn generate_sequence(
    cli: &Cli,
    entry: &str,
    output: OutputFormat,
    destination: Option<PathBuf>,
) -> Result<()> {
    let root = repository_root(cli)?;
    let (ai, client) = configured_ai_client()?;
    let analysis_spinner = spinner("AI is tracing the sequence", cli.quiet);
    let diagram = client
        .analyze_sequence_diagram(&root, &ai.model, entry)
        .await
        .map_err(|error| friendly_ai_error(ai.provider, error))?;
    analysis_spinner.finish_and_clear();
    let destination = destination.unwrap_or_else(|| root.join("reposcribe-sequence"));
    let render_spinner = spinner(&format!("Creating {output} diagram"), cli.quiet);
    let rendered = reposcribe_render::render_sequence(&diagram, output, &destination)
        .map_err(|error| miette!("diagram generation failed: {error}"))?;
    render_spinner.finish_and_clear();

    println!("{} Sequence diagram created", style("✓").green());
    println!("  Entry          {}", diagram.entry);
    println!("  Mermaid lines  {}", diagram.mermaid.lines().count());
    println!("  Sources        {}", diagram.source_files.len());
    println!("  Output         {}", rendered.display());
    Ok(())
}

async fn generate_flow(
    cli: &Cli,
    entry: &str,
    output: OutputFormat,
    destination: Option<PathBuf>,
) -> Result<()> {
    let root = repository_root(cli)?;
    let (ai, client) = configured_ai_client()?;
    let analysis_spinner = spinner("AI is tracing the flow", cli.quiet);
    let diagram = client
        .analyze_flow_diagram(&root, &ai.model, entry)
        .await
        .map_err(|error| friendly_ai_error(ai.provider, error))?;
    analysis_spinner.finish_and_clear();
    let destination = destination.unwrap_or_else(|| root.join("reposcribe-flow"));
    let render_spinner = spinner(&format!("Creating {output} diagram"), cli.quiet);
    let rendered = reposcribe_render::render_flow(&diagram, output, &destination)
        .map_err(|error| miette!("diagram generation failed: {error}"))?;
    render_spinner.finish_and_clear();

    println!("{} Flow diagram created", style("✓").green());
    println!("  Entry          {}", diagram.entry);
    println!("  Mermaid lines  {}", diagram.mermaid.lines().count());
    println!("  Sources        {}", diagram.source_files.len());
    println!("  Output         {}", rendered.display());
    Ok(())
}

fn configured_ai_client() -> Result<(AiConfiguration, AiClient)> {
    let store = ConfigStore::discover().map_err(|error| miette!("{error}"))?;
    let configuration = store.load().map_err(|error| miette!("{error}"))?;
    let ai = configuration.ai.ok_or_else(|| {
        miette!(
            help = "Run `reposcribe config ai` to select Anthropic or OpenAI and a model.",
            "an AI provider is required to inspect the repository"
        )
    })?;
    let client = client_from_environment(ai.provider)?;
    Ok((ai, client))
}

fn select_database_project(
    projects: Vec<DatabaseProjectAnalysis>,
    project: Option<&str>,
) -> Result<DatabaseProjectAnalysis> {
    if let Some(query) = project {
        let normalized = query.trim().trim_matches('/').to_ascii_lowercase();
        let mut matches: Vec<DatabaseProjectAnalysis> = projects
            .into_iter()
            .filter(|candidate| {
                candidate.name.eq_ignore_ascii_case(&normalized)
                    || candidate
                        .root
                        .to_string_lossy()
                        .trim_matches('/')
                        .eq_ignore_ascii_case(&normalized)
            })
            .collect();
        return match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(miette!(
                help = "Run `reposcribe erd` in an interactive terminal to choose from detected databases, or use a project name/path shown by `reposcribe project show`.",
                "no database matched --project {query}"
            )),
            _ => Err(miette!(
                help = "Use the project's relative path to select a specific database.",
                "more than one database matched --project {query}"
            )),
        };
    }

    if projects.len() == 1 {
        return Ok(projects.into_iter().next().expect("one project exists"));
    }
    require_interactive(
        "Multiple databases were found. Pass --project <name-or-path> when running non-interactively.",
    )?;
    Select::new("Select a project database to diagram:", projects)
        .with_page_size(12)
        .prompt()
        .map_err(|error| miette!("database selection was cancelled: {error}"))
}

async fn configure_ai(
    cli: &Cli,
    provider: Option<AiProvider>,
    model: Option<String>,
) -> Result<()> {
    let provider = match provider {
        Some(provider) => provider,
        None => select_provider()?,
    };
    let client = client_from_environment(provider)?;
    let model = match model {
        Some(model) if !model.trim().is_empty() => model,
        Some(_) => return Err(miette!("the model ID cannot be empty")),
        None => {
            require_interactive(
                "A model was not provided. Pass --model <model-id> when running non-interactively.",
            )?;
            let spinner = spinner(&format!("Loading {provider} models"), cli.quiet);
            let models = client
                .list_models()
                .await
                .map_err(|error| friendly_ai_error(provider, error))?;
            spinner.finish_and_clear();
            select_model(provider, models)?.id
        }
    };

    let store = ConfigStore::discover().map_err(|error| miette!("{error}"))?;
    let mut config = store.load().map_err(|error| miette!("{error}"))?;
    config.ai = Some(AiConfiguration {
        provider,
        model: model.clone(),
    });
    store.save(&config).map_err(|error| miette!("{error}"))?;

    println!("{} AI configuration saved", style("✓").green());
    println!("  Provider  {provider}");
    println!("  Model     {model}");
    println!("  Config    {}", style(store.path().display()).dim());
    println!();
    println!(
        "{}",
        style("The API key remains in your environment and was not saved.").dim()
    );
    Ok(())
}

fn show_configuration() -> Result<()> {
    let store = ConfigStore::discover().map_err(|error| miette!("{error}"))?;
    let config = store.load().map_err(|error| miette!("{error}"))?;

    println!("{}", style("RepoScribe configuration").bold());
    println!();
    match config.ai {
        Some(ai) => {
            let variable = ai.provider.api_key_environment_variable();
            println!("  Provider    {}", ai.provider);
            println!("  Model       {}", ai.model);
            println!(
                "  Credential  {}",
                if env::var_os(variable).is_some() {
                    style(format!("{variable} is set")).green().to_string()
                } else {
                    style(format!("{variable} is not set")).yellow().to_string()
                }
            );
        }
        None => println!(
            "{}",
            style("No AI provider is configured. Run `reposcribe config ai`.").yellow()
        ),
    }
    println!("  Config      {}", style(store.path().display()).dim());
    Ok(())
}

async fn list_models(cli: &Cli, provider: Option<AiProvider>) -> Result<()> {
    let provider = match provider {
        Some(provider) => provider,
        None => {
            let store = ConfigStore::discover().map_err(|error| miette!("{error}"))?;
            store
                .load()
                .map_err(|error| miette!("{error}"))?
                .ai
                .map(|ai| ai.provider)
                .ok_or_else(|| {
                    miette!(
                        help = "Run `reposcribe config ai`, or pass --provider anthropic|openai.",
                        "no AI provider is configured"
                    )
                })?
        }
    };
    let client = client_from_environment(provider)?;
    let spinner = spinner(&format!("Loading {provider} models"), cli.quiet);
    let models = client
        .list_models()
        .await
        .map_err(|error| friendly_ai_error(provider, error))?;
    spinner.finish_and_clear();

    println!("{}", style(format!("{provider} models")).bold());
    println!();
    for model in models {
        println!("  {} {}", style("•").cyan(), model);
    }
    Ok(())
}

fn select_provider() -> Result<AiProvider> {
    require_interactive(
        "An AI provider was not provided. Pass --provider anthropic|openai when running non-interactively.",
    )?;
    Select::new("Select an AI provider:", AiProvider::ALL.to_vec())
        .prompt()
        .map_err(|error| miette!("provider selection was cancelled: {error}"))
}

fn select_model(provider: AiProvider, models: Vec<ProviderModel>) -> Result<ProviderModel> {
    Select::new(&format!("Select a {provider} model:"), models)
        .with_page_size(12)
        .prompt()
        .map_err(|error| miette!("model selection was cancelled: {error}"))
}

fn require_interactive(message: &str) -> Result<()> {
    if !std::io::stdin().is_terminal() {
        return Err(miette!(
            help = message,
            "interactive selection needs a terminal"
        ));
    }
    Ok(())
}

fn client_from_environment(provider: AiProvider) -> Result<AiClient> {
    AiClient::from_environment(provider).map_err(|error| friendly_ai_error(provider, error))
}

fn friendly_ai_error(provider: AiProvider, error: AiError) -> miette::Report {
    if matches!(error, AiError::MissingApiKey { .. }) {
        let variable = provider.api_key_environment_variable();
        return miette!(
            help = format!(
                "Set it in your shell, for example:\n  export {variable}=\"your-key\"\n\nRepoScribe never stores this value."
            ),
            "{provider} API key was not found in {variable}"
        );
    }
    miette!("{error}")
}

fn requested_path(cli: &Cli) -> Result<PathBuf> {
    cli.repo
        .clone()
        .map(Ok)
        .unwrap_or_else(|| env::current_dir().into_diagnostic())
}

fn repository_root(cli: &Cli) -> Result<PathBuf> {
    let requested = requested_path(cli)?;
    find_repository_root(&requested).map_err(|error| {
        miette!(
            help = "Run RepoScribe inside a local Git checkout, or pass --repo <path>.",
            "{error}"
        )
    })
}

fn doctor(cli: &Cli) -> Result<()> {
    let root = repository_root(cli)?;
    let checks = [
        ("Local repository", true, root.display().to_string()),
        (
            "Anthropic API key",
            env::var_os("ANTHROPIC_API_KEY").is_some(),
            "ANTHROPIC_API_KEY".to_owned(),
        ),
        (
            "OpenAI API key",
            env::var_os("OPENAI_API_KEY").is_some(),
            "OPENAI_API_KEY".to_owned(),
        ),
        (
            "GitHub token",
            env::var_os("GH_TOKEN").is_some() || env::var_os("GITHUB_TOKEN").is_some(),
            "GH_TOKEN or GITHUB_TOKEN".to_owned(),
        ),
    ];

    println!("{}", style("RepoScribe doctor").bold());
    println!();
    for (label, available, detail) in checks {
        if available {
            println!(
                "{} {:<22} {}",
                style("✓").green(),
                label,
                style(detail).dim()
            );
        } else {
            println!(
                "{} {:<22} {}",
                style("•").yellow(),
                label,
                style(format!("not set ({detail})")).dim()
            );
        }
    }
    println!();
    println!(
        "{}",
        style("Keys are read from the environment and are never stored by RepoScribe.").dim()
    );
    Ok(())
}

fn show_project(cli: &Cli, refresh: bool) -> Result<()> {
    let root = repository_root(cli)?;
    let spinner = spinner(
        if refresh {
            "Refreshing project profile"
        } else {
            "Detecting projects and frameworks"
        },
        cli.quiet,
    );

    let outcome = load_or_detect_project(&root, refresh)
        .map_err(|error| miette!("project detection failed: {error}"))
        .wrap_err("RepoScribe could not build the local project profile")?;
    spinner.finish_and_clear();

    if let Some(warning) = outcome.warning {
        eprintln!("{} {warning}", style("warning:").yellow().bold());
    }

    let profile = outcome.profile;
    println!("{}", style("Repository profile").bold());
    println!("  Root       {}", profile.repository_root.display());
    println!(
        "  Structure  {}",
        if profile.is_monorepo {
            format!("monorepo ({} projects)", profile.modules.len())
        } else {
            format!("single project ({} module)", profile.modules.len())
        }
    );
    println!(
        "  Detection  {}",
        match outcome.cache_state {
            CacheState::Hit => "cached",
            CacheState::Refreshed => "refreshed",
            CacheState::Unavailable => "not cached",
        }
    );

    if profile.modules.is_empty() {
        println!();
        println!(
            "{}",
            style("No supported project manifests were found in this repository.").yellow()
        );
        return Ok(());
    }

    println!();
    for module in &profile.modules {
        print_module(module);
    }
    Ok(())
}

fn print_module(module: &ModuleProfile) {
    let root = if module.root.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        module.root.display().to_string()
    };
    println!("{} {}", style("●").cyan(), style(&module.name).bold());
    println!("  Path        {root}");
    println!("  Languages   {}", display_values(&module.languages));
    println!("  Frameworks  {}", display_values(&module.frameworks));
    println!(
        "  Database    {}",
        display_values(&module.database_technologies)
    );
    println!();
}

fn display_values<T: std::fmt::Display>(values: &[T]) -> String {
    if values.is_empty() {
        return style("not detected").dim().to_string();
    }
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn spinner(message: &str, quiet: bool) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    if quiet || !std::io::stderr().is_terminal() {
        spinner.set_draw_target(ProgressDrawTarget::hidden());
        return spinner;
    }
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .expect("the built-in spinner template is valid")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message(message.to_owned());
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner
}
