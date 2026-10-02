//! Network interfaces: counters from `/proc/net/dev`, the interface carrying
//! traffic from `/proc/net/route`, addresses from one netlink dump, and the
//! rest from `/sys/class/net`.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::time::Instant;

use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, netlink::SocketAddrNetlink,
};

use super::{elapsed, lines, rate};
use crate::sysfs::{self, HeldFile};

const CLASS_NET: &str = "/sys/class/net";

/// One interface, read once. See [`NetSampler`] for throughput and
/// [`addresses`] for its addresses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetInterface {
    /// Kernel name: `wlp7s0`, `enp6s0`, `lo`.
    pub name: String,
    /// What kind of interface it is: "Wi-Fi", "Ethernet", "Bridge"... The
    /// kernel name when there is nothing better.
    pub display: String,
    pub mac: Option<String>,
    /// Link speed. `None` on Wi-Fi, virtual interfaces and a link that is
    /// down.
    pub speed_mbit: Option<u32>,
    pub wireless: bool,
    pub loopback: bool,
    /// The kernel's interface index, which [`addresses`] is keyed by.
    pub index: u32,
}

impl NetInterface {
    /// The name to show: the kind of interface, or the kernel name.
    pub fn label(&self) -> &str {
        &self.display
    }
}

/// Lists the interfaces in `/proc/net/dev`: loopback last, otherwise by name.
pub fn interfaces() -> Vec<NetInterface> {
    let dev = fs::read("/proc/net/dev").unwrap_or_default();
    let mut list: Vec<_> = parse_net_dev(&dev)
        .filter_map(|(name, ..)| std::str::from_utf8(name).ok())
        .map(|name| interface(Path::new(CLASS_NET), name))
        .collect();
    list.sort_by(|a, b| {
        a.loopback
            .cmp(&b.loopback)
            .then_with(|| a.name.cmp(&b.name))
    });
    list
}

/// Reads one interface's attributes from a `/sys/class/net` directory.
pub fn interface(class_net: &Path, name: &str) -> NetInterface {
    let dir = class_net.join(name);
    let wireless = ["wireless", "phy80211"]
        .iter()
        .any(|m| dir.join(m).exists());
    let loopback = name == "lo";
    NetInterface {
        name: name.to_owned(),
        display: kind(&dir, name, wireless, loopback)
            .unwrap_or(name)
            .to_owned(),
        mac: sysfs::read_string(dir.join("address")).filter(|m| !m.is_empty()),
        // `speed` can't be read on a link that is down, and is -1 when the
        // driver doesn't know.
        speed_mbit: sysfs::read_string(dir.join("speed"))
            .and_then(|s| s.parse::<i64>().ok())
            .and_then(|s| u32::try_from(s).ok())
            .filter(|&s| s > 0),
        wireless,
        loopback,
        index: sysfs::read_uint(dir.join("ifindex"))
            .and_then(|i| u32::try_from(i).ok())
            .unwrap_or(0),
    }
}

/// What kind of interface `dir` is, for its label.
fn kind(dir: &Path, name: &str, wireless: bool, loopback: bool) -> Option<&'static str> {
    let prefixed = |p: &[&str]| p.iter().any(|p| name.starts_with(p));
    Some(if loopback {
        "Loopback"
    } else if wireless {
        "Wi-Fi"
    } else if prefixed(&["docker"]) {
        "Docker"
    } else if dir.join("bridge").exists() || prefixed(&["br-", "virbr"]) {
        "Bridge"
    } else if dir.join("device").exists() {
        // Only hardware has a device link.
        "Ethernet"
    } else if dir.join("brport").exists() || prefixed(&["veth"]) {
        // A virtual machine's or container's port on a bridge.
        "Virtual Ethernet"
    } else if dir.join("tun_flags").exists() || prefixed(&["tun", "tap", "wg"]) {
        "Tunnel / VPN"
    } else {
        return None;
    })
}

/// Walks `/proc/net/dev`: each interface's name and its received and sent
/// byte counters.
pub fn parse_net_dev(data: &[u8]) -> impl Iterator<Item = (&[u8], u64, u64)> {
    lines(data).filter_map(|line| {
        let colon = line.iter().position(|&c| c == b':')?;
        let counters = &line[colon + 1..];
        // Received bytes is the first column, sent bytes the ninth.
        let rx = sysfs::parse_uint(sysfs::field(counters, 0)?)?;
        let tx = sysfs::parse_uint(sysfs::field(counters, 8)?)?;
        Some((line[..colon].trim_ascii(), rx, tx))
    })
}

