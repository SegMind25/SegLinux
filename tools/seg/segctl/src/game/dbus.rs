//! Keeps the screen from blinking out during a game: a minimal D-Bus client,
//! just enough to call `org.freedesktop.ScreenSaver.Inhibit` on the session
//! bus and hold the connection open (the inhibition ends when it closes).
//! Plasma's PowerDevil and screen locker, GNOME and others implement that
//! interface. No D-Bus library: a method call is a few dozen bytes.

use std::io::{Read, Write};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixStream};
use std::time::Duration;

use crate::error::{Error, Result};

/// Where the session bus listens, from DBUS_SESSION_BUS_ADDRESS
/// (`unix:path=/run/user/1000/bus` or `unix:abstract=/tmp/dbus-AbC`).
#[derive(Debug, PartialEq, Eq)]
pub enum BusAddress {
    Path(String),
    Abstract(String),
}

pub fn parse_address(text: &str) -> Option<BusAddress> {
    text.split(';').find_map(|entry| {
        let rest = entry.strip_prefix("unix:")?;
        rest.split(',').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            let v = unescape(v);
            match k {
                "path" => Some(BusAddress::Path(v)),
                "abstract" => Some(BusAddress::Abstract(v)),
                _ => None,
            }
        })
    })
}

/// `%2f` style escapes of D-Bus addresses.
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(v) = u8::from_str_radix(hex, 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pad(buf: &mut Vec<u8>, align: usize) {
    buf.resize(buf.len().next_multiple_of(align), 0);
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    pad(buf, 4);
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_str(buf: &mut Vec<u8>, s: &str) {
    put_u32(buf, s.len() as u32);
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

/// One header field: `(code, variant)` with a string-like value.
fn put_field(buf: &mut Vec<u8>, code: u8, kind: u8, value: &str) {
    pad(buf, 8);
    buf.push(code);
    buf.extend_from_slice(&[1, kind, 0]);
    put_str(buf, value);
}

/// A METHOD_CALL message. `signature` and `body` go together (`"ss"` and
/// two marshalled strings), or are both empty.
pub fn method_call(
    serial: u32,
    destination: &str,
    path: &str,
    interface: &str,
    member: &str,
    signature: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut fields = Vec::new();
    put_field(&mut fields, 1, b'o', path);
    put_field(&mut fields, 6, b's', destination);
    put_field(&mut fields, 2, b's', interface);
    put_field(&mut fields, 3, b's', member);
    if !signature.is_empty() {
        pad(&mut fields, 8);
        fields.push(8);
        fields.extend_from_slice(&[1, b'g', 0]);
        fields.push(signature.len() as u8);
        fields.extend_from_slice(signature.as_bytes());
        fields.push(0);
    }
    let mut msg = vec![b'l', 1, 0, 1];
    msg.extend_from_slice(&(body.len() as u32).to_le_bytes());
    msg.extend_from_slice(&serial.to_le_bytes());
    msg.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    msg.extend_from_slice(&fields);
    pad(&mut msg, 8);
    msg.extend_from_slice(body);
    msg
}

pub fn hello(serial: u32) -> Vec<u8> {
    method_call(
        serial,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "Hello",
        "",
        &[],
    )
}

pub fn inhibit(serial: u32, application: &str, reason: &str) -> Vec<u8> {
    let mut body = Vec::new();
    put_str(&mut body, application);
    put_str(&mut body, reason);
    method_call(
        serial,
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver",
        "Inhibit",
        "ss",
        &body,
    )
}

#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    /// 2 method return, 3 error, 4 signal.
    pub kind: u8,
    pub reply_serial: Option<u32>,
    pub error_name: Option<String>,
    pub body: Vec<u8>,
}

fn get_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn align(at: usize, to: usize) -> usize {
    at.div_ceil(to) * to
}

/// A string-like value at `at` (aligned): its text and where it ends.
fn read_str(data: &[u8], at: usize) -> Option<(String, usize)> {
    let at = align(at, 4);
    let len = get_u32(data, at)? as usize;
    let text = data.get(at + 4..at + 4 + len)?;
    Some((String::from_utf8_lossy(text).into_owned(), at + 4 + len + 1))
}

/// Parse the part of a message after its first 16 bytes' fixed header:
/// `fields` is the header-field array, `body` the rest.
pub fn parse_reply(kind: u8, fields: &[u8], body: Vec<u8>) -> Option<Reply> {
    let mut reply = Reply {
        kind,
        reply_serial: None,
        error_name: None,
        body,
    };
    // The array starts at offset 16 of the message, a multiple of 8, so
    // offsets inside `fields` have the same alignment as in the message.
    let mut at = 0;
    while at < fields.len() {
        at = align(at, 8);
        if at >= fields.len() {
            break;
        }
        let code = *fields.get(at)?;
        let sig_len = *fields.get(at + 1)? as usize;
        let sig = fields.get(at + 2..at + 2 + sig_len)?;
        at += 2 + sig_len + 1;
        match sig {
            b"u" => {
                let v = get_u32(fields, align(at, 4))?;
                at = align(at, 4) + 4;
                if code == 5 {
                    reply.reply_serial = Some(v);
                }
            }
            b"s" | b"o" => {
                let (s, end) = read_str(fields, at)?;
                at = end;
                if code == 4 {
                    reply.error_name = Some(s);
                }
            }
            b"g" => {
                let len = *fields.get(at)? as usize;
                at += 1 + len + 1;
            }
            _ => return None,
        }
    }
    Some(reply)
}

fn read_message(stream: &mut UnixStream) -> std::io::Result<Option<Reply>> {
    let mut fixed = [0u8; 16];
    stream.read_exact(&mut fixed)?;
    if fixed[0] != b'l' {
        return Ok(None);
    }
    let body_len = u32::from_le_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]) as usize;
    let fields_len = u32::from_le_bytes([fixed[12], fixed[13], fixed[14], fixed[15]]) as usize;
    if body_len > 1 << 20 || fields_len > 1 << 16 {
        return Ok(None);
    }
    let mut fields = vec![0u8; fields_len];
    stream.read_exact(&mut fields)?;
    let padding = align(fields_len, 8) - fields_len;
    let mut skip = vec![0u8; padding];
    stream.read_exact(&mut skip)?;
    let mut body = vec![0u8; body_len];
    stream.read_exact(&mut body)?;
    Ok(parse_reply(fixed[1], &fields, body))
}

