//! Direct public HTTP(S), with checked DNS results handed to the connector.
//!
//! Special-purpose ranges follow IANA's IPv4/IPv6 registries (2025-10-09).
//! Tunnel/translation destinations are denied unless their standardized embedded
//! IPv4 address is itself public. No ambient proxy can bypass the connector.
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::{ClientBuilder, Url};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub(super) enum DestinationError {
    InvalidUrl,
    Scheme,
    NonPublic,
    EmptyDns,
    RedirectLimit,
}

impl fmt::Display for DestinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidUrl => "invalid HTTP(S) destination",
            Self::Scheme => "only public HTTP(S) destinations are supported",
            Self::NonPublic => "non-public destination blocked",
            Self::EmptyDns => "destination has no public DNS addresses",
            Self::RedirectLimit => "public HTTP redirect limit exceeded",
        })
    }
}
impl std::error::Error for DestinationError {}

pub(super) fn parse_url(raw: &str) -> Result<Url, DestinationError> {
    let url = Url::parse(raw).map_err(|_| DestinationError::InvalidUrl)?;
    validate_url(&url)?;
    Ok(url)
}

pub(super) fn validate_url(url: &Url) -> Result<(), DestinationError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DestinationError::Scheme);
    }
    let host = url.host_str().ok_or(DestinationError::InvalidUrl)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let dns_host = host.trim_end_matches('.');
    if dns_host.eq_ignore_ascii_case("localhost")
        || dns_host.to_ascii_lowercase().ends_with(".localhost")
    {
        return Err(DestinationError::NonPublic);
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public(ip) {
            return Err(DestinationError::NonPublic);
        }
    }
    Ok(())
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, d] = ip.octets();
    if [a, b, c] == [192, 0, 0] && matches!(d, 9 | 10) {
        return true;
    }
    !(matches!(a, 0 | 10 | 127 | 224..=255)
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && matches!(c, 0 | 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && matches!(b, 18 | 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let s = ip.segments();
    // The well-known NAT64 prefix has one unambiguous embedded IPv4 target.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return public_v4(Ipv4Addr::new(
            (s[6] >> 8) as u8,
            s[6] as u8,
            (s[7] >> 8) as u8,
            s[7] as u8,
        ));
    }
    // Native global unicast only: this excludes unspecified, local, multicast,
    // discard, local translation and future/reserved allocations.
    if s[0] & 0xe000 != 0x2000 {
        return false;
    }
    if s[0] == 0x2002 || (s[0] == 0x2001 && s[1] == 0xdb8) || (s[0] == 0x3fff && s[1] & 0xf000 == 0)
    {
        return false;
    }
    if s[0] == 0x2001 && s[1] < 0x200 {
        // More-specific globally reachable assignments within 2001::/23.
        return (s[1] == 1 && s[2..7] == [0, 0, 0, 0, 0] && matches!(s[7], 1..=3))
            || s[1] == 3
            || (s[1] == 4 && s[2] == 0x112)
            || (s[1] & 0xfff0 == 0x20)
            || (s[1] & 0xfff0 == 0x30);
    }
    true
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

fn checked_addresses(
    addresses: impl Iterator<Item = SocketAddr>,
) -> Result<Vec<SocketAddr>, DestinationError> {
    let addresses: Vec<_> = addresses.collect();
    if addresses.is_empty() {
        return Err(DestinationError::EmptyDns);
    }
    if addresses.iter().any(|address| !is_public(address.ip())) {
        return Err(DestinationError::NonPublic);
    }
    Ok(addresses)
}

struct PublicDns;
impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        // These boxes are reqwest's required resolver API, not a type-erased
        // application boundary. The connector receives these vetted addresses
        // directly; it does not perform a second DNS lookup after validation.
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((name.as_str(), 0)).await?;
            let addresses = checked_addresses(addresses)?;
            let addresses: Addrs = Box::new(addresses.into_iter());
            Ok(addresses)
        })
    }
}

pub(super) fn client_builder() -> ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .dns_resolver(PublicDns)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error(DestinationError::RedirectLimit);
            }
            match validate_url(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(error) => attempt.error(error),
            }
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_aliases_local_and_special_destinations_are_rejected() {
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/a",
            "http://localhost/",
            "http://x.localhost./",
            "http://127.0.0.1/",
            "http://2130706433/",
            "http://0177.0.0.1/",
            "http://0x7f000001/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[64:ff9b::a00:1]/",
            "http://169.254.169.254/",
            "http://10.0.0.1/",
            "http://100.64.0.1/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
            "http://224.0.0.1/",
            "http://240.0.0.1/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[ff02::1]/",
            "http://[2001:db8::1]/",
            "http://[3fff::1]/",
            "http://[2002:7f00:1::]/",
        ] {
            assert!(parse_url(url).is_err(), "{url}");
        }
    }
    #[test]
    fn public_addresses_and_public_registry_exceptions_remain_usable() {
        for url in [
            "https://example.com/a?q=b",
            "http://8.8.8.8:8080/",
            "http://192.0.0.9/",
            "http://192.0.0.10/",
            "http://192.31.196.1/",
            "http://[2606:4700:4700::1111]/",
            "http://[::ffff:8.8.8.8]/",
            "http://[64:ff9b::808:808]/",
            "http://[2001:1::3]/",
            "http://[2001:3::1]/",
            "http://[2001:4:112::1]/",
            "http://[2001:20::1]/",
            "http://[2001:30::1]/",
        ] {
            assert!(parse_url(url).is_ok(), "{url}");
        }
    }
    #[test]
    fn dns_results_are_all_checked_and_returned_without_re_resolution() {
        let public: SocketAddr = "8.8.8.8:0".parse().unwrap();
        let private: SocketAddr = "127.0.0.1:0".parse().unwrap();
        assert_eq!(
            checked_addresses([public].into_iter()).unwrap(),
            vec![public]
        );
        for addresses in [
            vec![],
            vec![private],
            vec![public, private],
            vec![private, public],
        ] {
            assert!(checked_addresses(addresses.into_iter()).is_err());
        }
    }
    #[tokio::test]
    async fn production_resolver_rejects_localhost_without_an_http_connection() {
        use std::str::FromStr;
        assert!(PublicDns
            .resolve(Name::from_str("localhost").unwrap())
            .await
            .is_err());
    }
}
