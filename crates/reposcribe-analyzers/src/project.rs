use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use ignore::{DirEntry, WalkBuilder};
use reposcribe_core::{
    CacheState, DatabaseTechnology, DetectionEvidence, Framework, Language, ModuleProfile,
    ProjectProfile, ProjectProfileOutcome,
};
use serde_json::Value;
use thiserror::Error;

const SCHEMA_VERSION: u32 = 1;
const DETECTOR_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum DetectionError {
    #[error("'{0}' is not inside a Git repository")]
    NotARepository(PathBuf),
    #[error("failed to read repository files: {0}")]
    Walk(#[from] ignore::Error),
    #[error("failed to read '{path}': {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize the project profile: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
struct Marker {
    relative_path: PathBuf,
    module_root: PathBuf,
    kind: MarkerKind,
    content: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    Cargo,
    ConvexSchema,
    CsProject,
    DrizzleConfig,
    Gemfile,
    GoMod,
    Gradle,
    NodePackage,
    Pom,
    PrismaSchema,
    PyProject,
    RailsSchema,
    SupabaseConfig,
}

#[derive(Default)]
struct ModuleBuilder {
    name: Option<String>,
    languages: BTreeSet<Language>,
    frameworks: BTreeSet<Framework>,
    database_technologies: BTreeSet<DatabaseTechnology>,
    evidence: Vec<DetectionEvidence>,
}

pub fn find_repository_root(start: &Path) -> Result<PathBuf, DetectionError> {
    let start = start
        .canonicalize()
        .map_err(|source| DetectionError::Read {
            path: start.to_path_buf(),
            source,
        })?;
    let mut current = if start.is_file() {
        start.parent().unwrap_or(&start).to_path_buf()
    } else {
        start
    };

    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(DetectionError::NotARepository(current));
        }
    }
}

pub fn detect_project(repository_root: &Path) -> Result<ProjectProfile, DetectionError> {
    let markers = collect_markers(repository_root)?;
    Ok(profile_from_markers(repository_root, &markers))
}

pub fn load_or_detect_project(
    repository_root: &Path,
    force_refresh: bool,
) -> Result<ProjectProfileOutcome, DetectionError> {
    let markers = collect_markers(repository_root)?;
    let fresh_profile = profile_from_markers(repository_root, &markers);
    let cache_path = cache_path(repository_root);

    if !force_refresh {
        if let Ok(path) = &cache_path {
            if let Ok(bytes) = fs::read(path) {
                if let Ok(cached) = serde_json::from_slice::<ProjectProfile>(&bytes) {
                    if cached.schema_version == SCHEMA_VERSION
                        && cached.detector_version == DETECTOR_VERSION
                        && cached.fingerprint == fresh_profile.fingerprint
                    {
                        return Ok(ProjectProfileOutcome {
                            profile: cached,
                            cache_state: CacheState::Hit,
                            warning: None,
                        });
                    }
                }
            }
        }
    }

    let cache_write = cache_path
        .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
        .and_then(|path| write_cache(&path, &fresh_profile));

    match cache_write {
        Ok(()) => Ok(ProjectProfileOutcome {
            profile: fresh_profile,
            cache_state: CacheState::Refreshed,
            warning: None,
        }),
        Err(error) => Ok(ProjectProfileOutcome {
            profile: fresh_profile,
            cache_state: CacheState::Unavailable,
            warning: Some(format!(
                "project detection succeeded, but the private cache could not be written: {error}"
            )),
        }),
    }
}

fn collect_markers(repository_root: &Path) -> Result<Vec<Marker>, DetectionError> {
    let mut markers = Vec::new();
    let walker = WalkBuilder::new(repository_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .filter_entry(should_visit)
        .build();

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let absolute_path = entry.into_path();
        let relative_path = absolute_path
            .strip_prefix(repository_root)
            .unwrap_or(&absolute_path)
            .to_path_buf();
        let Some((kind, module_root)) = classify_marker(&relative_path) else {
            continue;
        };
        let content = fs::read(&absolute_path).map_err(|source| DetectionError::Read {
            path: absolute_path.clone(),
            source,
        })?;
        markers.push(Marker {
            relative_path,
            module_root,
            kind,
            content,
        });
    }

    markers.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(markers)
}