/// An open inhibition: dropping it closes the connection, which ends it.
pub struct Inhibitor {
    _stream: UnixStream,
}

fn connect(address: &BusAddress) -> std::io::Result<UnixStream> {
    match address {
        BusAddress::Path(p) => UnixStream::connect(p),
        BusAddress::Abstract(name) => {
            UnixStream::connect_addr(&SocketAddr::from_abstract_name(name.as_bytes())?)
        }
    }
}

fn bus_error(e: impl std::fmt::Display) -> Error {
    Error::other(format!("session bus: {e}"))
}

/// Authenticate and say Hello, and return the stream.
fn open(address: &BusAddress, uid: u32) -> Result<UnixStream> {
    let mut stream = connect(address).map_err(bus_error)?;
    let timeout = Some(Duration::from_secs(2));
    stream.set_read_timeout(timeout).map_err(bus_error)?;
    stream.set_write_timeout(timeout).map_err(bus_error)?;
    let hex: String = uid
        .to_string()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect();
    stream
        .write_all(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes())
        .map_err(bus_error)?;
    // The answer is one line: "OK <guid>".
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while !line.ends_with(b"\r\n") {
        stream.read_exact(&mut byte).map_err(bus_error)?;
        line.push(byte[0]);
        if line.len() > 256 {
            return Err(bus_error("the bus answered nonsense"));
        }
    }
    if !line.starts_with(b"OK") {
        return Err(bus_error("authentication was refused"));
    }
    stream.write_all(b"BEGIN\r\n").map_err(bus_error)?;
    stream.write_all(&hello(1)).map_err(bus_error)?;
    wait_for(&mut stream, 1)?;
    Ok(stream)
}

/// Read messages until the reply to `serial` arrives (signals such as
/// NameAcquired come first).
fn wait_for(stream: &mut UnixStream, serial: u32) -> Result<Reply> {
    for _ in 0..16 {
        let message = read_message(stream).map_err(bus_error)?;
        if let Some(r) = message
            && r.reply_serial == Some(serial)
        {
            return match r.kind {
                2 => Ok(r),
                _ => Err(bus_error(r.error_name.unwrap_or_else(|| "an error".into()))),
            };
        }
    }
    Err(bus_error("no answer"))
}

