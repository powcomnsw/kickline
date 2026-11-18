//! Art-Net node discovery (ArtPoll / ArtPollReply).

use socket2::{Domain, Protocol as SockProto, Socket, Type};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ARTNET_PORT;

pub fn artpoll_packet() -> Vec<u8> {
    let mut p = Vec::with_capacity(14);
    p.extend_from_slice(b"Art-Net\0");
    p.extend_from_slice(&0x2000u16.to_le_bytes()); // OpPoll
    p.extend_from_slice(&14u16.to_be_bytes());
    p.push(0); // flags
    p.push(0); // diag priority
    p
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtNode {
    pub ip: Ipv4Addr,
    pub short_name: String,
    pub long_name: String,
    /// Output universes (15-bit port-addresses) the node reports.
    pub universes: Vec<u16>,
    pub mac: Option<[u8; 6]>,
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

pub fn parse_artpoll_reply(d: &[u8]) -> Option<ArtNode> {
    if d.len() < 197 || &d[0..8] != b"Art-Net\0" || d[8..10] != [0x00, 0x21] {
        return None;
    }
    let ip = Ipv4Addr::new(d[10], d[11], d[12], d[13]);
    let net = d[18] as u16 & 0x7f;
    let sub = d[19] as u16 & 0x0f;
    let ports = (u16::from_be_bytes([d[172], d[173]]) as usize).min(4);
    let mut universes = Vec::new();
    for i in 0..ports {
        // Port type bit 7 = can output DMX
        if d[174 + i] & 0x80 != 0 {
            universes.push((net << 8) | (sub << 4) | (d[190 + i] as u16 & 0x0f));
        }
    }
    let mac = (d.len() >= 207).then(|| [d[201], d[202], d[203], d[204], d[205], d[206]]);
    Some(ArtNode { ip, short_name: cstr(&d[26..44]), long_name: cstr(&d[44..108]), universes, mac })
}

/// Bind the Art-Net port in a way that coexists with other Art-Net software.
fn bind_artnet_port() -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(SockProto::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    s.set_reuse_port(true)?;
    s.set_broadcast(true)?;
    s.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, ARTNET_PORT)).into())?;
    Ok(s.into())
}

