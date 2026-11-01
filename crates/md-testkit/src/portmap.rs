//! A fake home router for UPnP port mapping: "forwards" by reporting
//! 127.0.0.1 with the same port (which on loopback really does reach the
//! engine), and records every map/unmap so tests can check the lifecycle.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Mutex;

use mdcore::nat::portmap::PortMapper;

#[derive(Debug, Default)]
pub struct FakeRouter {
    log: Mutex<Vec<String>>,
}

impl FakeRouter {
    /// "map 127.0.0.1:P" / "unmap P" entries, in order.
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl PortMapper for FakeRouter {
    fn map_udp(&self, local: SocketAddrV4, _lease_secs: u32) -> Option<SocketAddrV4> {
        let public = SocketAddrV4::new(Ipv4Addr::LOCALHOST, local.port());
        self.log.lock().unwrap().push(format!("map {public}"));
        Some(public)
    }

    fn unmap_udp(&self, external_port: u16) {
        self.log
            .lock()
            .unwrap()
            .push(format!("unmap {external_port}"));
    }
}
