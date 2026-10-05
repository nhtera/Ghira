// SPDX-License-Identifier: Apache-2.0
//! The pairing QR (doc 07 §3.2; slice 15-E): `"GHI1:" || base45(CBOR{v, dev,
//! pk, psk, addrs})`. It carries the desktop's key and a one-time PSK, never a
//! name. The parser rejects unknown `v`, trailing bytes and addresses that are
//! not LAN. Nothing logs a payload.
//!
//! The image is rendered here (D11): [`svg`] returns an SVG string the UI shows
//! as an `<img>` data URL; no JS QR library.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use ciborium::Value;

use crate::identity::Psk;
use crate::{Result, SyncError};

/// Text prefix of the payload.
pub const QR_PREFIX: &str = "GHI1:";
/// Payload version.
pub const QR_VERSION: u8 = 1;
/// Most addresses in a payload.
pub const MAX_QR_ADDRS: usize = 4;
/// Longest text [`parse`] looks at (a full payload is about 250 characters).
pub const MAX_QR_TEXT: usize = 1024;

/// What the phone learns from the QR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrPayload {
    pub v: u8,
    /// The hub's `device_gid` (16 bytes).
    pub dev: [u8; 16],
    /// The hub's static public key.
    pub pk: [u8; 32],
    /// The one-time QR PSK (redacted in `Debug`).
    pub psk: Psk,
    /// At most [`MAX_QR_ADDRS`], all LAN.
    pub addrs: Vec<SocketAddr>,
}

fn bad(what: &str) -> SyncError {
    SyncError::Wire(format!("pairing code: {what}"))
}

/// The text to put in the QR. Refuses what [`parse`] would refuse.
pub fn encode(payload: &QrPayload) -> Result<String> {
    if payload.v != QR_VERSION {
        return Err(bad("unknown version"));
    }
    if payload.addrs.len() > MAX_QR_ADDRS {
        return Err(bad("too many addresses"));
    }
    let mut addrs = Vec::with_capacity(payload.addrs.len());
    for a in &payload.addrs {
        if !ghi_net::is_lan(a.ip()) || a.port() == 0 {
            return Err(bad("address is not on the local network"));
        }
        let ip = match a.ip() {
            IpAddr::V4(v4) => v4.octets().to_vec(),
            IpAddr::V6(v6) => v6.octets().to_vec(),
        };
        addrs.push(Value::Array(vec![
            Value::Bytes(ip),
            Value::Integer(a.port().into()),
        ]));
    }
    let map = Value::Map(vec![
        (Value::Text("v".into()), Value::Integer(payload.v.into())),
        (
            Value::Text("dev".into()),
            Value::Bytes(payload.dev.to_vec()),
        ),
        (Value::Text("pk".into()), Value::Bytes(payload.pk.to_vec())),
        (
            Value::Text("psk".into()),
            Value::Bytes(payload.psk.as_bytes().to_vec()),
        ),
        (Value::Text("addrs".into()), Value::Array(addrs)),
    ]);
    let mut cbor = Vec::new();
    ciborium::into_writer(&map, &mut cbor).map_err(|_| bad("cannot encode"))?;
    Ok(format!("{QR_PREFIX}{}", base45::encode(&cbor)))
}

fn bytes_n<const N: usize>(v: Value) -> Result<[u8; N]> {
    match v {
        Value::Bytes(b) => <[u8; N]>::try_from(b.as_slice()).map_err(|_| bad("wrong field size")),
        _ => Err(bad("wrong field type")),
    }
}

fn addr_of(v: Value) -> Result<SocketAddr> {
    let Value::Array(pair) = v else {
        return Err(bad("wrong address type"));
    };
    let [Value::Bytes(ip), Value::Integer(port)] =
        <[Value; 2]>::try_from(pair).map_err(|_| bad("wrong address type"))?
    else {
        return Err(bad("wrong address type"));
    };
    let ip = match ip.len() {
        4 => IpAddr::V4(Ipv4Addr::from(
            <[u8; 4]>::try_from(ip.as_slice()).map_err(|_| bad("address"))?,
        )),
        16 => IpAddr::V6(Ipv6Addr::from(
            <[u8; 16]>::try_from(ip.as_slice()).map_err(|_| bad("address"))?,
        )),
        _ => return Err(bad("wrong address size")),
    };
    let port = u16::try_from(port).map_err(|_| bad("port out of range"))?;
    if port == 0 || !ghi_net::is_lan(ip) {
        return Err(bad("address is not on the local network"));
    }
    Ok(SocketAddr::new(ip, port))
}

