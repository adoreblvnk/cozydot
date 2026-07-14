use anyhow::{bail, Context, Result};
use serde::{de, Deserialize, Deserializer};
use std::fmt;
use url::{Host, Url};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl(Url);

impl HttpsUrl {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        validate_non_empty(value)?;
        if value.chars().any(char::is_whitespace)
            || value.chars().any(char::is_control)
            || value.contains('\\')
            || has_substitution(value)
        {
            bail!("invalid HTTPS URL {value:?}; must be literal and contain no whitespace or substitutions");
        }
        let parsed = Url::parse(value).with_context(|| {
            format!("invalid HTTPS URL {value:?}; must be a valid absolute URL")
        })?;
        let (raw_scheme, remainder) = value.split_once("://").unwrap_or_default();
        let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
        let host_port = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let (raw_host, empty_port) = if let Some(rest) = host_port.strip_prefix('[') {
            let closing = rest.find(']').map(|index| index + 1);
            let raw_host = closing.map_or(host_port, |index| &host_port[..=index]);
            let suffix = closing.map_or("", |index| &host_port[index + 1..]);
            (raw_host, suffix == ":")
        } else if let Some((host, port)) = host_port.rsplit_once(':') {
            (host, port.is_empty())
        } else {
            (host_port, false)
        };
        let invalid_host = parsed.host().is_none_or(|host| match host {
            Host::Ipv4(address) => raw_host != address.to_string(),
            Host::Ipv6(_) => false,
            Host::Domain(domain) => !valid_domain_host(domain),
        });
        if raw_scheme != "https"
            || parsed.scheme() != "https"
            || authority.is_empty()
            || authority.contains('%')
            || empty_port
            || invalid_host
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || authority.contains('@')
            || parsed.fragment().is_some()
        {
            bail!("invalid HTTPS URL {value:?}; must use HTTPS with a non-empty host and no credentials or fragment");
        }
        Ok(Self(parsed))
    }
}

impl<'de> Deserialize<'de> for HttpsUrl {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictStringVisitor;

        impl de::Visitor<'_> for StrictStringVisitor {
            type Value = String;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a YAML string")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
                Ok(value.to_owned())
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
                Ok(value)
            }
        }

        let value = deserializer.deserialize_any(StrictStringVisitor)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}

fn validate_non_empty(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("invalid HTTPS URL {value:?}; must be a non-empty string");
    }
    Ok(())
}

fn valid_domain_host(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            label.len() <= 63
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn has_substitution(value: &str) -> bool {
    value.contains('$') || value.contains("{{") || value.contains("{%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_canonicalizes_literal_https_urls() {
        let url = HttpsUrl::parse("https://münchen.example/path?q=1").unwrap();
        assert_eq!(url.as_str(), "https://xn--mnchen-3ya.example/path?q=1");
    }

    #[test]
    fn rejects_non_https_and_non_literal_urls() {
        for invalid in [
            "http://example.com",
            "https://user@example.com",
            "https://example.com/path#fragment",
            "https://example.com/${ARCH}",
        ] {
            let error = HttpsUrl::parse(invalid).unwrap_err().to_string();
            assert!(error.contains(invalid), "missing {invalid:?} in {error:?}");
        }
    }

    #[test]
    fn serde_requires_a_string_and_applies_canonicalization() {
        let url: HttpsUrl = serde_yaml::from_str("https://münchen.example").unwrap();
        assert_eq!(url.as_str(), "https://xn--mnchen-3ya.example/");
        assert!(serde_yaml::from_str::<HttpsUrl>("true").is_err());
    }
}
