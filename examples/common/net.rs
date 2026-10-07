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
            // like a browser: error statuses still carry a page to render
            .http_status_as_error(false)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36")
            .build();
        Self {
            agent: Agent::new_with_config(config),
        }
    }
}

impl HostFetcher {
    fn read(&mut self, mut response: ureq::http::Response<ureq::Body>, max_bytes: usize) -> Result<Response, String> {
        use std::io::Read;
        let final_url = response.get_uri().to_string();
        let status = response.status().as_u16();
        let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        let headers: Vec<(String, String)> = response.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string()))).collect();
        // truncate at the limit instead of failing
        let mut body = Vec::new();
        response.body_mut().as_reader().take(max_bytes as u64 + 1).read_to_end(&mut body).map_err(|e| e.to_string())?;
        body.truncate(max_bytes);
        Ok(Response { final_url, body, content_type, status, headers })
    }
}

impl Fetcher for HostFetcher {
    fn get(&mut self, url: &str, max_bytes: usize) -> Result<Response, String> {
        let parsed = Url::parse(url).map_err(|error| error.to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(String::from("only HTTP and HTTPS URLs are supported"));
        }
        let response = self
            .agent
            .get(url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .call()
            .map_err(|error| error.to_string())?;
        self.read(response, max_bytes)
    }

    fn request(&mut self, method: &str, url: &str, headers: &[(String, String)], body: Option<&str>, max_bytes: usize) -> Result<Response, String> {
        let parsed = Url::parse(url).map_err(|error| error.to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(String::from("only HTTP and HTTPS URLs are supported"));
        }
        let mut builder = ureq::http::Request::builder().method(method).uri(url);
        for (k, v) in headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        let req = builder.body(body.unwrap_or("").to_string()).map_err(|e| e.to_string())?;
        let response = self.agent.run(req).map_err(|error| error.to_string())?;
        self.read(response, max_bytes)
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
