use aiom_protocol::parse_token;
use clap::Parser;

/// AIO Monitor server.
///
/// Every option can also be set through the environment variable shown, or a `.env` file in
/// the working directory.
#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Config {
    /// Token clients must present as `Authorization: Bearer <token>`.
    #[arg(long, env = "AIOM_TOKEN", hide_env_values = true, value_parser = parse_token)]
    pub token: String,

    /// Address to listen on.
    #[arg(long, env = "AIOM_LISTEN", default_value = "0.0.0.0:3000")]
    pub listen: String,

    /// Route of the dashboard WebSocket.
    #[arg(long, env = "AIOM_DASHBOARD_PATH", default_value = "/ws", value_parser = parse_route)]
    pub dashboard_path: String,

    /// Route of the client WebSocket.
    #[arg(long, env = "AIOM_AGENT_PATH", default_value = "/agent", value_parser = parse_route)]
    pub agent_path: String,

    /// Optional ipinfo.io token, which raises the anonymous rate limit of IP geolocation.
    #[arg(long, env = "AIOM_IPINFO_TOKEN", hide_env_values = true)]
    pub ipinfo_token: Option<String>,

    /// Directory of the built dashboard (`web/dist`) to serve on all other routes.
    #[arg(long, env = "AIOM_WEB_DIR")]
    pub web_dir: Option<String>,
}

impl Config {
    /// The ipinfo.io token, treating an empty value as unset.
    pub fn ipinfo_token(&self) -> Option<&str> {
        self.ipinfo_token.as_deref().filter(|t| !t.is_empty())
    }

    /// The dashboard directory, treating an empty value as unset.
    pub fn web_dir(&self) -> Option<&str> {
        self.web_dir.as_deref().filter(|d| !d.is_empty())
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.dashboard_path == self.agent_path {
            return Err(format!(
                "the dashboard and agent paths must differ (both are {:?})",
                self.dashboard_path
            ));
        }
        if self.dashboard_path == crate::HEALTH_PATH || self.agent_path == crate::HEALTH_PATH {
            return Err(format!("{} is reserved", crate::HEALTH_PATH));
        }
        Ok(())
    }
}

fn parse_route(s: &str) -> Result<String, String> {
    if !s.starts_with('/') || s.len() < 2 {
        return Err("must start with `/` and not be the root path".into());
    }
    if s.contains(['{', '}', '*', '?', '#']) || s.chars().any(char::is_whitespace) {
        return Err("must be a plain path".into());
    }
    Ok(s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Config, clap::Error> {
        Config::try_parse_from(std::iter::once("aiom-server").chain(args.iter().copied()))
    }

    #[test]
    fn defaults_match_documented_values() {
        let config = parse(&["--token", "secret"]).unwrap();
        assert_eq!(config.token, "secret");
        assert_eq!(config.listen, "0.0.0.0:3000");
        assert_eq!(config.dashboard_path, "/ws");
        assert_eq!(config.agent_path, "/agent");
        assert_eq!(config.ipinfo_token(), None);
        assert_eq!(config.web_dir(), None);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn empty_optional_values_are_unset() {
        let config = parse(&["--token", "t", "--ipinfo-token", "", "--web-dir", ""]).unwrap();
        assert_eq!(config.ipinfo_token(), None);
        assert_eq!(config.web_dir(), None);
    }

    #[test]
    fn rejects_invalid_tokens() {
        assert!(parse(&["--token", ""]).is_err());
        assert!(parse(&["--token", "has space"]).is_err());
        assert!(parse(&["--token", "naïve"]).is_err());
    }

    #[test]
    fn rejects_invalid_routes() {
        assert!(parse_route("/agent").is_ok());
        assert!(parse_route("/api/v1/ws").is_ok());
        assert!(parse_route("agent").is_err());
        assert!(parse_route("/").is_err());
        assert!(parse_route("/{id}").is_err());
        assert!(parse_route("/a b").is_err());
    }

    #[test]
    fn rejects_conflicting_routes() {
        let config = parse(&[
            "--token",
            "t",
            "--dashboard-path",
            "/x",
            "--agent-path",
            "/x",
        ])
        .unwrap();
        assert!(config.validate().is_err());
        let config = parse(&["--token", "t", "--dashboard-path", "/healthz"]).unwrap();
        assert!(config.validate().is_err());
    }
}
