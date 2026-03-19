use crate::config::CliConfig;
use crate::error::{CliError, Result};
use reqwest::header::{HeaderMap, HeaderValue};

pub struct ApiClient {
    pub client: reqwest::Client,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl ApiClient {
    pub fn new(config: &CliConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert("User-Agent", HeaderValue::from_static("sany-cli/0.1.0"));

        if let Some(ref key) = config.api_key {
            headers.insert(
                "X-API-Key",
                HeaderValue::from_str(key).map_err(|_| CliError::Config("Invalid API key".into()))?,
            );
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            base_url: config.server_url(),
            api_key: config.api_key.clone(),
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    pub fn is_authenticated(&self) -> bool {
        self.api_key.is_some()
    }
}
