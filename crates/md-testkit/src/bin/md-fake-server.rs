//! The test servers as one process, for simulated networks where each device
//! is its own process (scripts/lab/nat-sim.sh):
//!
//! ```text
//! md-fake-server <ip> <second-ip> <user> <pass>
//! ```
//!
//! Registry on `<ip>:8080`, relay on `<ip>:8081`, and STUN on port 3478 of
//! both addresses (NAT classification needs two destinations). Prints one
//! line, `ready`, once everything listens, then serves until killed.

use md_testkit::{FakeRegistry, FakeRelay, FakeStun};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [ip, ip2, user, pass] = args.as_slice() else {
        eprintln!("usage: md-fake-server <ip> <second-ip> <user> <pass>");
        std::process::exit(2);
    };
    let relay = FakeRelay::start_on(&format!("{ip}:8081"));
    let registry = FakeRegistry::start_on_with_relay(&format!("{ip}:8080"), &relay);
    registry.add_user(user, pass);
    let _stun = [
        FakeStun::reflect_on(&format!("{ip}:3478")),
        FakeStun::reflect_on(&format!("{ip2}:3478")),
    ];
    println!("ready");
    loop {
        std::thread::park();
    }
}