/// Parses scanned text. Never trusts it: see the module docs.
pub fn parse(text: &str) -> Result<QrPayload> {
    if text.len() > MAX_QR_TEXT {
        return Err(bad("too long"));
    }
    let body = text
        .strip_prefix(QR_PREFIX)
        .ok_or_else(|| bad("not a Ghira code"))?;
    let cbor = base45::decode(body).map_err(|_| bad("not valid base45"))?;
    let mut rest: &[u8] = &cbor;
    let value: Value = ciborium::from_reader(&mut rest).map_err(|_| bad("not valid CBOR"))?;
    if !rest.is_empty() {
        return Err(bad("trailing bytes"));
    }
    let Value::Map(entries) = value else {
        return Err(bad("not a map"));
    };
    let (mut v, mut dev, mut pk, mut psk, mut addrs) = (None, None, None, None, None);
    for (key, val) in entries {
        let Value::Text(key) = key else {
            return Err(bad("bad key"));
        };
        let dup = match key.as_str() {
            "v" => v.replace(val).is_some(),
            "dev" => dev.replace(val).is_some(),
            "pk" => pk.replace(val).is_some(),
            "psk" => psk.replace(val).is_some(),
            "addrs" => addrs.replace(val).is_some(),
            _ => return Err(bad("unknown field")),
        };
        if dup {
            return Err(bad("repeated field"));
        }
    }
    let (Some(v), Some(dev), Some(pk), Some(psk), Some(addrs)) = (v, dev, pk, psk, addrs) else {
        return Err(bad("missing field"));
    };
    let Value::Integer(v) = v else {
        return Err(bad("wrong field type"));
    };
    if u8::try_from(v) != Ok(QR_VERSION) {
        return Err(bad("unknown version"));
    }
    let Value::Array(addrs) = addrs else {
        return Err(bad("wrong field type"));
    };
    if addrs.len() > MAX_QR_ADDRS {
        return Err(bad("too many addresses"));
    }
    Ok(QrPayload {
        v: QR_VERSION,
        dev: bytes_n(dev)?,
        pk: bytes_n(pk)?,
        psk: Psk::from_bytes(bytes_n(psk)?),
        addrs: addrs.into_iter().map(addr_of).collect::<Result<_>>()?,
    })
}

