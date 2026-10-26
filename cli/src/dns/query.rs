//! One A/AAAA query over UDP, and the wire format it needs.
//!
//! Deliberately small: no EDNS, no caching, no CNAME following (a server
//! answers a CNAME chain with the CNAME *and* the target's address records in
//! the same reply, and both are collected by address type), no DNSSEC. The
//! resolver it replaces — libc's, via a static musl build — does exactly this
//! much, so anything more would be a behavior change rather than a fix.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;

use super::config::Config;

pub(crate) const TYPE_A: u16 = 1;
pub(crate) const TYPE_AAAA: u16 = 28;
const CLASS_IN: u16 = 1;
/// Largest reply we accept on a 1500-byte path without EDNS.
const MAX_UDP_REPLY: usize = 1500;

/// What one server said about one name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reply {
    pub addresses: Vec<IpAddr>,
    /// DNS RCODE; 0 is NOERROR, 3 is NXDOMAIN.
    pub rcode: u8,
    pub truncated: bool,
}

/// Ask every nameserver for `name`, retrying the whole list when none answers.
///
/// A server that answers (even with "no such name") ends the retry loop: the
/// answer is authoritative for what this resolver can do, and repeating it
/// would only delay the error the caller sees.
pub(crate) async fn lookup(config: &Config, name: &str) -> Result<Reply, io::Error> {
    let mut last_failure = None;
    for _attempt in 0..config.attempts() {
        let mut collected: Vec<IpAddr> = Vec::new();
        let mut answered = false;
        let mut nxdomain = false;
        let mut truncated = false;
        for server in config.nameservers() {
            for qtype in [TYPE_A, TYPE_AAAA] {
                match query(*server, name, qtype, config.timeout()).await {
                    Ok(reply) => {
                        answered = true;
                        truncated |= reply.truncated;
                        if reply.rcode == 3 {
                            nxdomain = true;
                        }
                        for address in reply.addresses {
                            if !collected.contains(&address) {
                                collected.push(address);
                            }
                        }
                    }
                    Err(error) => {
                        last_failure = Some(error);
                    }
                }
                if !collected.is_empty() {
                    break;
                }
            }
            if !collected.is_empty() {
                break;
            }
        }
        if !collected.is_empty() || answered {
            return Ok(Reply {
                addresses: collected,
                rcode: u8::from(nxdomain),
                truncated,
            });
        }
        // Nobody answered; the next attempt repeats the whole list.
    }
    Err(last_failure.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "no answer for {name} within {}s",
                config.timeout().as_secs()
            ),
        )
    }))
}

async fn query(
    server: SocketAddr,
    name: &str,
    qtype: u16,
    timeout: Duration,
) -> Result<Reply, io::Error> {
    let id = rand::random::<u16>();
    let request = encode_query(id, name, qtype).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cannot encode a query for {name}: {error}"),
        )
    })?;
    let bind: SocketAddr = if server.is_ipv4() {
        SocketAddr::from(([0, 0, 0, 0], 0))
    } else {
        SocketAddr::from(([0u16; 8], 0))
    };
    let socket = UdpSocket::bind(bind).await?;
    let exchange = async {
        socket.connect(server).await?;
        socket.send(&request).await?;
        let mut buffer = vec![0u8; MAX_UDP_REPLY];
        let received = socket.recv(&mut buffer).await?;
        buffer.truncate(received);
        Ok::<Vec<u8>, io::Error>(buffer)
    };
    let response = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{server} did not answer a {} query for {name} within {}s",
                    record_type_name(qtype),
                    timeout.as_secs()
                ),
            )
        })??;
    decode_response(id, &response)
}

fn record_type_name(qtype: u16) -> &'static str {
    match qtype {
        TYPE_A => "A",
        TYPE_AAAA => "AAAA",
        _ => "?",
    }
}

/// Build one standard query: recursion desired, one question, no EDNS.
pub(crate) fn encode_query(id: u16, name: &str, qtype: u16) -> Result<Vec<u8>, String> {
    let name = name.trim_end_matches('.');
    if name.is_empty() {
        return Err("an empty name has nothing to ask".to_string());
    }
    let mut message = Vec::with_capacity(12 + name.len() + 5);
    message.extend_from_slice(&id.to_be_bytes());
    // QR=0, OPCODE=0, RD=1.
    message.extend_from_slice(&0x0100u16.to_be_bytes());
    message.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    message.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // AN/NS/AR
    let mut wire_length = 0usize;
    for label in name.split('.') {
        if label.is_empty() {
            return Err(format!("{name} has an empty label"));
        }
        if label.len() > 63 {
            return Err(format!("{name} has a label longer than 63 bytes"));
        }
        message.push(label.len() as u8);
        message.extend_from_slice(label.as_bytes());
        wire_length += label.len() + 1;
    }
    if wire_length > 253 {
        return Err(format!("{name} is longer than 253 bytes"));
    }
    message.push(0);
    message.extend_from_slice(&qtype.to_be_bytes());
    message.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(message)
}

