use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("Authentication required")]
    Unauthenticated,
    #[error("P2P error: {0}")]
    P2P(String),
    #[error("{0}")]
    Other(String),
}

