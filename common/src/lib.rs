use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

pub const JOIN: u8 = 1;
pub const AUDIO: u8 = 2;
pub const LEAVE: u8 = 3;
pub const PING: u8 = 4;

/// kind(1) + room(8) + user(4)
pub const HEADER: usize = 13;
pub const RATE: u32 = 48_000;
/// 20 мс при 48 кГц
pub const FRAME: usize = 960;

pub fn header(kind: u8, room: u64, user: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(HEADER + 512);
    v.push(kind);
    v.extend_from_slice(&room.to_be_bytes());
    v.extend_from_slice(&user.to_be_bytes());
    v
}

pub fn parse(buf: &[u8]) -> Option<(u8, u64, u32, &[u8])> {
    if buf.len() < HEADER {
        return None;
    }
    let room = u64::from_be_bytes(buf[1..9].try_into().ok()?);
    let user = u32::from_be_bytes(buf[9..13].try_into().ok()?);
    Some((buf[0], room, user, &buf[HEADER..]))
}

pub fn rand_u64() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u128(nanos);
    h.finish()
}

/// voice://host:port/0123456789abcdef
pub fn format_link(server: &str, room: u64) -> String {
    format!("voice://{server}/{room:016x}")
}

pub fn parse_link(s: &str) -> Option<(String, u64)> {
    let rest = s.trim().strip_prefix("voice://")?;
    let (server, room) = rest.rsplit_once('/')?;
    let room = u64::from_str_radix(room.trim_end_matches('/'), 16).ok()?;
    if server.is_empty() {
        return None;
    }
    Some((server.to_string(), room))
}