fn should_visit(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    !matches!(
        name.as_ref(),
        ".git" | ".reposcribe" | "node_modules" | "target" | "vendor" | "dist" | "build"
    )
}

fn classify_marker(path: &Path) -> Option<(MarkerKind, PathBuf)> {
    let file_name = path.file_name()?.to_string_lossy();
    let parent = path.parent().unwrap_or_else(|| Path::new(""));

    let direct = match file_name.as_ref() {
        "Cargo.toml" => Some(MarkerKind::Cargo),
        "Gemfile" => Some(MarkerKind::Gemfile),
        "go.mod" => Some(MarkerKind::GoMod),
        "package.json" => Some(MarkerKind::NodePackage),
        "pom.xml" => Some(MarkerKind::Pom),
        "pyproject.toml" => Some(MarkerKind::PyProject),
        "build.gradle" | "build.gradle.kts" => Some(MarkerKind::Gradle),
        "drizzle.config.ts" | "drizzle.config.js" | "drizzle.config.mjs" => {
            Some(MarkerKind::DrizzleConfig)
        }
        value if value.ends_with(".csproj") => Some(MarkerKind::CsProject),
        _ => None,
    };
    if let Some(kind) = direct {
        return Some((kind, parent.to_path_buf()));
    }

    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.ends_with("prisma/schema.prisma") {
        return Some((MarkerKind::PrismaSchema, parent.parent()?.to_path_buf()));
    }
    if normalized.ends_with("db/schema.rb") {
        return Some((MarkerKind::RailsSchema, parent.parent()?.to_path_buf()));
    }
    if normalized.ends_with("convex/schema.ts") {
        return Some((MarkerKind::ConvexSchema, parent.parent()?.to_path_buf()));
    }
    if normalized.ends_with("supabase/config.toml") {
        return Some((MarkerKind::SupabaseConfig, parent.parent()?.to_path_buf()));
    }

    None
}

fn profile_from_markers(repository_root: &Path, markers: &[Marker]) -> ProjectProfile {
    let mut modules: BTreeMap<PathBuf, ModuleBuilder> = BTreeMap::new();
    for marker in markers {
        let module = modules.entry(marker.module_root.clone()).or_default();
        apply_marker(module, marker);
    }

    let module_profiles: Vec<ModuleProfile> = modules
        .into_iter()
        .map(|(root, mut module)| {
            module
                .evidence
                .sort_by(|left, right| left.path.cmp(&right.path));
            let fallback_name = if root.as_os_str().is_empty() {
                repository_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("repository")
                    .to_owned()
            } else {
                root.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("module")
                    .to_owned()
            };
            ModuleProfile {
                name: module.name.unwrap_or(fallback_name),
                root,
                languages: module.languages.into_iter().collect(),
                frameworks: module.frameworks.into_iter().collect(),
                database_technologies: module.database_technologies.into_iter().collect(),
                evidence: module.evidence,
            }
        })
        .collect();

    ProjectProfile {
        schema_version: SCHEMA_VERSION,
        detector_version: DETECTOR_VERSION,
        repository_root: repository_root.to_path_buf(),
        fingerprint: fingerprint(markers),
        is_monorepo: module_profiles.len() > 1,
        modules: module_profiles,
    }
}