/// The interface with the lowest-metric IPv4 default route in
/// `/proc/net/route`: destination and mask both 0.0.0.0.
pub fn parse_default_route(data: &[u8]) -> Option<&[u8]> {
    lines(data)
        .skip(1) // the header
        .filter_map(|line| {
            let col = |i| sysfs::field(line, i);
            if col(1)? != b"00000000" || col(7)? != b"00000000" {
                return None;
            }
            Some((
                col(6).and_then(sysfs::parse_uint).unwrap_or(u64::MAX),
                col(0)?,
            ))
        })
        .min_by_key(|&(metric, _)| metric)
        .map(|(_, name)| name)
}

/// One interface's throughput.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetIo {
    pub name: String,
    /// Bytes received and sent since the interface came up.
    pub rx_total: u64,
    pub tx_total: u64,
    /// Bytes per second since the previous sample.
    pub rx_rate: f64,
    pub tx_rate: f64,
    seen: bool,
}

/// Samples `/proc/net/dev` and `/proc/net/route` from held files.
///
/// Interfaces come and go (a VPN, a phone tethered over USB), so a sample
/// lists whatever is there now; one seen for the first time has no rate yet.
#[derive(Debug)]
pub struct NetSampler {
    dev: Option<HeldFile>,
    route: Option<HeldFile>,
    io: Vec<NetIo>,
    last: Instant,
}

impl NetSampler {
    /// Opens the files and takes the first counters.
    pub fn new() -> Self {
        Self::open(Path::new("/proc/net/dev"), Path::new("/proc/net/route"))
    }

    fn open(dev: &Path, route: &Path) -> Self {
        let mut sampler = Self {
            dev: HeldFile::with_capacity(dev, 4096),
            route: HeldFile::with_capacity(route, 4096),
            io: Vec::new(),
            last: Instant::now(),
        };
        sampler.read(None);
        sampler
    }

    /// Reads every interface's counters and its rates since the last sample.
    pub fn sample(&mut self) -> &[NetIo] {
        let seconds = elapsed(&mut self.last);
        self.read(Some(seconds));
        &self.io
    }

    /// The interface carrying the default route, which the sidebar marks.
    pub fn default_route(&mut self) -> Option<&str> {
        let name = parse_default_route(self.route.as_mut()?.bytes()?)?;
        // Hand back this sampler's copy of the name: no allocation per tick.
        self.io
            .iter()
            .find(|io| io.name.as_bytes() == name)
            .map(|io| io.name.as_str())
    }

    fn read(&mut self, seconds: Option<f64>) {
        let Some(data) = self.dev.as_mut().and_then(HeldFile::bytes) else {
            return;
        };
        for io in &mut self.io {
            io.seen = false;
        }
        for (name, rx, tx) in parse_net_dev(data) {
            match self.io.iter_mut().find(|io| io.name.as_bytes() == name) {
                Some(io) => {
                    if let Some(s) = seconds {
                        io.rx_rate = rate(rx, io.rx_total, s);
                        io.tx_rate = rate(tx, io.tx_total, s);
                    }
                    io.rx_total = rx;
                    io.tx_total = tx;
                    io.seen = true;
                }
                None => self.io.push(NetIo {
                    name: String::from_utf8_lossy(name).into_owned(),
                    rx_total: rx,
                    tx_total: tx,
                    seen: true,
                    ..NetIo::default()
                }),
            }
        }
        self.io.retain(|io| io.seen);
    }
}

impl Default for NetSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// An interface's first IPv4 address and first IPv6 address that isn't
/// link-local.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Addresses {
    pub ipv4: Option<Ipv4Addr>,
    pub ipv6: Option<Ipv6Addr>,
}

// Netlink message layout, for reading the address table by hand.
const NLMSG_HDR: usize = 16; // struct nlmsghdr
const IFADDRMSG: usize = 8; // struct ifaddrmsg
const RTATTR: usize = 4; // struct rtattr
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const RTM_NEWADDR: u16 = 20;
const RTM_GETADDR: u16 = 22;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_DUMP: u16 = 0x300;
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;