pub(crate) fn decode_response(query_id: u16, bytes: &[u8]) -> Result<Reply, io::Error> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    if bytes.len() < 12 {
        return Err(invalid("a DNS reply shorter than its 12-byte header"));
    }
    let id = u16::from_be_bytes([bytes[0], bytes[1]]);
    if id != query_id {
        return Err(invalid("a DNS reply carrying a different transaction id"));
    }
    let flags = u16::from_be_bytes([bytes[2], bytes[3]]);
    if flags & 0x8000 == 0 {
        return Err(invalid("a DNS reply without the response bit set"));
    }
    let truncated = flags & 0x0200 != 0;
    let rcode = (flags & 0x000f) as u8;
    let questions = u16::from_be_bytes([bytes[4], bytes[5]]);
    let answers = u16::from_be_bytes([bytes[6], bytes[7]]);
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_name(bytes, offset)?;
        offset = skip(bytes, offset, 4)?;
    }
    let mut addresses = Vec::new();
    for _ in 0..answers {
        offset = skip_name(bytes, offset)?;
        let rtype = read_u16(bytes, offset)?;
        let class = read_u16(bytes, offset + 2)?;
        let length = read_u16(bytes, offset + 8)? as usize;
        let data = offset + 10;
        let rdata = bytes
            .get(data..data + length)
            .ok_or_else(|| invalid("an answer record longer than the reply"))?;
        if class == CLASS_IN {
            match (rtype, rdata.len()) {
                (TYPE_A, 4) => addresses.push(IpAddr::V4(Ipv4Addr::new(
                    rdata[0], rdata[1], rdata[2], rdata[3],
                ))),
                (TYPE_AAAA, 16) => {
                    let mut octets = [0u8; 16];
                    octets.copy_from_slice(rdata);
                    addresses.push(IpAddr::V6(Ipv6Addr::from(octets)));
                }
                _ => {}
            }
        }
        offset = data + length;
    }
    Ok(Reply {
        addresses,
        rcode,
        truncated,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, io::Error> {
    let pair = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a DNS record cut short"))?;
    Ok(u16::from_be_bytes([pair[0], pair[1]]))
}

fn skip(bytes: &[u8], offset: usize, count: usize) -> Result<usize, io::Error> {
    if offset + count > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a DNS record cut short",
        ));
    }
    Ok(offset + count)
}

