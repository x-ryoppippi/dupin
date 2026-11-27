use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Whether an extension may reach addresses on the user's own networks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrivateNetworks {
    /// Only globally routable addresses. The default.
    #[default]
    Block,
    /// Also RFC 1918, carrier-grade NAT and IPv6 unique local addresses, for a user who granted a
    /// LAN service. Loopback, link-local, multicast and reserved addresses stay blocked.
    Allow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Public,
    Private,
    Loopback,
    LinkLocal,
    Multicast,
    Reserved,
}

impl Class {
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::Public => "a public address",
            Self::Private => "a private network address",
            Self::Loopback => "a loopback address",
            Self::LinkLocal => "a link-local address",
            Self::Multicast => "a multicast address",
            Self::Reserved => "a reserved address",
        }
    }
}

pub(crate) fn classify(ip: IpAddr) -> Class {
    match ip {
        IpAddr::V4(ip) => classify_v4(ip),
        IpAddr::V6(ip) => classify_v6(ip),
    }
}

fn classify_v4(ip: Ipv4Addr) -> Class {
    let [a, b, c, _] = ip.octets();
    match (a, b, c) {
        (127, ..) => Class::Loopback,
        (10, ..) | (172, 16..=31, _) | (192, 168, _) | (100, 64..=127, _) => Class::Private,
        (169, 254, _) => Class::LinkLocal,
        (224..=239, ..) => Class::Multicast,
        (0, ..)
        | (240..=255, ..)
        | (192, 0, 0 | 2)
        | (192, 88, 99)
        | (198, 18..=19, _)
        | (198, 51, 100)
        | (203, 0, 113) => Class::Reserved,
        _ => Class::Public,
    }
}

fn classify_v6(ip: Ipv6Addr) -> Class {
    let s = ip.segments();
    if ip.is_loopback() {
        return Class::Loopback;
    }
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    // NAT64 and 6to4 carry an IPv4 destination, which decides where the packet ends up.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return classify_v4(embedded_v4(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return classify_v4(embedded_v4(s[1], s[2]));
    }
    match s[0] {
        0xfc00..=0xfdff => Class::Private,
        0xfe80..=0xfebf => Class::LinkLocal,
        0xff00..=0xffff => Class::Multicast,
        // 2001::/23 holds Teredo, benchmarking and ORCHID; 2001:db8::/32 is documentation.
        0x2001 if s[1] < 0x200 || s[1] == 0xdb8 => Class::Reserved,
        0x2000..=0x3fff => Class::Public,
        _ => Class::Reserved,
    }
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    Ipv4Addr::from((u32::from(high) << 16) | u32::from(low))
}

pub(crate) fn allowed(class: Class, private: PrivateNetworks, loopback: bool) -> bool {
    match class {
        Class::Public => true,
        Class::Private => private == PrivateNetworks::Allow,
        Class::Loopback => loopback,
        Class::LinkLocal | Class::Multicast | Class::Reserved => false,
    }
}
