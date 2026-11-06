use crate::{HostError, HostResult};
use std::net::Ipv6Addr;

/// Public service location only. Device credentials must remain bound to an origin.
pub fn normalize_signaling_service(input: &str) -> HostResult<Option<String>> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let invalid = || {
        HostError::new(
            "invalid_signaling_service",
            "请输入 IP、域名或 HTTPS 地址；可指定端口，不支持路径、账号或查询参数",
        )
    };
    if input.chars().any(char::is_whitespace) || input.contains('\\') {
        return Err(invalid());
    }
    let explicit_scheme = input.contains("://");
    let value = if input.parse::<Ipv6Addr>().is_ok() {
        format!("https://[{input}]:8443")
    } else if explicit_scheme {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let mut url = url::Url::parse(&value).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.port_or_known_default() == Some(0)
    {
        return Err(invalid());
    }
    // Explicit HTTPS URLs follow HTTPS's default port; bare hosts use CodePet's port.
    // Preserve an explicitly supplied :443 even though Url normalizes it away.
    if !explicit_scheme && url.port().is_none() && !input.trim_end_matches('/').ends_with(":443") {
        url.set_port(Some(8443)).map_err(|_| invalid())?;
    }
    Ok(Some(url.as_str().trim_end_matches('/').to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signaling_addresses_accept_ip_domain_and_explicit_https_port() {
        for (input, expected) in [
            (" 172.96.254.12 ", "https://172.96.254.12:8443"),
            ("signal.example.com", "https://signal.example.com:8443"),
            ("signal.example.com:443", "https://signal.example.com"),
            ("signal.example.com:9443", "https://signal.example.com:9443"),
            ("https://signal.example.com/", "https://signal.example.com"),
            ("2001:db8::1", "https://[2001:db8::1]:8443"),
            ("[2001:db8::1]:9443", "https://[2001:db8::1]:9443"),
        ] {
            assert_eq!(
                normalize_signaling_service(input).unwrap().as_deref(),
                Some(expected)
            );
        }
        assert_eq!(normalize_signaling_service(" ").unwrap(), None);
    }

    #[test]
    fn signaling_addresses_reject_ambiguous_or_credential_bearing_urls() {
        for input in [
            "http://example.com",
            "https://user:pass@example.com",
            "example.com/api",
            "example.com?token=secret",
            "example.com#fragment",
            "host name",
            "host:0",
            "host:65536",
            "https://",
            "example.com\\evil",
        ] {
            assert!(normalize_signaling_service(input).is_err(), "{input}");
        }
    }
}