/// Every interface's addresses, by interface index, from one `RTM_GETADDR`
/// dump. Asking per interface would cost a dump each.
///
/// Addresses change rarely; the sampling loop refreshes them every few
/// ticks, not every one.
pub fn addresses() -> io::Result<HashMap<u32, Addresses>> {
    let fd = rustix::net::socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        None, // NETLINK_ROUTE is protocol 0
    )?;
    let mut request = [0u8; NLMSG_HDR + IFADDRMSG];
    request[0..4].copy_from_slice(&((NLMSG_HDR + IFADDRMSG) as u32).to_ne_bytes());
    request[4..6].copy_from_slice(&RTM_GETADDR.to_ne_bytes());
    request[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
    request[8..12].copy_from_slice(&1u32.to_ne_bytes()); // sequence number
    // ifaddrmsg stays zero: every family, every interface.
    rustix::net::sendto(
        &fd,
        &request,
        SendFlags::empty(),
        &SocketAddrNetlink::new(0, 0),
    )?;

    let mut out = HashMap::new();
    let mut buf = vec![0u8; 32 * 1024];
    loop {
        let n = match rustix::net::recv(&fd, &mut buf[..], RecvFlags::empty()) {
            Ok((n, _)) => n,
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        };
        if n == 0 {
            return Ok(out);
        }
        match parse_addr_dump(&buf[..n], &mut out) {
            Dump::More => {}
            Dump::Done => return Ok(out),
            Dump::Error => return Err(io::Error::other("netlink address dump failed")),
        }
    }
}

/// Where a netlink dump stands after one datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dump {
    More,
    Done,
    Error,
}

