//! DMX-over-IP output: Art-Net 4 (ArtDmx) and sACN / E1.31.

pub mod discovery;
pub mod net;
pub use discovery::{discover, explain_send_error, ArtNode};
pub use net::{interfaces, NetIface};

use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

pub const ARTNET_PORT: u16 = 6454;
pub const SACN_PORT: u16 = 5568;

/// Build an ArtDmx packet. `universe` is the 15-bit port-address (0-based).
/// `data` is padded to an even length (min 2) as the spec requires.
pub fn artnet_packet(universe: u16, sequence: u8, data: &[u8]) -> Vec<u8> {
    let mut len = data.len().min(512);
    if len % 2 == 1 {
        len += 1;
    }
    let len = len.max(2);
    let mut p = Vec::with_capacity(18 + len);
    p.extend_from_slice(b"Art-Net\0");
    p.extend_from_slice(&0x5000u16.to_le_bytes()); // OpDmx
    p.extend_from_slice(&14u16.to_be_bytes()); // protocol version
    p.push(sequence); // 0 disables sequencing; callers use 1..=255
    p.push(0); // physical
    p.extend_from_slice(&(universe & 0x7fff).to_le_bytes()); // SubUni, Net
    p.extend_from_slice(&(len as u16).to_be_bytes());
    p.extend_from_slice(&data[..data.len().min(512)]);
    p.resize(18 + len, 0);
    p
}

/// Build an sACN (ANSI E1.31) data packet for a 512-slot universe.
/// `universe` is the 1-based sACN universe number.
pub fn sacn_packet(cid: &[u8; 16], source_name: &str, universe: u16, priority: u8, sequence: u8, data: &[u8]) -> Vec<u8> {
    const SLOTS: usize = 512;
    let mut p = vec![0u8; 126 + SLOTS];
    // Root layer
    p[0..2].copy_from_slice(&0x0010u16.to_be_bytes());
    p[4..16].copy_from_slice(b"ASC-E1.17\0\0\0");
    let total = p.len();
    p[16..18].copy_from_slice(&(0x7000u16 | (total - 16) as u16).to_be_bytes());
    p[18..22].copy_from_slice(&4u32.to_be_bytes()); // VECTOR_ROOT_E131_DATA
    p[22..38].copy_from_slice(cid);
    // Framing layer
    p[38..40].copy_from_slice(&(0x7000u16 | (total - 38) as u16).to_be_bytes());
    p[40..44].copy_from_slice(&2u32.to_be_bytes()); // VECTOR_E131_DATA_PACKET
    let name = source_name.as_bytes();
    let n = name.len().min(63); // keep NUL terminator
    p[44..44 + n].copy_from_slice(&name[..n]);
    p[108] = priority;
    p[111] = sequence;
    p[113..115].copy_from_slice(&universe.to_be_bytes());
    // DMP layer
    p[115..117].copy_from_slice(&(0x7000u16 | (total - 115) as u16).to_be_bytes());
    p[117] = 0x02; // VECTOR_DMP_SET_PROPERTY
    p[118] = 0xa1;
    p[119..121].copy_from_slice(&0u16.to_be_bytes());
    p[121..123].copy_from_slice(&1u16.to_be_bytes());
    p[123..125].copy_from_slice(&((SLOTS + 1) as u16).to_be_bytes());
    p[125] = 0; // DMX start code
    let n = data.len().min(SLOTS);
    p[126..126 + n].copy_from_slice(&data[..n]);
    p
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protocol {
    ArtNet,
    Sacn,
}

/// One UDP socket that can send any universe to any target.
#[derive(Clone, Debug, Default)]
pub struct SendStats {
    pub packets: u64,
    pub errors: u64,
    pub last_error: Option<String>,
    pub last_error_hint: Option<&'static str>,
    /// Times a socket was rebuilt after the network changed.
    pub reconnects: u64,
}

/// State of the network port a lumiverse sends through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteState {
    /// Let the computer choose.
    Auto,
    /// Sending through this port.
    Up(net::NetIface),
    /// The named port isn't connected right now; retrying.
    Waiting(String),
}

struct Route {
    socket: UdpSocket,
    iface: Option<net::NetIface>,
}

/// UDP sender for Art-Net and sACN. Keeps one socket per network port and
/// rebuilds it when the port disappears, comes back or changes address.
pub struct Sender {
    pub stats: SendStats,
    /// Art-Net destination port (6454; tests change it).
    pub artnet_port: u16,
    routes: std::collections::HashMap<Option<String>, Route>,
    ifaces: Vec<net::NetIface>,
    last_scan: std::time::Instant,
    cid: [u8; 16],
    sequences: std::collections::HashMap<(Protocol, Ipv4Addr, u16), u8>,
}

