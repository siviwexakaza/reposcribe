use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSchema {
    pub name: String,
    pub source_files: Vec<PathBuf>,
    pub entities: Vec<DatabaseEntity>,
    pub relationships: Vec<DatabaseRelationship>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryDatabaseAnalysis {
    pub projects: Vec<DatabaseProjectAnalysis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseProjectAnalysis {
    pub name: String,
    pub root: PathBuf,
    pub framework: Option<String>,
    pub database_technology: Option<String>,
    pub schema: DatabaseSchema,
}

impl std::fmt::Display for DatabaseProjectAnalysis {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} · {}", self.name, self.root.display())?;
        if let Some(database) = &self.database_technology {
            write!(formatter, " · {database}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseEntity {
    pub name: String,
    pub fields: Vec<DatabaseField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseField {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseRelationship {
    pub from_entity: String,
    pub from_field: Option<String>,
    pub to_entity: String,
    pub to_field: Option<String>,
    pub from_cardinality: Cardinality,
    pub to_cardinality: Cardinality,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    One,
    ZeroOrOne,
    Many,
    ZeroOrMany,
}

impl Cardinality {
    pub const fn mermaid_left(self) -> &'static str {
        match self {
            Self::One => "||",
            Self::ZeroOrOne => "|o",
            Self::Many => "}|",
            Self::ZeroOrMany => "}o",
        }
    }

    pub const fn mermaid_right(self) -> &'static str {
        match self {
            Self::One => "||",
            Self::ZeroOrOne => "o|",
            Self::Many => "|{",
            Self::ZeroOrMany => "o{",
        }
    }
}
