use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    CSharp,
    Go,
    Java,
    JavaScript,
    Kotlin,
    Python,
    Ruby,
    Rust,
    TypeScript,
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CSharp => "C#",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::JavaScript => "JavaScript",
            Self::Kotlin => "Kotlin",
            Self::Python => "Python",
            Self::Ruby => "Ruby",
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Framework {
    DotNet,
    Django,
    Express,
    FastApi,
    NextJs,
    Rails,
    SpringBoot,
}

impl fmt::Display for Framework {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DotNet => ".NET",
            Self::Django => "Django",
            Self::Express => "Express",
            Self::FastApi => "FastAPI",
            Self::NextJs => "Next.js",
            Self::Rails => "Rails",
            Self::SpringBoot => "Spring Boot",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseTechnology {
    ActiveRecord,
    Convex,
    Drizzle,
    EntityFramework,
    Prisma,
    SqlAlchemy,
    Supabase,
    TypeOrm,
}

impl fmt::Display for DatabaseTechnology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ActiveRecord => "Active Record",
            Self::Convex => "Convex",
            Self::Drizzle => "Drizzle",
            Self::EntityFramework => "Entity Framework",
            Self::Prisma => "Prisma",
            Self::SqlAlchemy => "SQLAlchemy",
            Self::Supabase => "Supabase",
            Self::TypeOrm => "TypeORM",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionEvidence {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleProfile {
    pub name: String,
    /// Path relative to the repository root. An empty path represents the root module.
    pub root: PathBuf,
    pub languages: Vec<Language>,
    pub frameworks: Vec<Framework>,
    pub database_technologies: Vec<DatabaseTechnology>,
    pub evidence: Vec<DetectionEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectProfile {
    pub schema_version: u32,
    pub detector_version: u32,
    pub repository_root: PathBuf,
    pub fingerprint: String,
    pub is_monorepo: bool,
    pub modules: Vec<ModuleProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheState {
    Hit,
    Refreshed,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct ProjectProfileOutcome {
    pub profile: ProjectProfile,
    pub cache_state: CacheState,
    pub warning: Option<String>,
}
