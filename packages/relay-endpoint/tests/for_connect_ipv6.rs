//! An IPv6 relay, whose address has to keep its brackets in the URL.
//!
//! `Url::set_host` parses its argument the way a URL host is parsed, so an IPv6
//! literal without brackets loses its last group to the port: `2001:db8::1`
//! became `2001:4222` once, which is a valid URL to a machine that does not
//! exist. That is what `address_url`'s bracket branch exists for, and it needs a
//! resolver that answers with a v6 address — its own process, because the plan is
//! decided once.

use std::net::UdpSocket;

/// A nameserver that answers every `AAAA` question with `::1`.
fn serve_ipv6_nameserver() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind the stub nameserver");
    let port = socket.local_addr().expect("stub address").port();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 512];
        while let Ok((length, peer)) = socket.recv_from(&mut buffer) {
            let request = &buffer[..length];
            if length < 16 {
                continue;
            }
            let qtype = u16::from_be_bytes([request[length - 4], request[length - 3]]);
            let mut reply = Vec::with_capacity(length + 28);
            reply.extend_from_slice(&request[0..2]);
            reply.extend_from_slice(&[0x81, 0x80]);
            reply.extend_from_slice(&[0x00, 0x01]);
            reply.extend_from_slice(&if qtype == 28 { 1u16 } else { 0u16 }.to_be_bytes());
            reply.extend_from_slice(&[0, 0, 0, 0]);
            reply.extend_from_slice(&request[12..]);
            if qtype == 28 {
                reply.extend_from_slice(&[0xC0, 0x0C, 0x00, 0x1C, 0x00, 0x01]); // AAAA, IN
                reply.extend_from_slice(&60u32.to_be_bytes());
                reply.extend_from_slice(&[0x00, 0x10]); // 16-byte address
                reply.extend_from_slice(&u128::from(std::net::Ipv6Addr::LOCALHOST).to_be_bytes());
            }
            let _ = socket.send_to(&reply, peer);
        }
    });
    port
}

#[tokio::test]
async fn an_ipv6_relay_keeps_its_brackets_in_the_rewritten_url() {
    let port = serve_ipv6_nameserver();
    std::env::set_var("FUTURE_DNS_SERVER", format!("127.0.0.1:{port}"));
    std::env::remove_var("FUTURE_RESOLV_CONF");

    let endpoint = future_relay_endpoint::for_connect("nats://relay.example:4222", None, true)
        .await
        .expect("a v6-only name is still a name with an address");

    // Brackets, port and scheme: `[::1]:4222`, not `::4222`-style corruption.
    assert!(endpoint.url.contains("[::1]:4222"), "{}", endpoint.url);
    assert!(!endpoint.url.contains("relay.example"), "{}", endpoint.url);
}
