//! Socket-rooted source IP resolution for security policy, not arbitrary identity headers.
use ipnet::IpNet;
use std::net::IpAddr;

const MAX_PROXIES: usize = 32;
const MAX_HOPS: usize = 32;
const MAX_HEADER_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SourceIpError {
    #[error("invalid trusted proxy configuration")]
    Configuration,
    #[error("invalid trusted proxy chain")]
    InvalidChain,
}

#[derive(Clone, Default)]
pub struct TrustedProxyChain {
    proxies: Vec<IpNet>,
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip.into()),
        ip => ip,
    }
}

impl TrustedProxyChain {
    /// No wildcard/trust-all, default private networks or hostname resolution.
    pub fn new(cidrs: &[String]) -> Result<Self, SourceIpError> {
        if cidrs.len() > MAX_PROXIES {
            return Err(SourceIpError::Configuration);
        }
        let proxies = cidrs
            .iter()
            .map(|cidr| {
                let net: IpNet = cidr.parse().map_err(|_| SourceIpError::Configuration)?;
                if net.prefix_len() == 0 || net.to_string() != *cidr || net.trunc() != net {
                    return Err(SourceIpError::Configuration);
                }
                Ok(net)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { proxies })
    }

    fn trusted(&self, ip: IpAddr) -> bool {
        self.proxies.iter().any(|net| net.contains(&ip))
    }

    /// Caller rejects duplicate X-Forwarded-For headers. An untrusted socket peer
    /// cannot change its identity with any forwarded header (even a malformed one).
    /// Trusted proxies MUST append their actual peer, or replace the incoming chain.
    pub fn resolve(
        &self,
        peer: IpAddr,
        forwarded_for: Option<&str>,
    ) -> Result<IpAddr, SourceIpError> {
        let mut current = canonical(peer);
        if !self.trusted(current) {
            return Ok(current);
        }
        let chain = forwarded_for.ok_or(SourceIpError::InvalidChain)?;
        if chain.len() > MAX_HEADER_BYTES || chain.split(',').count() > MAX_HOPS {
            return Err(SourceIpError::InvalidChain);
        }
        let hops = chain
            .split(',')
            .map(|hop| {
                hop.trim()
                    .parse::<IpAddr>()
                    .map(canonical)
                    .map_err(|_| SourceIpError::InvalidChain)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for hop in hops.into_iter().rev() {
            if !self.trusted(current) {
                break;
            }
            current = hop;
        }
        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }
    fn proxies() -> TrustedProxyChain {
        TrustedProxyChain::new(&["10.1.0.0/16".into(), "2001:db8:1::/48".into()]).unwrap()
    }
    #[test]
    fn untrusted_peer_ignores_every_forwarded_claim() {
        for header in [None, Some("127.0.0.1"), Some("unknown,broken")] {
            assert_eq!(
                proxies().resolve(ip("192.0.2.1"), header),
                Ok(ip("192.0.2.1"))
            );
        }
        assert_eq!(
            TrustedProxyChain::default().resolve(ip("10.1.2.3"), None),
            Ok(ip("10.1.2.3"))
        );
    }
    #[test]
    fn first_untrusted_hop_from_socket_wins_not_attacker_leftmost() {
        assert_eq!(
            proxies().resolve(ip("10.1.0.2"), Some("127.0.0.1, 192.0.2.3, 10.1.0.1")),
            Ok(ip("192.0.2.3"))
        );
        assert_eq!(
            proxies().resolve(ip("2001:db8:1::2"), Some("2001:db8:2::1, 2001:db8:1::1")),
            Ok(ip("2001:db8:2::1"))
        );
        assert_eq!(
            proxies().resolve(ip("::ffff:10.1.0.2"), Some("::ffff:192.0.2.3")),
            Ok(ip("192.0.2.3"))
        );
    }
    #[test]
    fn trusted_peer_missing_or_ambiguous_chain_fails_closed() {
        for header in [
            None,
            Some(""),
            Some("unknown"),
            Some("192.0.2.1,"),
            Some("192.0.2.1:80"),
            Some("[2001:db8::1]"),
            Some("\"192.0.2.1\""),
        ] {
            assert_eq!(
                proxies().resolve(ip("10.1.0.2"), header),
                Err(SourceIpError::InvalidChain)
            );
        }
        let long = ["10.1.0.1"; MAX_HOPS + 1].join(",");
        assert_eq!(
            proxies().resolve(ip("10.1.0.2"), Some(&long)),
            Err(SourceIpError::InvalidChain)
        );
        assert_eq!(
            proxies().resolve(ip("10.1.0.2"), Some(&"1".repeat(MAX_HEADER_BYTES + 1))),
            Err(SourceIpError::InvalidChain)
        );
    }
    #[test]
    fn config_is_explicit_bounded_canonical_and_never_trust_all() {
        for cidr in [
            "*",
            "0.0.0.0/0",
            "::/0",
            "10.1.1.1/16",
            "10.1.0.0",
            " 10.1.0.0/16",
        ] {
            assert!(matches!(
                TrustedProxyChain::new(&[cidr.into()]),
                Err(SourceIpError::Configuration)
            ));
        }
        assert!(matches!(
            TrustedProxyChain::new(&vec!["10.1.0.0/16".into(); MAX_PROXIES + 1]),
            Err(SourceIpError::Configuration)
        ));
    }
}
