use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheme {
    Https,
    #[cfg(test)]
    Http,
}

impl Scheme {
    fn default_port(self) -> u16 {
        match self {
            Self::Https => 443,
            #[cfg(test)]
            Self::Http => 80,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Https => "https",
            #[cfg(test)]
            Self::Http => "http",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Host {
    Domain(String),
    Ip(IpAddr),
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(name) => f.write_str(name),
            Self::Ip(IpAddr::V4(ip)) => write!(f, "{ip}"),
            Self::Ip(IpAddr::V6(ip)) => write!(f, "[{ip}]"),
        }
    }
}

/// An origin as the scope names it: scheme, host and port, with nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub(crate) scheme: Scheme,
    pub(crate) host: Host,
    pub(crate) port: u16,
}

impl Origin {
    /// Parses `https://host[:port]`, optionally with a trailing `/`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let url = Url::parse(text, false)?;
        if url.path_and_query != "/" || text.contains('?') || text.contains('#') {
            return Err(format!(
                "`{text}` must be an origin, without a path, query or fragment"
            ));
        }
        Ok(url.origin)
    }

    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn parse_allowing_http(text: &str) -> Result<Self, String> {
        Ok(Url::parse(text, true)?.origin)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme.as_str(), self.host)?;
        if self.port != self.scheme.default_port() {
            write!(f, ":{}", self.port)?;
        }
        Ok(())
    }
}

/// An absolute URL in the one canonical spelling Dupin hands curl. Anything that two parsers
/// could read differently (user info, backslashes, numeric host shorthands, spaces, non-ASCII) is
/// rejected rather than rewritten, so the host Dupin checked is the host curl connects to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Url {
    pub(crate) origin: Origin,
    path_and_query: String,
}

impl Url {
    #[cfg_attr(not(test), allow(unused_variables))]
    pub(crate) fn parse(text: &str, allow_http: bool) -> Result<Self, String> {
        let invalid = |why: &str| format!("`{}` is not a valid URL: {why}", truncate(text));
        if text.len() > 8192 {
            return Err(invalid("longer than 8 KiB"));
        }
        if let Some(c) = text.chars().find(|c| !c.is_ascii_graphic()) {
            return Err(invalid(&format!("contains {c:?}")));
        }
        if text.contains('\\') {
            return Err(invalid("contains a backslash"));
        }
        let (scheme, rest) = text
            .split_once("://")
            .ok_or_else(|| invalid("it has no scheme"))?;
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "https" => Scheme::Https,
            #[cfg(test)]
            "http" if allow_http => Scheme::Http,
            _ => return Err(invalid("only https is allowed")),
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.contains('@') {
            return Err(invalid("user info is not allowed"));
        }
        let (host, port) = split_port(authority).map_err(|why| invalid(&why))?;
        let host = parse_host(host).map_err(|why| invalid(&why))?;
        let port = match port {
            None => scheme.default_port(),
            Some(port) => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| invalid("bad port"))?,
        };
        let tail = tail.split_once('#').map_or(tail, |(before, _)| before);
        let path_and_query = match tail.as_bytes().first() {
            None => "/".to_owned(),
            Some(b'?') => format!("/{tail}"),
            Some(_) => tail.to_owned(),
        };
        Ok(Self {
            origin: Origin { scheme, host, port },
            path_and_query,
        })
    }

    /// Resolves a `Location` header against this URL.
    pub(crate) fn join(&self, reference: &str) -> Result<Self, String> {
        let allow_http = self.allows_http();
        let has_scheme = reference.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
        });
        if has_scheme {
            return Self::parse(reference, allow_http);
        }
        if let Some(rest) = reference.strip_prefix("//") {
            return Self::parse(&format!("{}://{rest}", self.origin.scheme.as_str()), allow_http);
        }
        let path = self.path_and_query.split('?').next().unwrap_or("/");
        let tail = if reference.starts_with('/') {
            reference.to_owned()
        } else if reference.starts_with('?') || reference.is_empty() || reference.starts_with('#') {
            format!("{path}{reference}")
        } else {
            let directory = &path[..=path.rfind('/').unwrap_or(0)];
            format!("{directory}{reference}")
        };
        Self::parse(&format!("{}{tail}", self.origin), allow_http)
    }

    fn allows_http(&self) -> bool {
        #[cfg(test)]
        if self.origin.scheme == Scheme::Http {
            return true;
        }
        false
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.origin, self.path_and_query)
    }
}

fn truncate(text: &str) -> &str {
    let mut end = text.len().min(200);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn split_port(authority: &str) -> Result<(&str, Option<&str>), String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (_, after) = rest.split_once(']').ok_or("unclosed `[`")?;
        let host_len = authority.len() - after.len();
        return match after {
            "" => Ok((&authority[..host_len], None)),
            _ => match after.strip_prefix(':') {
                Some(port) => Ok((&authority[..host_len], Some(port))),
                None => Err("junk after the IPv6 address".to_owned()),
            },
        };
    }
    Ok(match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    })
}

fn parse_host(host: &str) -> Result<Host, String> {
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner
            .parse::<Ipv6Addr>()
            .map(|ip| Host::Ip(IpAddr::V6(ip)))
            .map_err(|_| "bad IPv6 address".to_owned());
    }
    if host.is_empty() || host.len() > 253 {
        return Err("bad host".to_owned());
    }
    let host = host.to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    for label in &labels {
        let valid = !label.is_empty()
            && label.len() <= 63
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !valid {
            return Err(format!("bad host label `{label}`"));
        }
    }
    // Resolvers read `127.1`, `0x7f.0.0.1` and `2130706433` as IPv4 addresses. Only the strict
    // dotted quad may name one, so a numeric-looking host is never sent to DNS as a name.
    let last = labels.last().copied().unwrap_or_default();
    if last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x") {
        return host
            .parse::<Ipv4Addr>()
            .map(|ip| Host::Ip(IpAddr::V4(ip)))
            .map_err(|_| "a numeric host must be a dotted-quad IPv4 address".to_owned());
    }
    Ok(Host::Domain(host))
}