/// Reads the `RTM_NEWADDR` messages in one datagram of an address dump into
/// `out`, keeping the first address of each family per interface. A point-
/// to-point link puts its own address in `IFA_LOCAL` and the peer's in
/// `IFA_ADDRESS`; everywhere else they are the same.
pub fn parse_addr_dump(mut buf: &[u8], out: &mut HashMap<u32, Addresses>) -> Dump {
    let u16_at = |b: &[u8], i: usize| u16::from_ne_bytes([b[i], b[i + 1]]);
    while buf.len() >= NLMSG_HDR {
        let len = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        if len < NLMSG_HDR || len > buf.len() {
            return Dump::Error;
        }
        let kind = u16_at(buf, 4);
        let body = &buf[NLMSG_HDR..len];
        buf = buf.get(len.next_multiple_of(4)..).unwrap_or_default();
        match kind {
            NLMSG_DONE => return Dump::Done,
            NLMSG_ERROR => return Dump::Error,
            RTM_NEWADDR if body.len() >= IFADDRMSG => {}
            _ => continue,
        }
        let family = body[0];
        let index = u32::from_ne_bytes([body[4], body[5], body[6], body[7]]);
        let mut addr: Option<&[u8]> = None;
        let mut attrs = &body[IFADDRMSG..];
        while attrs.len() >= RTATTR {
            let len = usize::from(u16_at(attrs, 0));
            if len < RTATTR || len > attrs.len() {
                break;
            }
            match u16_at(attrs, 2) {
                IFA_LOCAL => addr = Some(&attrs[RTATTR..len]),
                IFA_ADDRESS if addr.is_none() => addr = Some(&attrs[RTATTR..len]),
                _ => {}
            }
            attrs = attrs.get(len.next_multiple_of(4)..).unwrap_or_default();
        }
        let entry = out.entry(index).or_default();
        match (family, addr) {
            (AF_INET, Some(a)) if entry.ipv4.is_none() => {
                if let Ok(a) = <[u8; 4]>::try_from(a) {
                    entry.ipv4 = Some(Ipv4Addr::from(a));
                }
            }
            (AF_INET6, Some(a)) if entry.ipv6.is_none() => {
                if let Ok(a) = <[u8; 16]>::try_from(a) {
                    let ip = Ipv6Addr::from(a);
                    entry.ipv6 = (!ip.is_unicast_link_local()).then_some(ip);
                }
            }
            _ => {}
        }
    }
    Dump::More
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET_DEV: &[u8] = include_bytes!("../../tests/fixtures/net_dev");
    const NET_ROUTE: &[u8] = include_bytes!("../../tests/fixtures/net_route");

    #[test]
    fn parses_net_dev() {
        let got: Vec<_> = parse_net_dev(NET_DEV).collect();
        assert_eq!(
            got,
            [
                (&b"lo"[..], 39_315_496, 39_315_496),
                (b"enp6s0", 0, 0),
                (b"wlp7s0", 45_985_783_214, 3_055_300_893),
                (b"virbr0", 21_327_888, 6_664_432_987),
            ]
        );
        assert_eq!(
            parse_net_dev(b"eth0: 1 2 3\n").count(),
            0,
            "too few columns"
        );
    }

    #[test]
    fn finds_the_default_route() {
        assert_eq!(parse_default_route(NET_ROUTE), Some(&b"wlp7s0"[..]));
        // Two default routes: the lower metric wins; a more specific route is
        // not a default route.
        let two =
            b"Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
            wlp7s0\t00000000\t0102A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
            enp6s0\t00000000\t0102A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
            enp6s0\t0002A8C0\t00000000\t0001\t0\t0\t50\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(parse_default_route(two), Some(&b"enp6s0"[..]));
        // Destination 0 with a mask is not a default route.
        let masked = b"Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
            tun0\t00000000\t00000000\t0001\t0\t0\t0\t000000FF\n";
        assert_eq!(parse_default_route(masked), None);
        assert_eq!(parse_default_route(b"Iface\tDestination\tGateway\n"), None);
        assert_eq!(parse_default_route(b""), None);
    }

    #[test]
    fn sampler_follows_interfaces_coming_and_going() {
        let dir = tempfile::tempdir().unwrap();
        let (dev, route) = (dir.path().join("dev"), dir.path().join("route"));
        fs::write(&dev, NET_DEV).unwrap();
        fs::write(&route, NET_ROUTE).unwrap();
        let mut s = NetSampler::open(&dev, &route);
        assert_eq!(s.io.len(), 4);
        assert_eq!(s.default_route(), Some("wlp7s0"));

        // virbr0 is gone, tun0 is new, wlp7s0 received 1 MiB.
        fs::write(
            &dev,
            "h1\nh2\n    lo: 39315496 0 0 0 0 0 0 0 39315496 0 0 0 0 0 0 0\n\
             wlp7s0: 45986831790 0 0 0 0 0 0 0 3055300893 0 0 0 0 0 0 0\n\
             tun0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n",
        )
        .unwrap();
        let io = s.sample().to_vec();
        let names: Vec<_> = io.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["lo", "wlp7s0", "tun0"]);
        assert!(io[1].rx_rate > 0.0);
        assert_eq!(io[1].tx_rate, 0.0);
        assert_eq!(
            (io[2].rx_rate, io[2].tx_rate),
            (0.0, 0.0),
            "no rate on first sight"
        );
        assert_eq!(io[2].tx_total, 20);
    }

    #[test]
    fn names_interfaces_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str, marker: Option<&str>| {
            let d = dir.path().join(name);
            fs::create_dir_all(&d).unwrap();
            if let Some(m) = marker {
                fs::create_dir_all(d.join(m)).unwrap();
            }
            fs::write(d.join("ifindex"), "3\n").unwrap();
        };
        make("wlp7s0", Some("phy80211"));
        make("enp6s0", Some("device"));
        make("podman0", Some("bridge"));
        make("tun0", None);
        make("vnet0", Some("brport"));
        fs::write(dir.path().join("vnet0/tun_flags"), "0x1002\n").unwrap();
        make("odd0", None);
        fs::write(dir.path().join("enp6s0/speed"), "1000\n").unwrap();
        fs::write(dir.path().join("enp6s0/address"), "aa:bb:cc:dd:ee:ff\n").unwrap();
        fs::write(dir.path().join("wlp7s0/speed"), "-1\n").unwrap();

        let get = |name| interface(dir.path(), name);
        let wifi = get("wlp7s0");
        assert!(wifi.wireless);
        assert_eq!((wifi.label(), wifi.speed_mbit), ("Wi-Fi", None));
        let eth = get("enp6s0");
        assert_eq!(
            (eth.label(), eth.speed_mbit, eth.index),
            ("Ethernet", Some(1000), 3)
        );
        assert_eq!(eth.mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(get("podman0").label(), "Bridge");
        assert_eq!(get("tun0").label(), "Tunnel / VPN");
        assert_eq!(get("vnet0").label(), "Virtual Ethernet");
        assert_eq!(get("odd0").label(), "odd0");
        assert_eq!(get("missing").index, 0);
        let lo = get("lo");
        assert!(lo.loopback);
        assert_eq!(lo.label(), "Loopback");
    }

    /// Builds one netlink message.
    fn message(kind: u16, body: &[u8]) -> Vec<u8> {
        let len = NLMSG_HDR + body.len();
        let mut m = Vec::new();
        m.extend((len as u32).to_ne_bytes());
        m.extend(kind.to_ne_bytes());
        m.extend([0u8; 10]);
        m.extend(body);
        m.resize(len.next_multiple_of(4), 0);
        m
    }

    fn new_addr(family: u8, index: u32, attrs: &[(u16, &[u8])]) -> Vec<u8> {
        let mut body = vec![family, 0, 0, 0];
        body.extend(index.to_ne_bytes());
        for (kind, data) in attrs {
            let len = RTATTR + data.len();
            body.extend((len as u16).to_ne_bytes());
            body.extend(kind.to_ne_bytes());
            body.extend(*data);
            body.resize(body.len().next_multiple_of(4), 0);
        }
        message(RTM_NEWADDR, &body)
    }

    #[test]
    fn parses_an_address_dump() {
        let link_local = Ipv6Addr::new(0xfe80, 0, 0, 0, 1, 2, 3, 4).octets();
        let global = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1).octets();
        let mut dump = Vec::new();
        dump.extend(new_addr(AF_INET, 2, &[(IFA_ADDRESS, &[192, 168, 1, 20])]));
        dump.extend(new_addr(AF_INET, 2, &[(IFA_ADDRESS, &[10, 0, 0, 1])])); // second: ignored
        // Point-to-point: own address in LOCAL, peer in ADDRESS.
        dump.extend(new_addr(
            AF_INET,
            5,
            &[(IFA_ADDRESS, &[10, 8, 0, 1]), (IFA_LOCAL, &[10, 8, 0, 2])],
        ));
        dump.extend(new_addr(AF_INET6, 2, &[(IFA_ADDRESS, &link_local)]));
        dump.extend(new_addr(AF_INET6, 2, &[(IFA_ADDRESS, &global)]));
        let mut out = HashMap::new();
        assert_eq!(parse_addr_dump(&dump, &mut out), Dump::More);
        assert_eq!(
            parse_addr_dump(&message(NLMSG_DONE, &[0; 4]), &mut out),
            Dump::Done
        );
        assert_eq!(out[&2].ipv4, Some(Ipv4Addr::new(192, 168, 1, 20)));
        assert_eq!(out[&2].ipv6, Some(Ipv6Addr::from(global)));
        assert_eq!(out[&5].ipv4, Some(Ipv4Addr::new(10, 8, 0, 2)));
    }

    #[test]
    fn rejects_broken_dumps() {
        let mut out = HashMap::new();
        assert_eq!(parse_addr_dump(&[], &mut out), Dump::More);
        assert_eq!(parse_addr_dump(&[1, 2, 3], &mut out), Dump::More);
        assert_eq!(parse_addr_dump(&[0; 16], &mut out), Dump::Error);
        let mut huge = vec![0xff; 4];
        huge.extend([0; 12]);
        assert_eq!(parse_addr_dump(&huge, &mut out), Dump::Error);
        assert_eq!(
            parse_addr_dump(&message(NLMSG_ERROR, &[0; 20]), &mut out),
            Dump::Error
        );
        // An attribute claiming more than the message holds stops the walk.
        let mut bad = new_addr(AF_INET, 9, &[(IFA_ADDRESS, &[1, 2, 3, 4])]);
        bad[NLMSG_HDR + IFADDRMSG] = 0xff;
        assert_eq!(parse_addr_dump(&bad, &mut out), Dump::More);
        assert_eq!(out.get(&9).and_then(|a| a.ipv4), None);
        // A wrong-sized address is no address.
        let short = new_addr(AF_INET, 7, &[(IFA_ADDRESS, &[1, 2, 3])]);
        parse_addr_dump(&short, &mut out);
        assert_eq!(out.get(&7).and_then(|a| a.ipv4), None);
    }

    // Live tests: invariants only. CI's container has its own network
    // namespace, with at least a loopback.

    #[test]
    fn live_loopback_has_its_address() {
        let list = interfaces();
        let Some(lo) = list.iter().find(|i| i.loopback) else {
            return;
        };
        assert_eq!(list.last(), Some(lo), "loopback goes last");
        // Rootless containers may show the host's /sys/class/net (no index
        // for lo here), netlink may be blocked, and lo may be down (no
        // address at all): only an address that is there is checked.
        let Ok(addrs) = addresses() else {
            return;
        };
        if let Some(v4) = addrs
            .get(&lo.index)
            .and_then(|a| a.ipv4)
            .filter(|_| lo.index != 0)
        {
            assert!(v4.is_loopback(), "{v4}");
        }
    }

    #[test]
    fn live_counters_only_go_up() {
        let mut s = NetSampler::new();
        let before: Vec<_> = s.io.clone();
        for io in s.sample() {
            if let Some(b) = before.iter().find(|b| b.name == io.name) {
                assert!(
                    io.rx_total >= b.rx_total && io.tx_total >= b.tx_total,
                    "{}",
                    io.name
                );
            }
            assert!(io.rx_rate >= 0.0 && io.tx_rate >= 0.0);
        }
        if let Some(name) = s.default_route().map(str::to_owned) {
            assert!(interfaces().iter().any(|i| i.name == name));
        }
    }
}