const RESCAN: std::time::Duration = std::time::Duration::from_secs(2);

impl Sender {
    pub fn new() -> io::Result<Self> {
        // sACN needs a CID unique per source; derive one from pid + time.
        let mut cid = [0u8; 16];
        let seed = std::process::id() as u128
            ^ std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        cid.copy_from_slice(&seed.to_le_bytes());
        let mut s = Sender {
            stats: SendStats::default(),
            artnet_port: ARTNET_PORT,
            routes: Default::default(),
            ifaces: net::interfaces_with(true),
            last_scan: std::time::Instant::now(),
            cid,
            sequences: Default::default(),
        };
        // Fail early if we can't even open a socket.
        s.route(None)?;
        Ok(s)
    }

    /// Re-read the network ports; drop sockets whose port vanished or changed address.
    pub fn rescan(&mut self) {
        self.ifaces = net::interfaces_with(true);
        let ifaces = &self.ifaces;
        let before = self.routes.len();
        self.routes.retain(|name, r| match (name, &r.iface) {
            (Some(n), Some(old)) => ifaces.iter().any(|i| &i.name == n && i.ip == old.ip && i.netmask == old.netmask),
            _ => true,
        });
        self.stats.reconnects += (before - self.routes.len()) as u64;
        self.last_scan = std::time::Instant::now();
    }

    fn find_iface(&self, name: &str) -> Option<net::NetIface> {
        self.ifaces.iter().find(|i| i.name == name).cloned()
    }

