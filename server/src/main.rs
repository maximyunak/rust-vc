use common::*;
use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const MAX_PER_ROOM: usize = 32;
const TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> std::io::Result<()> {
    let bind = std::env::args().nth(1).unwrap_or_else(|| "0.0.0.0:4000".into());
    let sock = UdpSocket::bind(&bind)?;
    sock.set_read_timeout(Some(Duration::from_secs(1)))?;
    println!("Сервер слушает UDP {bind}");

    let mut rooms: HashMap<u64, HashMap<u32, (SocketAddr, Instant)>> = HashMap::new();
    let mut buf = [0u8; 2048];
    let mut last_clean = Instant::now();

    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, addr)) => {
                if let Some((kind, room, user, _)) = parse(&buf[..n]) {
                    if kind == LEAVE {
                        if let Some(r) = rooms.get_mut(&room) {
                            r.remove(&user);
                            for (_, (a, _)) in r.iter() {
                                let _ = sock.send_to(&buf[..n], a);
                            }
                            if r.is_empty() {
                                rooms.remove(&room);
                            }
                        }
                    } else if matches!(kind, JOIN | AUDIO | PING) {
                        let r = rooms.entry(room).or_default();
                        if !r.contains_key(&user) && r.len() >= MAX_PER_ROOM {
                            continue;
                        }
                        r.insert(user, (addr, Instant::now()));
                        for (&u, (a, _)) in r.iter() {
                            if u != user {
                                let _ = sock.send_to(&buf[..n], a);
                            }
                        }
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => eprintln!("recv: {e}"),
        }

        if last_clean.elapsed() > Duration::from_secs(3) {
            last_clean = Instant::now();
            for r in rooms.values_mut() {
                r.retain(|_, (_, t)| t.elapsed() < TIMEOUT);
            }
            rooms.retain(|_, r| !r.is_empty());
        }
    }
}
