//! Network ports, and sockets tied to them.
//!
//! A lumiverse can name the port it should use ("en5"). We look the port up
//! by name every couple of seconds, so it survives the IP changing (DHCP),
//! the cable being unplugged and plugged back, or a USB adapter re-enumerating.

use socket2::{Domain, Protocol as SockProto, Socket, Type};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetIface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub broadcast: Ipv4Addr,
    pub index: Option<u32>,
}

impl NetIface {
    pub fn prefix(&self) -> u32 {
        u32::from(self.netmask).count_ones()
    }

    pub fn label(&self) -> String {
        format!("{} · {}/{}", self.name, self.ip, self.prefix())
    }

    /// Is `ip` on this port's subnet?
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let m = u32::from(self.netmask);
        u32::from(self.ip) & m == u32::from(ip) & m
    }
}

fn subnet_broadcast(ip: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(ip) | !u32::from(mask))
}

/// IPv4 network ports, loopback optional.
pub fn interfaces_with(include_loopback: bool) -> Vec<NetIface> {
    let mut v: Vec<NetIface> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if include_loopback || !a.ip.is_loopback() => Some(NetIface {
                broadcast: a.broadcast.unwrap_or_else(|| subnet_broadcast(a.ip, a.netmask)),
                name: i.name,
                ip: a.ip,
                netmask: a.netmask,
                index: i.index,
            }),
            _ => None,
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name).then(a.ip.cmp(&b.ip)));
    v
}

/// The ports a user can pick (no loopback).
pub fn interfaces() -> Vec<NetIface> {
    interfaces_with(false)
}

/// Where to actually send. Limited broadcast (255.255.255.255) on a chosen
/// port becomes that port's subnet broadcast: macOS otherwise sends it out of
/// one interface only, whichever it likes.
pub fn resolve_target(target: Ipv4Addr, via: Option<&NetIface>) -> Ipv4Addr {
    match via {
        Some(i) if target == Ipv4Addr::BROADCAST => i.broadcast,
        _ => target,
    }
}

/// A UDP socket that sends from `via` (or from any port when None).
pub fn open_socket(via: Option<&NetIface>) -> io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(SockProto::UDP))?;
    s.set_broadcast(true)?;
    s.set_multicast_ttl_v4(8)?;
    let bind_ip = via.map(|i| i.ip).unwrap_or(Ipv4Addr::UNSPECIFIED);
    s.bind(&SocketAddr::V4(SocketAddrV4::new(bind_ip, 0)).into())?;
    if let Some(i) = via {
        s.set_multicast_if_v4(&i.ip)?;
        // On macOS, also pin the socket to the interface itself, so broadcasts and
        // packets to addresses outside every subnet still leave through this port.
        #[cfg(target_vendor = "apple")]
        if let Some(idx) = i.index.and_then(std::num::NonZeroU32::new) {
            s.bind_device_by_index_v4(Some(idx))?;
        }
    }
    s.set_nonblocking(true)?;
    Ok(s.into())
}

/// Errors that mean "the network under this socket changed": rebuild it.
pub fn is_network_change(e: &io::Error) -> bool {
    use io::ErrorKind::*;
    matches!(e.kind(), AddrNotAvailable | NetworkUnreachable | HostUnreachable | NetworkDown | NotConnected)
        || matches!(e.raw_os_error(), Some(49 | 50 | 51 | 65 | 99 | 100 | 101 | 113))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(ip: [u8; 4], mask: [u8; 4]) -> NetIface {
        let (ip, netmask) = (Ipv4Addr::from(ip), Ipv4Addr::from(mask));
        NetIface { name: "en5".into(), ip, netmask, broadcast: subnet_broadcast(ip, netmask), index: None }
    }

    #[test]
    fn subnet_logic() {
        let i = iface([2, 0, 0, 100], [255, 0, 0, 0]);
        assert_eq!(i.broadcast, Ipv4Addr::new(2, 255, 255, 255));
        assert!(i.contains(Ipv4Addr::new(2, 0, 0, 50)));
        assert!(!i.contains(Ipv4Addr::new(192, 168, 1, 50)));
        assert_eq!(i.label(), "en5 · 2.0.0.100/8");
        assert_eq!(resolve_target(Ipv4Addr::BROADCAST, Some(&i)), Ipv4Addr::new(2, 255, 255, 255));
        assert_eq!(resolve_target(Ipv4Addr::new(2, 0, 0, 50), Some(&i)), Ipv4Addr::new(2, 0, 0, 50));
        assert_eq!(resolve_target(Ipv4Addr::BROADCAST, None), Ipv4Addr::BROADCAST);
    }

    #[test]
    fn socket_on_loopback_port_sends() {
        let lo = interfaces_with(true).into_iter().find(|i| i.ip.is_loopback()).expect("loopback");
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let tx = open_socket(Some(&lo)).unwrap();
        tx.send_to(b"hi", rx.local_addr().unwrap()).unwrap();
        let mut buf = [0u8; 8];
        let (n, from) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hi");
        assert!(from.ip().is_loopback());
    }
}
