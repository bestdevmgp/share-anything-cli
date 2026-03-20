use crate::config::CliConfig;
use crate::error::{CliError, Result};
use reqwest::header::{HeaderMap, HeaderValue};

pub struct ApiClient {
    pub client: reqwest::Client,
    pub base_url: String,
    pub token: Option<String>,
}

impl ApiClient {
    pub fn new(config: &CliConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HeaderValue::from_str(&format!("share-cli/{}", env!("CARGO_PKG_VERSION"))).unwrap());

        if let Some(ref token) = config.token {
            headers.insert(
                "X-Personal-Token",
                HeaderValue::from_str(token).map_err(|_| CliError::Config("Invalid personal token".into()))?,
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            base_url: config.server_url(),
            token: config.token.clone(),
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }
}
