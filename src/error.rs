use std::fmt;

#[derive(Debug)]
pub enum CliError {
    Http(reqwest::Error),
    Io(std::io::Error),
    Api { status: u16, message: String },
    Config(String),
    Other(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Http(e) => write!(f, "HTTP error: {}", e),
            CliError::Io(e) => write!(f, "IO error: {}", e),
            CliError::Api { status, message } => write!(f, "API error ({}): {}", status, message),
            CliError::Config(msg) => write!(f, "Config error: {}", msg),
            CliError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for CliError {}

impl From<reqwest::Error> for CliError {
    fn from(e: reqwest::Error) -> Self {
        CliError::Http(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, CliError>;