/// Advance past a (possibly compressed) domain name.
pub(crate) fn skip_name(bytes: &[u8], mut offset: usize) -> Result<usize, io::Error> {
    loop {
        let length = *bytes
            .get(offset)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a DNS name cut short"))?;
        if length == 0 {
            return Ok(offset + 1);
        }
        if length & 0xc0 == 0xc0 {
            // A pointer is always the last element of the name.
            return skip(bytes, offset, 2);
        }
        if length & 0xc0 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a DNS name with an unsupported label type",
            ));
        }
        offset += 1 + length as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply_with(answers: &[(&str, u16, Vec<u8>)], rcode: u8) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(&7u16.to_be_bytes());
        message.extend_from_slice(&(0x8000u16 | 0x0100 | u16::from(rcode)).to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&(answers.len() as u16).to_be_bytes());
        message.extend_from_slice(&[0, 0, 0, 0]);
        // Question: example.com A IN
        message.extend_from_slice(b"\x07example\x03com\x00");
        message.extend_from_slice(&TYPE_A.to_be_bytes());
        message.extend_from_slice(&CLASS_IN.to_be_bytes());
        for (owner, rtype, rdata) in answers {
            for label in owner.split('.') {
                message.push(label.len() as u8);
                message.extend_from_slice(label.as_bytes());
            }
            if !owner.is_empty() {
                message.push(0);
            }
            message.extend_from_slice(&rtype.to_be_bytes());
            message.extend_from_slice(&CLASS_IN.to_be_bytes());
            message.extend_from_slice(&60u32.to_be_bytes());
            message.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
            message.extend_from_slice(rdata);
        }
        message
    }

    #[test]
    fn a_query_carries_one_question_and_the_recursion_bit() {
        let encoded = encode_query(0xbeef, "example.com", TYPE_A).unwrap();
        assert_eq!(&encoded[0..2], &[0xbe, 0xef]);
        assert_eq!(u16::from_be_bytes([encoded[2], encoded[3]]), 0x0100);
        assert_eq!(&encoded[4..6], &[0, 1]);
        assert!(encoded.ends_with(&[0, 1, 0, 1]));
        assert_eq!(&encoded[12..], b"\x07example\x03com\x00\x00\x01\x00\x01");
    }

    #[test]
    fn a_trailing_dot_is_the_same_name() {
        assert_eq!(
            encode_query(1, "example.com.", TYPE_A).unwrap(),
            encode_query(1, "example.com", TYPE_A).unwrap()
        );
    }

    #[test]
    fn an_impossible_name_is_refused_rather_than_sent() {
        assert!(encode_query(1, "", TYPE_A).is_err());
        assert!(encode_query(1, "a..b", TYPE_A).is_err());
        assert!(encode_query(1, &"x".repeat(64), TYPE_A).is_err());
    }

    #[test]
    fn addresses_of_the_requested_family_are_returned() {
        let bytes = reply_with(
            &[
                ("example.com", TYPE_A, vec![93, 184, 216, 34]),
                (
                    "example.com",
                    TYPE_AAAA,
                    vec![0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
                ),
            ],
            0,
        );
        let reply = decode_response(7, &bytes).unwrap();
        assert_eq!(
            reply.addresses,
            vec![
                "93.184.216.34".parse::<IpAddr>().unwrap(),
                "2001:db8::1".parse::<IpAddr>().unwrap(),
            ]
        );
        assert_eq!(reply.rcode, 0);
        assert!(!reply.truncated);
    }

    #[test]
    fn a_compressed_owner_name_inside_an_answer_is_skipped_correctly() {
        let mut bytes = reply_with(&[], 0);
        // One answer whose owner is a pointer back to the question at offset 12.
        bytes[6] = 0;
        bytes[7] = 1;
        bytes.extend_from_slice(&[0xc0, 0x0c]);
        bytes.extend_from_slice(&TYPE_A.to_be_bytes());
        bytes.extend_from_slice(&CLASS_IN.to_be_bytes());
        bytes.extend_from_slice(&60u32.to_be_bytes());
        bytes.extend_from_slice(&4u16.to_be_bytes());
        bytes.extend_from_slice(&[10, 0, 0, 9]);
        let reply = decode_response(7, &bytes).unwrap();
        assert_eq!(reply.addresses, vec!["10.0.0.9".parse::<IpAddr>().unwrap()]);
    }

    #[test]
    fn nxdomain_is_reported_as_a_rcode_not_an_error() {
        let bytes = reply_with(&[], 3);
        let reply = decode_response(7, &bytes).unwrap();
        assert!(reply.addresses.is_empty());
        assert_eq!(reply.rcode, 3);
    }

    #[test]
    fn a_reply_to_another_query_is_rejected() {
        let bytes = reply_with(&[], 0);
        assert!(decode_response(8, &bytes).is_err());
    }

    #[test]
    fn a_message_that_is_not_a_response_is_rejected() {
        let mut bytes = reply_with(&[], 0);
        bytes[2] = 0;
        assert!(decode_response(7, &bytes).is_err());
    }

    #[test]
    fn a_truncated_answer_keeps_what_fits_and_says_so() {
        let mut bytes = reply_with(&[("example.com", TYPE_A, vec![10, 0, 0, 1])], 0);
        bytes[2] |= 0x02;
        let reply = decode_response(7, &bytes).unwrap();
        assert!(reply.truncated);
        assert_eq!(reply.addresses, vec!["10.0.0.1".parse::<IpAddr>().unwrap()]);
    }

    #[test]
    fn a_record_longer_than_the_reply_is_an_error_not_a_panic() {
        let mut bytes = reply_with(&[("example.com", TYPE_A, vec![10, 0, 0, 1])], 0);
        let length = bytes.len();
        bytes[length - 6] = 0;
        bytes[length - 5] = 40;
        assert!(decode_response(7, &bytes).is_err());
    }
}