/// Broadcast ArtPoll to each address and collect replies for `wait`.
pub fn discover(broadcasts: &[Ipv4Addr], wait: Duration) -> io::Result<Vec<ArtNode>> {
    let sock = bind_artnet_port()?;
    sock.set_read_timeout(Some(Duration::from_millis(100)))?;
    let poll = artpoll_packet();
    let mut send_errors = Vec::new();
    for b in broadcasts {
        if let Err(e) = sock.send_to(&poll, SocketAddr::from((*b, ARTNET_PORT))) {
            send_errors.push(format!("{b}: {e}"));
        }
    }
    if send_errors.len() == broadcasts.len() && !broadcasts.is_empty() {
        return Err(io::Error::other(send_errors.join("; ")));
    }
    let mut nodes: Vec<ArtNode> = Vec::new();
    let end = Instant::now() + wait;
    let mut buf = [0u8; 1024];
    while Instant::now() < end {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                if let Some(node) = parse_artpoll_reply(&buf[..n]) {
                    merge_node(&mut nodes, node);
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(nodes)
}

/// Nodes with more than 4 outputs answer once per group of 4 ("bind index"),
/// all from the same IP. Fold those into one node with every universe.
pub fn merge_node(nodes: &mut Vec<ArtNode>, node: ArtNode) -> usize {
    if let Some(i) = nodes.iter().position(|x| x.ip == node.ip) {
        let x = &mut nodes[i];
        for u in node.universes {
            if !x.universes.contains(&u) {
                x.universes.push(u);
            }
        }
        x.universes.sort_unstable();
        if x.long_name.is_empty() {
            x.long_name = node.long_name;
        }
        i
    } else {
        nodes.push(node);
        nodes.len() - 1
    }
}

// ------------------------------------------------------------ live watching

/// How often a controller polls (Art-Net asks for 2.5–3 s).
pub const POLL_EVERY: Duration = Duration::from_secs(3);
/// A node that hasn't answered for this long counts as gone.
pub const ONLINE_FOR: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct SeenNode {
    pub node: ArtNode,
    /// Network port the node is reachable through, if one's subnet contains it.
    pub port: Option<String>,
    pub last_seen: Instant,
}

impl SeenNode {
    pub fn online(&self) -> bool {
        self.last_seen.elapsed() < ONLINE_FOR
    }
    pub fn name(&self) -> &str {
        if self.node.long_name.is_empty() {
            &self.node.short_name
        } else {
            &self.node.long_name
        }
    }
}

#[derive(Default)]
struct Watch {
    nodes: Vec<SeenNode>,
    error: Option<String>,
    polls: u64,
}

/// Polls for Art-Net nodes in the background every few seconds while active,
/// and remembers every node it has heard from. Cheap to clone.
#[derive(Clone)]
pub struct NodeWatcher {
    inner: Arc<Mutex<Watch>>,
    active: Arc<AtomicBool>,
    poll_now: Arc<AtomicBool>,
}

impl Default for NodeWatcher {
    fn default() -> Self {
        Self::start()
    }
}

impl NodeWatcher {
    pub fn start() -> Self {
        let w = NodeWatcher {
            inner: Arc::default(),
            active: Arc::new(AtomicBool::new(false)),
            poll_now: Arc::new(AtomicBool::new(false)),
        };
        let t = w.clone();
        let _ = std::thread::Builder::new().name("artpoll".into()).spawn(move || t.run());
        w
    }

    /// Poll only while something needs it (output on, or the patch is open).
    pub fn set_active(&self, on: bool) {
        if on && !self.active.swap(true, Ordering::Relaxed) {
            self.poll_now.store(true, Ordering::Relaxed);
        } else if !on {
            self.active.store(false, Ordering::Relaxed);
        }
    }

    pub fn poll_now(&self) {
        self.poll_now.store(true, Ordering::Relaxed);
    }

    pub fn nodes(&self) -> Vec<SeenNode> {
        self.inner.lock().map(|w| w.nodes.clone()).unwrap_or_default()
    }

    pub fn error(&self) -> Option<String> {
        self.inner.lock().ok().and_then(|w| w.error.clone())
    }

    /// Number of polls sent so far (lets callers wait for a fresh round).
    pub fn polls(&self) -> u64 {
        self.inner.lock().map(|w| w.polls).unwrap_or(0)
    }

    /// Feed a reply in directly (used by tests and by the probe).
    pub fn record(&self, node: ArtNode, ports: &[crate::NetIface]) {
        let port = ports.iter().find(|p| p.contains(node.ip)).map(|p| p.name.clone());
        if let Ok(mut w) = self.inner.lock() {
            let mut plain: Vec<ArtNode> = w.nodes.iter().map(|n| n.node.clone()).collect();
            let i = merge_node(&mut plain, node);
            if i < w.nodes.len() {
                w.nodes[i].node = plain[i].clone();
                w.nodes[i].last_seen = Instant::now();
                if port.is_some() {
                    w.nodes[i].port = port;
                }
            } else {
                w.nodes.push(SeenNode { node: plain[i].clone(), port, last_seen: Instant::now() });
            }
            w.nodes.sort_by_key(|n| n.node.ip);
        }
    }

    fn set_error(&self, e: Option<String>) {
        if let Ok(mut w) = self.inner.lock() {
            w.error = e;
        }
    }

    fn run(self) {
        let mut sock: Option<UdpSocket> = None;
        let mut last_poll: Option<Instant> = None;
        // After any poll, keep reading replies for a while even if inactive.
        let mut listen_until = Instant::now();
        let mut last_bind_try: Option<Instant> = None;
        let mut ports = crate::net::interfaces();
        let mut buf = [0u8; 1024];
        loop {
            let active = self.active.load(Ordering::Relaxed);
            if !active && !self.poll_now.load(Ordering::Relaxed) && Instant::now() >= listen_until {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            if sock.is_none() && last_bind_try.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
                last_bind_try = Some(Instant::now());
                match bind_artnet_port().and_then(|s| s.set_read_timeout(Some(Duration::from_millis(100))).map(|_| s)) {
                    Ok(s) => {
                        sock = Some(s);
                        self.set_error(None);
                    }
                    Err(e) => self.set_error(Some(format!("{e}. {}", explain_send_error(&e)))),
                }
            }
            let Some(s) = sock.as_ref() else {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            };
            let due = last_poll.is_none_or(|t| t.elapsed() >= POLL_EVERY);
            if self.poll_now.swap(false, Ordering::Relaxed) || (active && due) {
                ports = crate::net::interfaces();
                let poll = artpoll_packet();
                let mut targets = vec![Ipv4Addr::BROADCAST];
                targets.extend(ports.iter().map(|p| p.broadcast));
                targets.dedup();
                let mut errors = Vec::new();
                for t in &targets {
                    if let Err(e) = s.send_to(&poll, SocketAddr::from((*t, ARTNET_PORT))) {
                        errors.push(e);
                    }
                }
                self.set_error(if errors.len() == targets.len() {
                    errors.first().map(|e| format!("{e}. {}", explain_send_error(e)))
                } else {
                    None
                });
                if let Ok(mut w) = self.inner.lock() {
                    w.polls += 1;
                    // Forget nodes not heard from for 10 minutes.
                    w.nodes.retain(|n| n.last_seen.elapsed() < Duration::from_secs(600));
                }
                last_poll = Some(Instant::now());
                listen_until = Instant::now() + POLL_EVERY;
            }
            match s.recv_from(&mut buf) {
                Ok((n, _)) => {
                    if let Some(node) = parse_artpoll_reply(&buf[..n]) {
                        self.record(node, &ports);
                    }
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => {
                    self.set_error(Some(format!("{e}. {}", explain_send_error(&e))));
                    sock = None;
                }
            }
        }
    }
}

/// Plain-language hint for a socket error, with the macOS gotchas first.
pub fn explain_send_error(e: &io::Error) -> &'static str {
    let code = e.raw_os_error().unwrap_or(0);
    // EHOSTUNREACH: 65 on macOS, 113 on Linux. On macOS 15+ this is what you get
    // when the app hasn't been allowed on the Local Network.
    if code == 65 || code == 113 || e.kind() == io::ErrorKind::HostUnreachable {
        return "Host unreachable. On macOS: System Settings > Privacy & Security > Local Network, allow Kickline. Otherwise check the cable / Wi-Fi and the target IP.";
    }
    if code == 51 || code == 101 || e.kind() == io::ErrorKind::NetworkUnreachable {
        return "Network unreachable: no interface on that subnet. Give your computer an IP in the same range as the node (e.g. 2.x.x.x or 192.168.x.x).";
    }
    if code == 13 || e.kind() == io::ErrorKind::PermissionDenied {
        return "Permission denied: broadcast blocked, or a firewall is stopping UDP.";
    }
    if code == 48 || code == 98 || e.kind() == io::ErrorKind::AddrInUse {
        return "Port in use: another app (Resolume, QLC+, a visualiser) has port 6454 open exclusively.";
    }
    "Unexpected network error."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_reply() -> Vec<u8> {
        let mut d = vec![0u8; 239];
        d[0..8].copy_from_slice(b"Art-Net\0");
        d[8..10].copy_from_slice(&[0x00, 0x21]);
        d[10..14].copy_from_slice(&[2, 0, 0, 50]);
        d[18] = 0; // net
        d[19] = 1; // sub
        d[26..33].copy_from_slice(b"PixLite");
        d[44..60].copy_from_slice(b"PixLite 16 MkII\0");
        d[172..174].copy_from_slice(&2u16.to_be_bytes());
        d[174] = 0x80;
        d[175] = 0x80;
        d[190] = 0;
        d[191] = 3;
        d[201..207].copy_from_slice(&[0, 1, 2, 3, 4, 5]);
        d
    }

    #[test]
    fn poll_layout() {
        let p = artpoll_packet();
        assert_eq!(&p[8..12], &[0x00, 0x20, 0, 14]);
        assert_eq!(p.len(), 14);
    }

    #[test]
    fn parse_reply() {
        let n = parse_artpoll_reply(&fake_reply()).unwrap();
        assert_eq!(n.ip, Ipv4Addr::new(2, 0, 0, 50));
        assert_eq!(n.short_name, "PixLite");
        assert_eq!(n.long_name, "PixLite 16 MkII");
        assert_eq!(n.universes, vec![16, 19]);
        assert_eq!(n.mac, Some([0, 1, 2, 3, 4, 5]));
        assert!(parse_artpoll_reply(&fake_reply()[..100]).is_none());
    }

    #[test]
    fn multi_bind_replies_merge() {
        let mut a = parse_artpoll_reply(&fake_reply()).unwrap();
        let mut nodes = Vec::new();
        merge_node(&mut nodes, a.clone());
        a.universes = vec![20, 21, 22, 23];
        merge_node(&mut nodes, a);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].universes, vec![16, 19, 20, 21, 22, 23]);
    }

    #[test]
    fn watcher_tracks_nodes_and_port() {
        let w = NodeWatcher { inner: Arc::default(), active: Arc::new(AtomicBool::new(false)), poll_now: Arc::new(AtomicBool::new(false)) };
        let port = crate::NetIface { name: "en5".into(), ip: Ipv4Addr::new(2, 0, 0, 100), netmask: Ipv4Addr::new(255, 0, 0, 0), broadcast: Ipv4Addr::new(2, 255, 255, 255), index: None };
        let n = parse_artpoll_reply(&fake_reply()).unwrap();
        w.record(n.clone(), &[port.clone()]);
        w.record(n, &[port]);
        let seen = w.nodes();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].port.as_deref(), Some("en5"));
        assert!(seen[0].online());
        assert_eq!(seen[0].name(), "PixLite 16 MkII");
    }

    #[test]
    fn explain_mac_local_network() {
        let e = io::Error::from_raw_os_error(65);
        assert!(explain_send_error(&e).contains("Local Network"));
    }

    #[test]
    fn discover_runs_without_nodes() {
        // No nodes in CI, but binding and polling loopback must work.
        let r = discover(&[Ipv4Addr::LOCALHOST], Duration::from_millis(200));
        assert!(r.is_ok(), "{r:?}");
    }
}