/// Ask the screen saver not to start, for `reason`.
pub fn inhibit_screensaver(application: &str, reason: &str) -> Result<Inhibitor> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        .ok()
        .and_then(|a| parse_address(&a))
        .or_else(|| {
            let uid = crate::sys::euid().ok()?;
            let path = format!("/run/user/{uid}/bus");
            std::path::Path::new(&path)
                .exists()
                .then_some(BusAddress::Path(path))
        })
        .ok_or_else(|| Error::other("no session bus (DBUS_SESSION_BUS_ADDRESS is not set)"))?;
    let mut stream = open(&address, crate::sys::euid()?)?;
    stream
        .write_all(&inhibit(2, application, reason))
        .map_err(bus_error)?;
    let reply = wait_for(&mut stream, 2)?;
    // The answer carries a cookie; an answer without one is not an inhibition.
    get_u32(&reply.body, 0).ok_or_else(|| bus_error("the answer has no cookie"))?;
    Ok(Inhibitor { _stream: stream })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            parse_address("unix:path=/run/user/1000/bus"),
            Some(BusAddress::Path("/run/user/1000/bus".into()))
        );
        assert_eq!(
            parse_address("unix:abstract=/tmp/dbus-AbC123,guid=0123"),
            Some(BusAddress::Abstract("/tmp/dbus-AbC123".into()))
        );
        assert_eq!(
            parse_address("tcp:host=x,port=1;unix:path=/a%20b/bus"),
            Some(BusAddress::Path("/a b/bus".into()))
        );
        assert_eq!(parse_address("tcp:host=x"), None);
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn hello_is_the_message_the_spec_shows() {
        // The 128 bytes dbus-daemon expects for Hello: little endian, a
        // method call, no body, serial 1, 110 bytes of header fields.
        let m = hello(1);
        assert_eq!(m.len(), 128);
        assert_eq!(
            &m[..16],
            &[b'l', 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 110, 0, 0, 0]
        );
        // PATH, then DESTINATION, INTERFACE and MEMBER, each on an 8-byte boundary.
        assert_eq!(&m[16..20], &[1, 1, b'o', 0]);
        assert_eq!(&m[20..24], &[21, 0, 0, 0]);
        assert_eq!(&m[24..45], b"/org/freedesktop/DBus");
        assert_eq!(&m[48..52], &[6, 1, b's', 0]);
        assert_eq!(&m[112..116], &[3, 1, b's', 0]);
        assert_eq!(&m[116..120], &[5, 0, 0, 0]);
        assert_eq!(&m[120..126], b"Hello\0");
        assert_eq!(&m[126..], &[0, 0]);
    }

    #[test]
    fn inhibit_carries_two_strings_and_their_signature() {
        let m = inhibit(2, "seg", "playing a game");
        // body: 4+3+1 (padded to 4) = 8, then 4+14+1 = 19
        let body_len = u32::from_le_bytes([m[4], m[5], m[6], m[7]]) as usize;
        assert_eq!(body_len, 8 + 19);
        let body = &m[m.len() - body_len..];
        assert_eq!(&body[..4], &[3, 0, 0, 0]);
        assert_eq!(&body[4..8], b"seg\0");
        assert_eq!(&body[8..12], &[14, 0, 0, 0]);
        assert_eq!(&body[12..27], b"playing a game\0");
        // The header ends 8-aligned, so the body starts aligned.
        assert_eq!((m.len() - body_len) % 8, 0);
        let text = String::from_utf8_lossy(&m);
        assert!(text.contains("org.freedesktop.ScreenSaver"));
        assert!(text.contains("Inhibit"));
        assert!(m.windows(3).any(|w| w == [b's', b's', 0]), "signature ss");
    }

    /// Fields of a METHOD_RETURN as dbus-daemon sends them: REPLY_SERIAL=2,
    /// DESTINATION=":1.5", SENDER="org.freedesktop.ScreenSaver", SIGNATURE=u.
    fn return_fields() -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&[5, 1, b'u', 0]);
        f.extend_from_slice(&2u32.to_le_bytes());
        put_field(&mut f, 6, b's', ":1.5");
        put_field(&mut f, 7, b's', "org.freedesktop.ScreenSaver");
        pad(&mut f, 8);
        f.extend_from_slice(&[8, 1, b'g', 0, 1, b'u', 0]);
        f
    }

    #[test]
    fn a_method_return_gives_its_serial_and_cookie() {
        let r = parse_reply(2, &return_fields(), 4242u32.to_le_bytes().to_vec()).unwrap();
        assert_eq!(r.kind, 2);
        assert_eq!(r.reply_serial, Some(2));
        assert_eq!(r.error_name, None);
        assert_eq!(get_u32(&r.body, 0), Some(4242));
    }

    #[test]
    fn an_error_gives_its_name() {
        let mut f = Vec::new();
        put_field(&mut f, 4, b's', "org.freedesktop.DBus.Error.ServiceUnknown");
        pad(&mut f, 8);
        f.extend_from_slice(&[5, 1, b'u', 0]);
        f.extend_from_slice(&2u32.to_le_bytes());
        let r = parse_reply(3, &f, vec![]).unwrap();
        assert_eq!(r.kind, 3);
        assert_eq!(r.reply_serial, Some(2));
        assert_eq!(
            r.error_name.as_deref(),
            Some("org.freedesktop.DBus.Error.ServiceUnknown")
        );
    }

    #[test]
    fn damaged_headers_are_not_a_panic() {
        assert!(parse_reply(2, &[5, 1], vec![]).is_none());
        assert!(parse_reply(2, &[5, 1, b'q', 0, 0, 0, 0, 0], vec![]).is_none());
        assert_eq!(
            parse_reply(2, &[], vec![]).map(|r| r.reply_serial),
            Some(None)
        );
        // A string that claims to be longer than the data.
        assert!(parse_reply(3, &[4, 1, b's', 0, 99, 0, 0, 0, b'x', 0], vec![]).is_none());
    }

    #[test]
    fn without_a_bus_it_is_an_error_not_a_hang() {
        let r = open(&BusAddress::Path("/definitely/not/a/socket".into()), 1000);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().starts_with("session bus: "));
    }
}