fn apply_marker(module: &mut ModuleBuilder, marker: &Marker) {
    let text = String::from_utf8_lossy(&marker.content);
    let lower = text.to_ascii_lowercase();

    match marker.kind {
        MarkerKind::NodePackage => {
            let package: Value = serde_json::from_slice(&marker.content).unwrap_or(Value::Null);
            module.name = package
                .get("name")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| module.name.take());
            module.languages.insert(Language::JavaScript);
            if has_node_dependency(&package, "typescript") {
                module.languages.insert(Language::TypeScript);
            }
            if has_node_dependency(&package, "next") {
                module.frameworks.insert(Framework::NextJs);
            }
            if has_node_dependency(&package, "express") {
                module.frameworks.insert(Framework::Express);
            }
            if has_node_dependency(&package, "prisma")
                || has_node_dependency(&package, "@prisma/client")
            {
                module
                    .database_technologies
                    .insert(DatabaseTechnology::Prisma);
            }
            if has_node_dependency(&package, "drizzle-orm") {
                module
                    .database_technologies
                    .insert(DatabaseTechnology::Drizzle);
            }
            if has_node_dependency(&package, "typeorm") {
                module
                    .database_technologies
                    .insert(DatabaseTechnology::TypeOrm);
            }
        }
        MarkerKind::Gemfile => {
            module.languages.insert(Language::Ruby);
            if lower.contains("rails") {
                module.frameworks.insert(Framework::Rails);
                module
                    .database_technologies
                    .insert(DatabaseTechnology::ActiveRecord);
            }
        }
        MarkerKind::Cargo => {
            module.languages.insert(Language::Rust);
            if let Ok(document) = toml::from_str::<toml::Value>(&text) {
                if let Some(name) = document
                    .get("package")
                    .and_then(|package| package.get("name"))
                    .and_then(toml::Value::as_str)
                {
                    module.name = Some(name.to_owned());
                }
            }
        }
        MarkerKind::PyProject => {
            module.languages.insert(Language::Python);
            if lower.contains("django") {
                module.frameworks.insert(Framework::Django);
            }
            if lower.contains("fastapi") {
                module.frameworks.insert(Framework::FastApi);
            }
            if lower.contains("sqlalchemy") || lower.contains("sqlmodel") {
                module
                    .database_technologies
                    .insert(DatabaseTechnology::SqlAlchemy);
            }
        }
        MarkerKind::Pom | MarkerKind::Gradle => {
            module.languages.insert(Language::Java);
            if marker.relative_path.to_string_lossy().ends_with(".kts") {
                module.languages.insert(Language::Kotlin);
            }
            if lower.contains("spring-boot") || lower.contains("org.springframework.boot") {
                module.frameworks.insert(Framework::SpringBoot);
            }
        }
        MarkerKind::CsProject => {
            module.languages.insert(Language::CSharp);
            module.frameworks.insert(Framework::DotNet);
            if lower.contains("entityframeworkcore") {
                module
                    .database_technologies
                    .insert(DatabaseTechnology::EntityFramework);
            }
        }
        MarkerKind::GoMod => {
            module.languages.insert(Language::Go);
            if let Some(name) = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("module "))
            {
                module.name = Some(name.to_owned());
            }
        }
        MarkerKind::PrismaSchema
        | MarkerKind::RailsSchema
        | MarkerKind::DrizzleConfig
        | MarkerKind::ConvexSchema
        | MarkerKind::SupabaseConfig => {
            if let Some(technology) = database_technology(marker.kind) {
                module.database_technologies.insert(technology);
            }
            if matches!(
                marker.kind,
                MarkerKind::ConvexSchema | MarkerKind::DrizzleConfig
            ) {
                module.languages.insert(Language::TypeScript);
            }
        }
    }

    module.evidence.push(DetectionEvidence {
        path: marker.relative_path.clone(),
        reason: evidence_reason(marker.kind).to_owned(),
    });
}

fn has_node_dependency(package: &Value, dependency: &str) -> bool {
    ["dependencies", "devDependencies", "peerDependencies"]
        .iter()
        .any(|section| {
            package
                .get(section)
                .and_then(|dependencies| dependencies.get(dependency))
                .is_some()
        })
}

fn database_technology(kind: MarkerKind) -> Option<DatabaseTechnology> {
    match kind {
        MarkerKind::PrismaSchema => Some(DatabaseTechnology::Prisma),
        MarkerKind::RailsSchema => Some(DatabaseTechnology::ActiveRecord),
        MarkerKind::DrizzleConfig => Some(DatabaseTechnology::Drizzle),
        MarkerKind::ConvexSchema => Some(DatabaseTechnology::Convex),
        MarkerKind::SupabaseConfig => Some(DatabaseTechnology::Supabase),
        _ => None,
    }
}

fn evidence_reason(kind: MarkerKind) -> &'static str {
    match kind {
        MarkerKind::Cargo => "Rust manifest",
        MarkerKind::ConvexSchema => "Convex schema",
        MarkerKind::CsProject => ".NET project manifest",
        MarkerKind::DrizzleConfig => "Drizzle configuration",
        MarkerKind::Gemfile => "Ruby dependency manifest",
        MarkerKind::GoMod => "Go module manifest",
        MarkerKind::Gradle => "Gradle build manifest",
        MarkerKind::NodePackage => "Node package manifest",
        MarkerKind::Pom => "Maven project manifest",
        MarkerKind::PrismaSchema => "Prisma schema",
        MarkerKind::PyProject => "Python project manifest",
        MarkerKind::RailsSchema => "Rails database schema",
        MarkerKind::SupabaseConfig => "Supabase project configuration",
    }
}

