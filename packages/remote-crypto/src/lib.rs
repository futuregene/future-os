//! Remote v2 cryptography, independent of NATS and the desktop host.
//!
//! Noise authenticates the connection; ChaCha20-Poly1305 records bind the
//! routing context and an explicit sequence number. NATS may reorder records.
//! Never interpret an unauthenticated record or retry it as plaintext.
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use zeroize::{Zeroize, Zeroizing};

pub const VERSION: u32 = 2;
pub const HEADER_LEN: usize = 28;
pub const TAG_LEN: usize = 16;
pub const MAX_PLAINTEXT: usize = 1_048_576 - HEADER_LEN - TAG_LEN;
const WINDOW: u64 = 4096;
// Force a new handshake well before the AEAD's per-key usage limits.
const MAX_SEQUENCE: u64 = u32::MAX as u64;
const MAGIC: &[u8; 4] = b"FRE2";

#[derive(Debug, thiserror::Error)]
#[error("remote_secure_channel_invalid")]
pub struct Error;
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy)]
pub enum Pattern {
    Pair,
    Reconnect,
}
impl Pattern {
    fn name(self) -> &'static str {
        match self {
            Self::Pair => "Noise_XXpsk0_25519_ChaChaPoly_BLAKE2b",
            Self::Reconnect => "Noise_IK_25519_ChaChaPoly_BLAKE2b",
        }
    }
}

pub fn generate_identity() -> Result<(Zeroizing<Vec<u8>>, Vec<u8>)> {
    let pair = snow::Builder::new(Pattern::Pair.name().parse().map_err(|_| Error)?)
        .generate_keypair()
        .map_err(|_| Error)?;
    Ok((Zeroizing::new(pair.private), pair.public))
}

pub fn prologue(pair_id: &str, desktop_id: &str) -> Result<Vec<u8>> {
    if [pair_id, desktop_id].iter().any(|s| {
        s.is_empty()
            || s.len() > 128
            || !s
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    }) {
        return Err(Error);
    }
    Ok(format!("futureos-remote-v2\n{pair_id}\n{desktop_id}").into_bytes())
}

pub struct Handshake {
    state: snow::HandshakeState,
    initiator: bool,
}
impl Handshake {
    pub fn new(
        pattern: Pattern,
        initiator: bool,
        private: &[u8],
        remote: Option<&[u8]>,
        secret: Option<&[u8; 32]>,
        prologue: &[u8],
    ) -> Result<Self> {
        let mut builder = snow::Builder::new(pattern.name().parse().map_err(|_| Error)?)
            .local_private_key(private)
            .map_err(|_| Error)?
            .prologue(prologue)
            .map_err(|_| Error)?;
        if let Some(remote) = remote {
            builder = builder.remote_public_key(remote).map_err(|_| Error)?;
        }
        if let Some(secret) = secret {
            builder = builder.psk(0, secret).map_err(|_| Error)?;
        }
        let state = if initiator {
            builder.build_initiator()
        } else {
            builder.build_responder()
        }
        .map_err(|_| Error)?;
        Ok(Self { state, initiator })
    }
    pub fn write(&mut self, payload: &[u8]) -> Result<Vec<u8>> {
        if payload.len() > 4096 {
            return Err(Error);
        }
        let mut out = vec![0; 8192];
        let len = self
            .state
            .write_message(payload, &mut out)
            .map_err(|_| Error)?;
        out.truncate(len);
        Ok(out)
    }
    pub fn read(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        if message.len() > 8192 {
            return Err(Error);
        }
        let mut out = vec![0; 8192];
        let len = self
            .state
            .read_message(message, &mut out)
            .map_err(|_| Error)?;
        out.truncate(len);
        Ok(out)
    }
    pub fn remote_public_key(&self) -> Result<&[u8]> {
        self.state.get_remote_static().ok_or(Error)
    }
    /// The caller MUST validate/persist the remote identity before activating
    /// this channel. Initial pairing also requires the one-time QR PSK.
    pub fn finish(mut self) -> Result<Channel> {
        if !self.state.is_handshake_finished() {
            return Err(Error);
        }
        let id: [u8; 16] = self.state.get_handshake_hash()[..16]
            .try_into()
            .map_err(|_| Error)?;
        let (mut first, mut second) = self.state.dangerously_get_raw_split();
        let (send, receive) = if self.initiator {
            (first, second)
        } else {
            (second, first)
        };
        first.zeroize();
        second.zeroize();
        Ok(Channel::new(send, receive, id))
    }
}

