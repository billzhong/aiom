//! The public address a client connects from.

use aiom_protocol::is_public;
use axum::http::HeaderMap;
use std::net::{IpAddr, SocketAddr};

/// Returns the public address of a client connecting from `peer`: the first public address in
/// `X-Forwarded-For`, else in `X-Real-IP`, else `peer` itself.
///
/// The headers are trusted from any peer. Only authenticated clients come here, and they can
/// report any address they like anyway.
pub(crate) fn public_addr(headers: &HeaderMap, peer: Option<IpAddr>) -> Option<IpAddr> {
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .chain(headers.get_all("x-real-ip"))
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(parse_addr);
    forwarded.chain(peer).find(|&ip| is_public(ip))
}

/// Parses an address as proxies write it, with or without a port.
fn parse_addr(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    s.parse()
        .ok()
        .or_else(|| s.parse::<SocketAddr>().ok().map(|addr| addr.ip()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for &(name, value) in pairs {
            headers.append(name, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn uses_the_peer_without_proxy_headers() {
        let peer = Some(ip("198.51.100.1"));
        assert_eq!(public_addr(&HeaderMap::new(), peer), peer);
        assert_eq!(public_addr(&HeaderMap::new(), Some(ip("10.0.0.2"))), None);
        assert_eq!(public_addr(&HeaderMap::new(), None), None);
    }

    #[test]
    fn prefers_forwarded_addresses() {
        let peer = Some(ip("127.0.0.1"));
        let forwarded = headers(&[("x-forwarded-for", "203.0.113.7")]);
        assert_eq!(public_addr(&forwarded, peer), Some(ip("203.0.113.7")));

        let real_ip = headers(&[("x-real-ip", "2001:db8::7")]);
        assert_eq!(public_addr(&real_ip, peer), Some(ip("2001:db8::7")));

        let both = headers(&[
            ("x-real-ip", "198.51.100.2"),
            ("x-forwarded-for", "203.0.113.7"),
        ]);
        assert_eq!(public_addr(&both, peer), Some(ip("203.0.113.7")));
    }

    #[test]
    fn skips_local_and_malformed_forwarded_entries() {
        let chain = headers(&[
            ("x-forwarded-for", "unknown, 192.168.0.5"),
            ("x-forwarded-for", " 203.0.113.7:4711 , 198.51.100.9"),
        ]);
        assert_eq!(public_addr(&chain, None), Some(ip("203.0.113.7")));

        let local = headers(&[("x-forwarded-for", "10.0.0.1")]);
        let peer = Some(ip("198.51.100.1"));
        assert_eq!(public_addr(&local, peer), peer);
    }
}