fn fingerprint(markers: &[Marker]) -> String {
    let mut hasher = blake3::Hasher::new();
    for marker in markers {
        hasher.update(marker.relative_path.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        hasher.update(&marker.content);
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

fn cache_path(repository_root: &Path) -> Result<PathBuf, std::io::Error> {
    let marker = repository_root.join(".git");
    let git_directory = if marker.is_dir() {
        marker
    } else {
        let contents = fs::read_to_string(&marker)?;
        let path = contents
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} is not a valid Git directory pointer", marker.display()),
                )
            })?;
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            repository_root.join(path)
        }
    };

    Ok(git_directory
        .join("reposcribe")
        .join("project-profile.json"))
}

fn write_cache(path: &Path, profile: &ProjectProfile) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(profile)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn monorepo_fixture() -> TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        write(
            temp.path(),
            "package.json",
            r#"{"name":"acme","private":true,"workspaces":["apps/*","services/*"]}"#,
        );
        write(
            temp.path(),
            "apps/web/package.json",
            r#"{"name":"web","dependencies":{"next":"latest","typescript":"latest"}}"#,
        );
        write(
            temp.path(),
            "apps/api/package.json",
            r#"{"name":"api","dependencies":{"express":"latest","typeorm":"latest"}}"#,
        );
        write(temp.path(), "services/billing/Gemfile", "gem 'rails'\n");
        write(
            temp.path(),
            "services/billing/db/schema.rb",
            "ActiveRecord::Schema.define do\nend\n",
        );
        temp
    }

    #[test]
    fn detects_multiple_projects_in_a_monorepo() {
        let temp = monorepo_fixture();
        let profile = detect_project(temp.path()).unwrap();

        assert!(profile.is_monorepo);
        assert_eq!(profile.modules.len(), 4);
        assert!(profile.modules.iter().any(|module| {
            module.name == "web" && module.frameworks.contains(&Framework::NextJs)
        }));
        assert!(profile.modules.iter().any(|module| {
            module.name == "api"
                && module.frameworks.contains(&Framework::Express)
                && module
                    .database_technologies
                    .contains(&DatabaseTechnology::TypeOrm)
        }));
        assert!(profile.modules.iter().any(|module| {
            module.root == Path::new("services/billing")
                && module.frameworks.contains(&Framework::Rails)
                && module
                    .database_technologies
                    .contains(&DatabaseTechnology::ActiveRecord)
        }));
    }

    #[test]
    fn reuses_and_invalidates_the_private_cache() {
        let temp = monorepo_fixture();

        let first = load_or_detect_project(temp.path(), false).unwrap();
        assert_eq!(first.cache_state, CacheState::Refreshed);

        let second = load_or_detect_project(temp.path(), false).unwrap();
        assert_eq!(second.cache_state, CacheState::Hit);

        write(
            temp.path(),
            "apps/web/package.json",
            r#"{"name":"web","dependencies":{"next":"latest","prisma":"latest"}}"#,
        );
        let third = load_or_detect_project(temp.path(), false).unwrap();
        assert_eq!(third.cache_state, CacheState::Refreshed);
        assert_ne!(second.profile.fingerprint, third.profile.fingerprint);
    }

    #[test]
    fn caches_profiles_for_git_worktrees() {
        let temp = tempfile::tempdir().unwrap();
        let shared_git = tempfile::tempdir().unwrap();
        let worktree_git = shared_git.path().join("worktrees/example");
        fs::create_dir_all(&worktree_git).unwrap();
        write(
            temp.path(),
            ".git",
            &format!("gitdir: {}\n", worktree_git.display()),
        );
        write(temp.path(), "Cargo.toml", "[package]\nname = 'example'\n");

        let outcome = load_or_detect_project(temp.path(), false).unwrap();

        assert_eq!(outcome.cache_state, CacheState::Refreshed);
        assert!(
            worktree_git
                .join("reposcribe/project-profile.json")
                .is_file()
        );
    }
}