pub struct Channel {
    send: Zeroizing<[u8; 32]>,
    receive: Zeroizing<[u8; 32]>,
    id: [u8; 16],
    next: u64,
    highest: u64,
    seen: Vec<u32>,
}
impl Channel {
    fn new(send: [u8; 32], receive: [u8; 32], id: [u8; 16]) -> Self {
        Self {
            send: Zeroizing::new(send),
            receive: Zeroizing::new(receive),
            id,
            next: 0,
            highest: 0,
            seen: vec![0; WINDOW as usize],
        }
    }
    pub fn seal(&mut self, context: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
        if plaintext.len() > MAX_PLAINTEXT || self.next >= MAX_SEQUENCE {
            return Err(Error);
        }
        let sequence = self.next;
        self.next += 1;
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&self.id);
        header.extend_from_slice(&sequence.to_le_bytes());
        let aad = associated_data(&header, context)?;
        let nonce = nonce(sequence);
        let body = ChaCha20Poly1305::new((&*self.send).into())
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| Error)?;
        header.extend_from_slice(&body);
        Ok(header)
    }
    pub fn open(&mut self, context: &str, wire: &[u8]) -> Result<Vec<u8>> {
        if wire.len() < HEADER_LEN + TAG_LEN
            || wire.len() > MAX_PLAINTEXT + HEADER_LEN + TAG_LEN
            || &wire[..4] != MAGIC
            || wire[4..20] != self.id
        {
            return Err(Error);
        }
        let sequence = u64::from_le_bytes(wire[20..28].try_into().map_err(|_| Error)?);
        if sequence >= MAX_SEQUENCE
            || self.seen[(sequence % WINDOW) as usize] as u64 == sequence + 1
            || (self.highest >= WINDOW && sequence <= self.highest - WINDOW)
        {
            return Err(Error);
        }
        let aad = associated_data(&wire[..HEADER_LEN], context)?;
        let nonce = nonce(sequence);
        let plaintext = ChaCha20Poly1305::new((&*self.receive).into())
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &wire[HEADER_LEN..],
                    aad: &aad,
                },
            )
            .map_err(|_| Error)?;
        // Never let forged counters evict legitimate messages from the window.
        self.highest = self.highest.max(sequence);
        self.seen[(sequence % WINDOW) as usize] = (sequence + 1) as u32;
        Ok(plaintext)
    }
}

fn associated_data(header: &[u8], context: &str) -> Result<Vec<u8>> {
    if context.is_empty() || context.len() > 1024 || !context.is_ascii() {
        return Err(Error);
    }
    let mut aad = header.to_vec();
    aad.extend_from_slice(context.as_bytes());
    Ok(aad)
}
fn nonce(sequence: u64) -> [u8; 12] {
    let mut nonce = [0; 12];
    nonce[4..].copy_from_slice(&sequence.to_le_bytes());
    nonce
}

