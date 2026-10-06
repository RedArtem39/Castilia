use castilia::network::{Fetcher, Response};
use std::time::Duration;
use ureq::{Agent, ResponseExt};
use url::Url;

pub struct HostFetcher {
    agent: Agent,
}

impl HostFetcher {
    pub fn new() -> Self {
        let config = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent("Castilia/0.1 (local engine test)")
            .build();
        Self {
            agent: Agent::new_with_config(config),
        }
    }
}

impl Fetcher for HostFetcher {
    fn get(&mut self, url: &str, max_bytes: usize) -> Result<Response, String> {
        let parsed = Url::parse(url).map_err(|error| error.to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(String::from("only HTTP and HTTPS URLs are supported"));
        }
        let mut response = self
            .agent
            .get(url)
            .call()
            .map_err(|error| error.to_string())?;
        let final_url = response.get_uri().to_string();
        let limit = u64::try_from(max_bytes).map_err(|error| error.to_string())?;
        let body = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(|error| error.to_string())?;
        Ok(Response { final_url, body })
    }

    fn resolve(&self, base: &str, reference: &str) -> Result<String, String> {
        let url = Url::parse(base)
            .and_then(|url| url.join(reference))
            .map_err(|error| error.to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(String::from("only HTTP and HTTPS URLs are supported"));
        }
        Ok(url.to_string())
    }
}