/// Renders `text` as an SVG QR code (SVG only; no `image` dependency).
pub fn svg(text: &str) -> Result<String> {
    use qrcode::render::svg;
    use qrcode::{EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|_| bad("too long to draw"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(256, 256)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QrPayload {
        QrPayload {
            v: 1,
            dev: [1; 16],
            pk: [2; 32],
            psk: Psk::from_bytes([3; 32]),
            addrs: vec![
                "192.168.1.20:48000".parse().unwrap(),
                "10.0.0.5:48000".parse().unwrap(),
                "[fd12:3456::7]:48000".parse().unwrap(),
            ],
        }
    }

    /// Text for an arbitrary CBOR value, as the phone would scan it.
    fn text_of(v: &Value) -> String {
        let mut cbor = Vec::new();
        ciborium::into_writer(v, &mut cbor).unwrap();
        format!("{QR_PREFIX}{}", base45::encode(&cbor))
    }

    fn raw(v: u8, addrs: Vec<Value>) -> Value {
        Value::Map(vec![
            (Value::Text("v".into()), Value::Integer(v.into())),
            (Value::Text("dev".into()), Value::Bytes(vec![1; 16])),
            (Value::Text("pk".into()), Value::Bytes(vec![2; 32])),
            (Value::Text("psk".into()), Value::Bytes(vec![3; 32])),
            (Value::Text("addrs".into()), Value::Array(addrs)),
        ])
    }

    fn addr(ip: &[u8], port: u16) -> Value {
        Value::Array(vec![Value::Bytes(ip.to_vec()), Value::Integer(port.into())])
    }

    #[test]
    fn round_trips_and_stays_in_one_small_qr() {
        let p = sample();
        let text = encode(&p).unwrap();
        assert!(text.starts_with("GHI1:"));
        assert!(text.len() < 300, "{}", text.len());
        assert_eq!(parse(&text).unwrap(), p);
        let svg = svg(&text).unwrap();
        assert!(svg.starts_with("<?xml") || svg.contains("<svg"));
        assert!(!svg.contains("<image"));
    }

    #[test]
    fn rejects_bad_text() {
        let good = encode(&sample()).unwrap();
        assert!(parse("").is_err());
        assert!(parse("GHI2:abc").is_err());
        assert!(parse(&good.replacen("GHI1:", "ghi1:", 1)).is_err());
        assert!(parse(&format!("{good}{good}")).is_err());
        assert!(parse(&format!("{QR_PREFIX}{}", "A".repeat(2000))).is_err());
        assert!(parse(&format!("{QR_PREFIX}!!!")).is_err());
        assert!(parse(&good[..good.len() - 7]).is_err());
    }

    #[test]
    fn rejects_unknown_version_and_trailing_bytes() {
        assert!(parse(&text_of(&raw(1, vec![]))).is_ok());
        for v in [0, 2, 255] {
            assert!(parse(&text_of(&raw(v, vec![]))).is_err(), "v{v}");
        }
        let mut cbor = Vec::new();
        ciborium::into_writer(&raw(1, vec![]), &mut cbor).unwrap();
        cbor.push(0);
        let text = format!("{QR_PREFIX}{}", base45::encode(&cbor));
        assert!(parse(&text).is_err());
    }

    #[test]
    fn rejects_non_lan_and_malformed_addresses() {
        for bad_ip in [
            &[127, 0, 0, 1][..],
            &[169, 254, 1, 1],
            &[100, 64, 0, 1],
            &[8, 8, 8, 8],
            &[0, 0, 0, 0],
            &[192, 168, 1],
        ] {
            let t = text_of(&raw(1, vec![addr(&[192, 168, 1, 2], 9), addr(bad_ip, 9)]));
            assert!(parse(&t).is_err(), "{bad_ip:?}");
        }
        let mut link_local = [0u8; 16];
        link_local[0] = 0xfe;
        link_local[1] = 0x80;
        assert!(parse(&text_of(&raw(1, vec![addr(&link_local, 9)]))).is_err());
        assert!(parse(&text_of(&raw(1, vec![addr(&[10, 0, 0, 1], 0)]))).is_err());
        assert!(parse(&text_of(&raw(1, vec![addr(&[10, 0, 0, 1], 70)]))).is_ok());
        let five: Vec<Value> = (1..=5).map(|i| addr(&[10, 0, 0, i], 9)).collect();
        assert!(parse(&text_of(&raw(1, five))).is_err());
        // The encoder refuses them too.
        let mut p = sample();
        p.addrs.push("8.8.8.8:1".parse().unwrap());
        assert!(encode(&p).is_err());
    }

    #[test]
    fn rejects_wrong_shapes() {
        let mut m = raw(1, vec![]);
        if let Value::Map(e) = &mut m {
            e[2].1 = Value::Bytes(vec![2; 31]);
        }
        assert!(parse(&text_of(&m)).is_err());
        let mut m = raw(1, vec![]);
        if let Value::Map(e) = &mut m {
            e.push((Value::Text("extra".into()), Value::Null));
        }
        assert!(parse(&text_of(&m)).is_err());
        let mut m = raw(1, vec![]);
        if let Value::Map(e) = &mut m {
            e.pop();
        }
        assert!(parse(&text_of(&m)).is_err());
        assert!(parse(&text_of(&Value::Array(vec![]))).is_err());
    }

    #[test]
    fn arbitrary_text_never_panics() {
        for n in 0..300usize {
            let s: String = (0..n)
                .map(|i| b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:"[(i * 7 + n) % 45] as char)
                .collect();
            let _ = parse(&format!("{QR_PREFIX}{s}"));
        }
    }
}
