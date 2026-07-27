use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceDiagram {
    pub name: String,
    pub entry: String,
    pub source_files: Vec<PathBuf>,
    pub mermaid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowDiagram {
    pub name: String,
    pub entry: String,
    pub source_files: Vec<PathBuf>,
    pub mermaid: String,
}
