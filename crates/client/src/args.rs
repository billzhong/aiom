use aiom_protocol::parse_token;
use clap::Parser;
use std::time::Duration;
use url::Url;

/// AIO Monitor client: reports the metrics of this host to an AIO Monitor server.
#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Args {
    /// Name of this node on the dashboard
    #[arg(short, long, env = "AIOM_NAME", value_parser = parse_name)]
    pub name: String,

    /// Agent URL of the server, e.g. wss://monitor.example.com/agent
    #[arg(short, long, env = "AIOM_SERVER", value_parser = parse_server_url)]
    pub server: Url,

    /// Authorization token
    #[arg(short, long, env = "AIOM_TOKEN", hide_env_values = true, value_parser = parse_token)]
    pub token: String,

    /// Sending interval in milliseconds
    #[arg(long, env = "AIOM_INTERVAL", default_value_t = 1000, value_parser = clap::value_parser!(u64).range(500..=5000))]
    pub interval: u64,

    /// Retry delay in seconds
    #[arg(long, env = "AIOM_RETRY", default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=60))]
    pub retry: u64,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,
}

impl Args {
    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval)
    }

    pub fn retry(&self) -> Duration {
        Duration::from_secs(self.retry)
    }
}

fn parse_name(s: &str) -> Result<String, String> {
    let name = s.trim();
    if name.is_empty() {
        return Err("must not be empty".into());
    }
    Ok(name.to_owned())
}

fn parse_server_url(s: &str) -> Result<Url, String> {
    let url = Url::parse(s).map_err(|e| format!("invalid URL: {e}"))?;
    match url.scheme() {
        "ws" | "wss" => Ok(url),
        scheme => Err(format!("unsupported scheme {scheme:?}, expected ws or wss")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(std::iter::once("aiom-client").chain(args.iter().copied()))
    }

    const REQUIRED: [&str; 6] = ["-n", "node", "-s", "ws://127.0.0.1:3000/agent", "-t", "tok"];

    #[test]
    fn parses_required_arguments_with_defaults() {
        let args = parse(&REQUIRED).unwrap();
        assert_eq!(args.name, "node");
        assert_eq!(args.server.as_str(), "ws://127.0.0.1:3000/agent");
        assert_eq!(args.token, "tok");
        assert_eq!(args.interval(), Duration::from_secs(1));
        assert_eq!(args.retry(), Duration::from_secs(5));
        assert!(!args.debug);
    }

    #[test]
    fn parses_long_options() {
        let args = parse(&[
            "--name",
            " web-1 ",
            "--server",
            "wss://example.com/agent",
            "--token",
            "tok",
            "--interval",
            "500",
            "--retry",
            "60",
            "--debug",
        ])
        .unwrap();
        assert_eq!(args.name, "web-1");
        assert_eq!(args.interval(), Duration::from_millis(500));
        assert_eq!(args.retry(), Duration::from_secs(60));
        assert!(args.debug);
    }

    #[test]
    fn requires_name_server_and_token() {
        assert!(parse(&[]).is_err());
        assert!(parse(&REQUIRED[..4]).is_err());
    }

    #[test]
    fn rejects_invalid_values() {
        let with = |extra: &[&str]| {
            let mut args = REQUIRED.to_vec();
            args.extend_from_slice(extra);
            parse(&args)
        };
        assert!(with(&["--interval", "499"]).is_err());
        assert!(with(&["--interval", "5001"]).is_err());
        assert!(with(&["--retry", "0"]).is_err());
        assert!(with(&["--retry", "61"]).is_err());
        assert!(with(&["-n", "  "]).is_err());
        assert!(with(&["-t", "two words"]).is_err());
        assert!(with(&["-t", "naïve"]).is_err());
        assert!(with(&["-s", "http://example.com/agent"]).is_err());
        assert!(with(&["-s", "not a url"]).is_err());
    }
}
