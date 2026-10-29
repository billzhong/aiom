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

    /// Directory of the built dashboard (`web/dist`) to serve on all other routes, instead of
    /// the one built into the binary, if any.
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
        validate_route(&self.dashboard_path)
            .map_err(|error| format!("invalid dashboard path: {error}"))?;
        validate_route(&self.agent_path).map_err(|error| format!("invalid agent path: {error}"))?;
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
    validate_route(s)?;
    Ok(s.to_owned())
}

fn validate_route(s: &str) -> Result<(), String> {
    if !s.starts_with('/') || s.len() < 2 {
        return Err("must start with `/` and not be the root path".into());
    }
    // RFC 3986 path characters, which clients send unchanged; this leaves out the `{}` capture
    // syntax of axum and anything a browser would percent-encode.
    if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@%/".contains(&b))
    {
        return Err("must be a plain path".into());
    }
    // Browsers and the client URL parser remove `.` and `..` segments, including encoded dots,
    // before sending a request, so a route the parser rewrites cannot be reached literally. The
    // characters above are ones it leaves alone, so dot segments are all it can change.
    if !reqwest::Url::parse(&format!("http://h{s}")).is_ok_and(|url| url.path() == s) {
        return Err(
            "must not contain `.` or `..` path segments, including percent-encoded dots".into(),
        );
    }
    Ok(())
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
        assert!(parse_route("/:ws").is_ok());
        assert!(parse_route("/api/*").is_ok());
        assert!(parse_route("agent").is_err());
        assert!(parse_route("/").is_err());
        assert!(parse_route("/{id}").is_err());
        assert!(parse_route("/a b").is_err());
        assert!(parse_route("/仪表盘").is_err());
    }

    #[test]
    fn rejects_dot_segments_in_configured_routes() {
        for segment in [
            ".", "..", "%2e", "%2E", ".%2e", ".%2E", "%2e.", "%2E.", "%2e%2e", "%2E%2e", "%2e%2E",
            "%2E%2E",
        ] {
            for route in [
                format!("/{segment}/ws"),
                format!("/api/{segment}/ws"),
                format!("/api/{segment}"),
            ] {
                assert!(parse_route(&route).is_err(), "{route}");
            }
        }
    }

    #[test]
    fn accepts_dots_within_normal_route_segments() {
        for route in [
            "/.well-known/ws",
            "/api/v1.0/ws",
            "/.../ws",
            "/%2e%2e%2e/ws",
            "/name%2eext/ws",
            "/%252e/ws",
        ] {
            assert_eq!(parse_route(route).unwrap(), route);
        }
    }

    #[test]
    fn validates_routes_set_without_clap() {
        let mut config = parse(&["--token", "t"]).unwrap();
        config.dashboard_path = "/api/../ws".into();
        assert!(config.validate().is_err());

        config.dashboard_path = "/ws".into();
        config.agent_path = "/%2e/agent".into();
        assert!(config.validate().is_err());
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