/// Replies bind the authenticated request's subject, channel, and sequence,
/// NOT merely the broker-supplied reply inbox. A broker can substitute inboxes.
pub fn reply_context(subject: &str, request: &[u8]) -> Result<String> {
    if request.len() < HEADER_LEN || &request[..4] != MAGIC {
        return Err(Error);
    }
    let mut context = format!("reply:{subject}:");
    use std::fmt::Write;
    for byte in &request[4..HEADER_LEN] {
        write!(context, "{byte:02x}").map_err(|_| Error)?;
    }
    if context.len() > 1024 {
        return Err(Error);
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (Channel, Channel) {
        let (a, _) = generate_identity().unwrap();
        let (b, public_b) = generate_identity().unwrap();
        let psk = [7; 32];
        let prologue = prologue("pair_1", "desktop_1").unwrap();
        let mut i = Handshake::new(Pattern::Pair, true, &a, None, Some(&psk), &prologue).unwrap();
        let mut r = Handshake::new(Pattern::Pair, false, &b, None, Some(&psk), &prologue).unwrap();
        r.read(&i.write(b"").unwrap()).unwrap();
        i.read(&r.write(b"").unwrap()).unwrap();
        assert_eq!(i.remote_public_key().unwrap(), public_b);
        r.read(&i.write(b"").unwrap()).unwrap();
        (i.finish().unwrap(), r.finish().unwrap())
    }
    #[test]
    fn javascript_noise_and_record_vectors() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/vectors.json")).unwrap();
        for vector in vectors.as_array().unwrap() {
            let name = vector["pattern"].as_str().unwrap();
            let protocol = format!("Noise_{name}_25519_ChaChaPoly_BLAKE2b");
            let prologue = prologue("pair_1", "desktop_1").unwrap();
            let private_i = [1; 32];
            let private_r = [2; 32];
            let ei = [3; 32];
            let er = [4; 32];
            let psk = [7; 32];
            let public_r = hex::decode(vector["desktopPublicKey"].as_str().unwrap()).unwrap();
            let mut bi = snow::Builder::new(protocol.parse().unwrap())
                .local_private_key(&private_i)
                .unwrap()
                .prologue(&prologue)
                .unwrap()
                .fixed_ephemeral_key_for_testing_only(&ei);
            let mut br = snow::Builder::new(protocol.parse().unwrap())
                .local_private_key(&private_r)
                .unwrap()
                .prologue(&prologue)
                .unwrap()
                .fixed_ephemeral_key_for_testing_only(&er);
            if name == "XXpsk0" {
                bi = bi.psk(0, &psk).unwrap();
                br = br.psk(0, &psk).unwrap();
            } else {
                bi = bi.remote_public_key(&public_r).unwrap();
            }
            let mut i = Handshake {
                state: bi.build_initiator().unwrap(),
                initiator: true,
            };
            let mut r = Handshake {
                state: br.build_responder().unwrap(),
                initiator: false,
            };
            for (n, expected) in vector["messages"].as_array().unwrap().iter().enumerate() {
                let (sender, receiver) = if n % 2 == 0 {
                    (&mut i, &mut r)
                } else {
                    (&mut r, &mut i)
                };
                let wire = sender.write(b"").unwrap();
                assert_eq!(
                    hex::encode(&wire),
                    expected.as_str().unwrap(),
                    "{name} message {n}"
                );
                assert!(receiver.read(&wire).unwrap().is_empty());
            }
            assert_eq!(
                hex::encode(i.state.get_handshake_hash()),
                vector["hash"].as_str().unwrap()
            );
            let (mut i, mut r) = (i.finish().unwrap(), r.finish().unwrap());
            let context = vector["context"].as_str().unwrap();
            let wire = i.seal(context, b"private prompt").unwrap();
            assert_eq!(hex::encode(&wire), vector["request"].as_str().unwrap());
            assert_eq!(r.open(context, &wire).unwrap(), b"private prompt");
            let reply_context = reply_context(context, &wire).unwrap();
            assert_eq!(reply_context, vector["replyContext"].as_str().unwrap());
            let reply = r.seal(&reply_context, b"private reply").unwrap();
            assert_eq!(hex::encode(&reply), vector["reply"].as_str().unwrap());
            assert_eq!(i.open(&reply_context, &reply).unwrap(), b"private reply");
        }
    }

    #[test]
    fn roundtrip_and_replay() {
        let (mut i, mut r) = pair();
        let request = i.seal("p.pair_1.cmd.list", b"private prompt").unwrap();
        assert!(!request.windows(14).any(|w| w == b"private prompt"));
        assert_eq!(
            r.open("p.pair_1.cmd.list", &request).unwrap(),
            b"private prompt"
        );
        assert!(r.open("p.pair_1.cmd.list", &request).is_err());
        let context = reply_context("p.pair_1.cmd.list", &request).unwrap();
        let reply = r.seal(&context, b"private reply").unwrap();
        assert_eq!(i.open(&context, &reply).unwrap(), b"private reply");
    }
    #[test]
    fn rejects_tampering_routing_reflection_and_plaintext() {
        let (mut i, mut r) = pair();
        let wire = i.seal("command", b"approval").unwrap();
        for index in 0..wire.len() {
            let mut changed = wire.clone();
            changed[index] ^= 1;
            assert!(r.open("command", &changed).is_err());
        }
        assert!(r.open("another-command", &wire).is_err());
        assert!(i.open("command", &wire).is_err());
        assert!(r
            .open("command", br#"{"type":"approval_decision"}"#)
            .is_err());
        assert_eq!(r.open("command", &wire).unwrap(), b"approval");
    }
    #[test]
    fn supports_bounded_out_of_order_and_large_records() {
        let (mut i, mut r) = pair();
        let first = i.seal("file", b"first").unwrap();
        let second = i.seal("file", &vec![42; MAX_PLAINTEXT]).unwrap();
        assert_eq!(r.open("file", &second).unwrap().len(), MAX_PLAINTEXT);
        assert_eq!(r.open("file", &first).unwrap(), b"first");
        assert!(i.seal("file", &vec![0; MAX_PLAINTEXT + 1]).is_err());
    }
    #[test]
    fn replay_window_boundaries_and_nonce_exhaustion() {
        let (mut i, mut r) = pair();
        let zero = i.seal("event", b"zero").unwrap();
        let one = i.seal("event", b"one").unwrap();
        i.next = WINDOW;
        let high = i.seal("event", b"high").unwrap();
        r.open("event", &high).unwrap();
        assert!(r.open("event", &zero).is_err());
        assert_eq!(r.open("event", &one).unwrap(), b"one");
        let wrapped = i.seal("event", b"wrapped slot").unwrap();
        r.open("event", &wrapped).unwrap();
        assert!(r.open("event", &one).is_err());
        i.next = MAX_SEQUENCE - 1;
        let last = i.seal("event", b"last nonce").unwrap();
        assert!(r.open("event", &last).is_ok());
        assert!(i.seal("event", b"must rekey").is_err());
    }

    #[test]
    fn wrong_psk_and_wrong_prologue_fail() {
        let (a, _) = generate_identity().unwrap();
        let (b, _) = generate_identity().unwrap();
        for (psk, prologue) in [
            ([8; 32], b"correct".as_slice()),
            ([7; 32], b"wrong".as_slice()),
        ] {
            let mut i =
                Handshake::new(Pattern::Pair, true, &a, None, Some(&[7; 32]), b"correct").unwrap();
            let mut r =
                Handshake::new(Pattern::Pair, false, &b, None, Some(&psk), prologue).unwrap();
            assert!(r.read(&i.write(b"").unwrap()).is_err());
        }
    }
}
