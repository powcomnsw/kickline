//! Art-Net node discovery (ArtPoll / ArtPollReply).

use socket2::{Domain, Protocol as SockProto, Socket, Type};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
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
                    if !nodes.iter().any(|x| x.ip == node.ip && x.short_name == node.short_name) {
                        nodes.push(node);
                    }
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(nodes)
}

/// Plain-language hint for a socket error, with the macOS gotchas first.
pub fn explain_send_error(e: &io::Error) -> &'static str {
    let code = e.raw_os_error().unwrap_or(0);
    // EHOSTUNREACH: 65 on macOS, 113 on Linux. On macOS 15+ this is what you get
    // when the app hasn't been allowed on the Local Network.
    if code == 65 || code == 113 || e.kind() == io::ErrorKind::HostUnreachable {
        return "Host unreachable. On macOS: System Settings > Privacy & Security > Local Network, allow Lumen. Otherwise check the cable / Wi-Fi and the target IP.";
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
