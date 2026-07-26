use std::{env, fmt, fs, path::PathBuf, str::FromStr};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CONFIG_DIRECTORY_ENV: &str = "REPOSCRIBE_CONFIG_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    Anthropic,
    OpenAi,
}

impl AiProvider {
    pub const ALL: [Self; 2] = [Self::Anthropic, Self::OpenAi];

    pub const fn api_key_environment_variable(self) -> &'static str {
        match self {
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::OpenAi => "OPENAI_API_KEY",
        }
    }

    pub const fn config_name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }
}

impl fmt::Display for AiProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
        })
    }
}

impl FromStr for AiProvider {
    type Err = ParseAiProviderError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => Ok(Self::Anthropic),
            "openai" | "open-ai" => Ok(Self::OpenAi),
            _ => Err(ParseAiProviderError(value.to_owned())),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("unsupported AI provider '{0}'; expected anthropic or openai")]
pub struct ParseAiProviderError(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiConfiguration {
    pub provider: AiProvider,
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub ai: Option<AiConfiguration>,
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn discover() -> Result<Self, ConfigError> {
        if let Some(directory) = env::var_os(CONFIG_DIRECTORY_ENV) {
            return Ok(Self::at(PathBuf::from(directory).join("config.toml")));
        }

        let base = BaseDirs::new().ok_or(ConfigError::NoConfigurationDirectory)?;
        Ok(Self::at(
            base.config_dir().join("reposcribe").join("config.toml"),
        ))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn load(&self) -> Result<AppConfig, ConfigError> {
        if !self.path.exists() {
            return Ok(AppConfig::default());
        }
        let contents = fs::read_to_string(&self.path).map_err(|source| ConfigError::Read {
            path: self.path.clone(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| ConfigError::Parse {
            path: self.path.clone(),
            source,
        })
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        let contents = toml::to_string_pretty(config)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&self.path, contents).map_err(|source| ConfigError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the operating system configuration directory")]
    NoConfigurationDirectory,
    #[error("could not read configuration at '{}': {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("configuration at '{}' is invalid: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("could not write configuration at '{}': {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_only_provider_and_model() {
        let directory = tempfile::tempdir().unwrap();
        let store = ConfigStore::at(directory.path().join("config.toml"));
        let expected = AppConfig {
            ai: Some(AiConfiguration {
                provider: AiProvider::OpenAi,
                model: "gpt-example".to_owned(),
            }),
        };

        store.save(&expected).unwrap();

        assert_eq!(store.load().unwrap(), expected);
        let saved = fs::read_to_string(store.path()).unwrap();
        assert!(!saved.to_ascii_lowercase().contains("api_key"));
        assert!(!saved.contains("secret"));
    }
}
