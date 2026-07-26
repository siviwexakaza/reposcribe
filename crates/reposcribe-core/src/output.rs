use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Pdf,
    Html,
    Markdown,
}

impl OutputFormat {
    pub const ALL: [Self; 3] = [Self::Pdf, Self::Html, Self::Markdown];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Html => "html",
            Self::Markdown => "md",
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Pdf => "pdf",
            Self::Html => "html",
            Self::Markdown => "markdown",
        };
        formatter.write_str(value)
    }
}

impl FromStr for OutputFormat {
    type Err = ParseOutputFormatError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "pdf" => Ok(Self::Pdf),
            "html" => Ok(Self::Html),
            "markdown" | "md" => Ok(Self::Markdown),
            _ => Err(ParseOutputFormatError(value.to_owned())),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("unsupported output format '{0}'; expected pdf, html, or markdown")]
pub struct ParseOutputFormatError(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_is_the_default() {
        assert_eq!(OutputFormat::default(), OutputFormat::Pdf);
        assert_eq!(OutputFormat::default().extension(), "pdf");
    }

    #[test]
    fn parses_only_supported_formats() {
        assert_eq!("html".parse(), Ok(OutputFormat::Html));
        assert_eq!("md".parse(), Ok(OutputFormat::Markdown));
        assert!("svg".parse::<OutputFormat>().is_err());
        assert!("png".parse::<OutputFormat>().is_err());
        assert!("json".parse::<OutputFormat>().is_err());
    }
}