    fn route(&mut self, via: Option<&str>) -> io::Result<&mut Route> {
        if self.last_scan.elapsed() > RESCAN {
            self.rescan();
        }
        let key = via.map(str::to_string);
        if !self.routes.contains_key(&key) {
            let iface = match via {
                Some(n) => Some(self.find_iface(n).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, format!("network port {n} isn't connected"))
                })?),
                None => None,
            };
            let socket = net::open_socket(iface.as_ref())?;
            self.routes.insert(key.clone(), Route { socket, iface });
        }
        Ok(self.routes.get_mut(&key).unwrap())
    }

    /// What's happening on a lumiverse's port.
    pub fn route_state(&self, via: Option<&str>) -> RouteState {
        match via {
            None => RouteState::Auto,
            Some(n) => match self.routes.get(&Some(n.to_string())).and_then(|r| r.iface.clone()) {
                Some(i) => RouteState::Up(i),
                None => match self.find_iface(n) {
                    Some(i) => RouteState::Up(i),
                    None => RouteState::Waiting(n.to_string()),
                },
            },
        }
    }

    fn next_seq(&mut self, key: (Protocol, Ipv4Addr, u16)) -> u8 {
        let s = self.sequences.entry(key).or_insert(0);
        *s = if *s == 255 { 1 } else { *s + 1 }; // Art-Net: 0 means "off", so skip it
        *s
    }

    /// Send one universe through whatever port the computer picks.
    pub fn send(&mut self, protocol: Protocol, target: Ipv4Addr, universe: u16, data: &[u8]) -> io::Result<()> {
        self.send_via(protocol, target, universe, data, None)
    }

    /// Send one universe through a named network port (`None` = automatic).
    /// `universe` is the 0-based Art-Net port-address; for sACN it is sent as
    /// `universe + 1` to the standard multicast group.
    pub fn send_via(&mut self, protocol: Protocol, target: Ipv4Addr, universe: u16, data: &[u8], via: Option<&str>) -> io::Result<()> {
        let seq = self.next_seq((protocol, target, universe));
        let (port, cid) = (self.artnet_port, self.cid);
        let res = self.route(via).and_then(|r| match protocol {
            Protocol::ArtNet => {
                let pkt = artnet_packet(universe, seq, data);
                let dest = net::resolve_target(target, r.iface.as_ref());
                r.socket.send_to(&pkt, SocketAddr::from((dest, port)))
            }
            Protocol::Sacn => {
                let u = universe + 1;
                let pkt = sacn_packet(&cid, "Lumen", u, 100, seq, data);
                let dest = Ipv4Addr::new(239, 255, (u >> 8) as u8, (u & 0xff) as u8);
                r.socket.send_to(&pkt, SocketAddr::from((dest, SACN_PORT)))
            }
        });
        match res {
            Ok(_) => {
                self.stats.packets += 1;
                Ok(())
            }
            // Dropping a frame is better than stalling the show.
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => {
                if net::is_network_change(&e) {
                    // Rebuild this port's socket on the next send, and re-read ports soon.
                    if self.routes.remove(&via.map(str::to_string)).is_some() {
                        self.stats.reconnects += 1;
                    }
                    self.last_scan = std::time::Instant::now() - RESCAN;
                }
                self.stats.errors += 1;
                self.stats.last_error = Some(e.to_string());
                self.stats.last_error_hint = Some(explain_send_error(&e));
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artnet_header_layout() {
        let data = [1u8, 2, 3];
        let p = artnet_packet(0x0102, 7, &data);
        assert_eq!(&p[0..8], b"Art-Net\0");
        assert_eq!(&p[8..10], &[0x00, 0x50]); // OpDmx little-endian
        assert_eq!(&p[10..12], &[0, 14]);
        assert_eq!(p[12], 7);
        assert_eq!(&p[14..16], &[0x02, 0x01]); // SubUni then Net
        assert_eq!(&p[16..18], &[0, 4]); // odd length padded to 4
        assert_eq!(&p[18..], &[1, 2, 3, 0]);
    }

    #[test]
    fn artnet_full_universe() {
        let p = artnet_packet(0, 1, &[9u8; 512]);
        assert_eq!(p.len(), 18 + 512);
        assert_eq!(&p[16..18], &[2, 0]);
    }

    #[test]
    fn sacn_lengths_and_fields() {
        let cid = [0xAB; 16];
        let p = sacn_packet(&cid, "Lumen", 258, 100, 9, &[5u8; 512]);
        assert_eq!(p.len(), 638);
        assert_eq!(&p[16..18], &[0x72, 0x6e]);
        assert_eq!(&p[38..40], &[0x72, 0x58]);
        assert_eq!(&p[115..117], &[0x72, 0x0b]);
        assert_eq!(&p[4..16], b"ASC-E1.17\0\0\0");
        assert_eq!(&p[44..49], b"Lumen");
        assert_eq!(p[108], 100);
        assert_eq!(p[111], 9);
        assert_eq!(&p[113..115], &[1, 2]);
        assert_eq!(&p[123..125], &[0x02, 0x01]);
        assert_eq!(p[125], 0);
        assert_eq!(p[126], 5);
        assert_eq!(p[637], 5);
    }

    /// Through a named port (loopback here): packets arrive, a missing port waits, and comes back.
    #[test]
    fn sender_routes_and_recovers() {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let mut s = Sender::new().unwrap();
        s.artnet_port = rx.local_addr().unwrap().port();
        let lo = net::interfaces_with(true).into_iter().find(|i| i.ip.is_loopback()).unwrap();
        s.send_via(Protocol::ArtNet, Ipv4Addr::LOCALHOST, 3, &[7, 8, 9], Some(&lo.name)).unwrap();
        let mut buf = [0u8; 600];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[18..21], &[7, 8, 9]);
        assert_eq!(n, 18 + 4);
        assert!(matches!(s.route_state(Some(&lo.name)), RouteState::Up(_)));
        // A port that isn't there: error, counted, and reported as waiting.
        let e = s.send_via(Protocol::ArtNet, Ipv4Addr::LOCALHOST, 3, &[1], Some("nope0")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotConnected);
        assert_eq!(s.route_state(Some("nope0")), RouteState::Waiting("nope0".into()));
        assert_eq!(s.stats.errors, 1);
        // Automatic still works.
        s.send(Protocol::ArtNet, Ipv4Addr::LOCALHOST, 0, &[1, 2]).unwrap();
        rx.recv_from(&mut buf).unwrap();
        assert_eq!(s.route_state(None), RouteState::Auto);
    }

    /// If a port's address changes (new DHCP lease), the old socket is dropped and rebuilt.
    #[test]
    fn rescan_drops_stale_socket() {
        let mut s = Sender::new().unwrap();
        let lo = net::interfaces_with(true).into_iter().find(|i| i.ip.is_loopback()).unwrap();
        let mut stale = lo.clone();
        stale.ip = Ipv4Addr::new(127, 9, 9, 9);
        let socket = net::open_socket(None).unwrap();
        s.routes.insert(Some(lo.name.clone()), Route { socket, iface: Some(stale) });
        s.rescan();
        assert!(!s.routes.contains_key(&Some(lo.name.clone())));
        assert_eq!(s.stats.reconnects, 1);
        // next send rebuilds it on the real address
        s.artnet_port = 9; // discard
        s.send_via(Protocol::ArtNet, Ipv4Addr::LOCALHOST, 0, &[1], Some(&lo.name)).unwrap();
        assert_eq!(s.route_state(Some(&lo.name)), RouteState::Up(lo));
    }

    #[test]
    fn udp_loopback_artnet() {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        // Send via raw socket to the receiver's ephemeral port, exercising the same packet builder.
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let pkt = artnet_packet(2, 1, &[255, 0, 128]);
        tx.send_to(&pkt, rx.local_addr().unwrap()).unwrap();
        let mut buf = [0u8; 600];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], &pkt[..]);
    }
}
