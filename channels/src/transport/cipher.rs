//! AES-256-CBC with PKCS#7 padding, for the encrypted callbacks the WeChat
//! family of platforms uses.
//!
//! The platform hands the operator a 43-character base64 "encoding key". It
//! decodes to exactly 32 bytes — an AES-256 key — and the first 16 of those
//! bytes double as the IV. That convention is specific to these platforms, not
//! a general rule, so this is a narrow helper rather than a crypto toolkit.
//!
//! Chaining and padding are implemented here over the `aes` block cipher, so
//! the dependency stays at the primitive instead of growing a mode crate. Both
//! directions are checked against the RFC 3602 AES-256-CBC vectors, and against
//! a round trip, in `tests` below.
//!
//! Decrypting a callback is only safe **after** the platform's signature over
//! it has been verified: a ciphertext from the network is attacker-chosen
//! input. Callers verify first; see `providers::wecom`.

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use anyhow::{anyhow, bail, Result};

/// AES block size, and the size of the IV these platforms use.
pub const BLOCK: usize = 16;
/// Length of the decoded encoding key: AES-256 takes 32 bytes.
pub const KEY_BYTES: usize = 32;

/// One decoded encoding key, with the IV derived the way the platform derives
/// it.
pub struct AesKey {
    key: [u8; KEY_BYTES],
    iv: [u8; BLOCK],
}

/// Deliberately opaque.
///
/// A derived `Debug` would print the key into any log that formats this type,
/// and an encoding key is a shared secret. Tests need `Debug` only because
/// `Result::expect_err` requires it.
impl std::fmt::Debug for AesKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AesKey(<redacted>)")
    }
}

impl AesKey {
    /// Decode the base64 encoding key.
    ///
    /// The platform console shows 43 characters, which is 32 bytes of base64
    /// with the padding its final group needs stripped. Rather than assume that
    /// one `=` is always enough, the padding is normalized to a whole group —
    /// the real key takes one, and a key that carries its own padding (or needs
    /// two) is then rejected for its *length*, which is the useful message.
    pub fn parse(encoding_key: &str) -> Result<Self> {
        use base64::Engine;
        let trimmed = encoding_key.trim();
        if trimmed.is_empty() {
            bail!("the encoding key is empty");
        }
        let mut padded = trimmed.to_string();
        while !padded.len().is_multiple_of(4) {
            padded.push('=');
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&padded)
            .map_err(|error| anyhow!("the encoding key is not base64: {error}"))?;
        let key: [u8; KEY_BYTES] = decoded.as_slice().try_into().map_err(|_| {
            anyhow!(
                "the encoding key decodes to {} bytes, not {KEY_BYTES}",
                decoded.len()
            )
        })?;
        let mut iv = [0u8; BLOCK];
        iv.copy_from_slice(&key[..BLOCK]);
        Ok(Self { key, iv })
    }
}

/// The key's bytes, in the order the platform derives the IV from them. Only
/// tests need to look; production goes through [`encrypt`] / [`decrypt`].
#[cfg(test)]
fn key_bytes(key: &AesKey) -> &[u8; KEY_BYTES] {
    &key.key
}

/// Decrypt a padded ciphertext.
pub fn decrypt(key: &AesKey, ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(BLOCK) {
        bail!(
            "the ciphertext is {} bytes, which is not a whole number of {BLOCK}-byte blocks",
            ciphertext.len()
        );
    }
    unpad(cbc_decrypt(&aes256(key), key.iv, ciphertext))
}

/// Encrypt a plaintext, padding it to the block size.
///
/// Production only ever decrypts — these platforms accept an empty response
/// where an encrypted reply would otherwise be needed, and the bridge sends
/// answers through the active-send API instead — so this exists to keep the
/// chaining honest: a round trip over many lengths is what makes `decrypt`
/// trustworthy.
pub fn encrypt(key: &AesKey, plaintext: &[u8]) -> Vec<u8> {
    cbc_encrypt(&aes256(key), key.iv, &pad(plaintext))
}

/// The cipher for a key whose length the type already guarantees.
fn aes256(key: &AesKey) -> Aes256 {
    Aes256::new_from_slice(&key.key).expect("AES-256 takes a 32-byte key")
}

/// PKCS#7: append `n` bytes of value `n`, a whole block when already aligned.
fn pad(plaintext: &[u8]) -> Vec<u8> {
    let padding = BLOCK - (plaintext.len() % BLOCK);
    let mut out = plaintext.to_vec();
    out.resize(out.len() + padding, padding as u8);
    out
}

/// Strip PKCS#7 padding, rejecting anything malformed.
///
/// A wrong key produces a wrong final byte with overwhelming probability, and
/// this is where that is noticed. The checks are separate because each one
/// names a different mistake to whoever reads the log.
fn unpad(mut data: Vec<u8>) -> Result<Vec<u8>> {
    let Some(&last) = data.last() else {
        bail!("the plaintext is empty");
    };
    let padding = last as usize;
    if padding == 0 || padding > BLOCK {
        bail!("the padding byte is {padding}, which cannot pad a {BLOCK}-byte block");
    }
    if padding > data.len() {
        bail!(
            "the padding claims {padding} bytes but only {} are present",
            data.len()
        );
    }
    let start = data.len() - padding;
    if data[start..].iter().any(|&byte| byte != last) {
        bail!("the padding is not uniform, so the key or the input is wrong");
    }
    data.truncate(start);
    Ok(data)
}

fn xor_block(block: &mut GenericArray<u8, aes::cipher::consts::U16>, mask: &[u8; BLOCK]) {
    for (byte, mask) in block.iter_mut().zip(mask.iter()) {
        *byte ^= mask;
    }
}

fn cbc_encrypt(cipher: &Aes256, iv: [u8; BLOCK], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut previous = iv;
    for chunk in data.chunks_exact(BLOCK) {
        let mut block = GenericArray::clone_from_slice(chunk);
        xor_block(&mut block, &previous);
        cipher.encrypt_block(&mut block);
        previous.copy_from_slice(&block);
        out.extend_from_slice(&block);
    }
    out
}

fn cbc_decrypt(cipher: &Aes256, iv: [u8; BLOCK], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut previous = iv;
    for chunk in data.chunks_exact(BLOCK) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        // Chain on the *ciphertext* just consumed, not on the plaintext.
        xor_block(&mut block, &previous);
        previous.copy_from_slice(chunk);
        out.extend_from_slice(&block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 43-character encoding key: what the platform console shows. 43
    /// base64 characters decode to exactly the 32 bytes AES-256 needs.
    const ENCODING_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    /// Same shape, different bytes: a stand-in for "the wrong key".
    const OTHER_KEY: &str = "WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo";

    fn from_hex(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex"))
            .collect()
    }

    fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
        bytes.try_into().expect("length")
    }

    /// RFC 3602 §4, AES-256-CBC, four blocks.
    const KEY: &str = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4";
    const IV: &str = "000102030405060708090a0b0c0d0e0f";
    const PLAIN: &str = concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710",
    );
    const CIPHER: &str = concat!(
        "f58c4c04d6e5f1ba779eabfb5f7bfbd6",
        "9cfc4e967edb808d679f777bc6702c7d",
        "39f23369a9d9bacfa530e26304231461",
        "b2eb05e2c39be9fcda6c19078c6a9d1b",
    );

    /// The RFC key with an IV chosen independently of its first bytes, which
    /// the platform convention would otherwise tie together.
    fn vector_key() -> AesKey {
        AesKey {
            key: array(&from_hex(KEY)),
            iv: array(&from_hex(IV)),
        }
    }

    #[test]
    fn the_rfc_vector_matches_in_both_directions() {
        let cipher = aes256(&vector_key());
        let plain = from_hex(PLAIN);
        let expected = from_hex(CIPHER);
        assert_eq!(
            cbc_encrypt(&cipher, array(&from_hex(IV)), &plain),
            expected,
            "encryption must match the published vector"
        );
        assert_eq!(
            cbc_decrypt(&cipher, array(&from_hex(IV)), &expected),
            plain,
            "decryption must match the published vector"
        );
    }

    #[test]
    fn a_round_trip_recovers_every_plaintext() {
        // Lengths on both sides of a block boundary, including the case where
        // padding has to add a whole block.
        let key = AesKey::parse(ENCODING_KEY).expect("parse");
        for length in [0usize, 1, 15, 16, 17, 31, 32, 33, 64] {
            let plaintext: Vec<u8> = (0..length).map(|index| index as u8).collect();
            let sealed = encrypt(&key, &plaintext);
            assert_eq!(sealed.len() % BLOCK, 0, "ciphertext is whole blocks");
            assert!(
                sealed.len() > plaintext.len(),
                "padding always adds at least one byte"
            );
            assert_eq!(decrypt(&key, &sealed).expect("decrypt"), plaintext);
        }
    }

    #[test]
    fn the_key_convention_matches_the_platform() {
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(format!("{ENCODING_KEY}="))
            .expect("base64");
        assert_eq!(decoded.len(), KEY_BYTES, "43 characters decode to 32 bytes");
        let key = AesKey::parse(ENCODING_KEY).expect("parse");
        assert_eq!(key_bytes(&key).as_slice(), decoded.as_slice());
        assert_eq!(
            key.iv.as_slice(),
            &decoded[..BLOCK],
            "the IV is the key prefix"
        );
        // Surrounding whitespace is tolerated: the key is pasted from a console.
        assert!(AesKey::parse(&format!("  {ENCODING_KEY}\n")).is_ok());
    }

    #[test]
    fn a_key_that_is_not_a_key_is_rejected_with_the_reason() {
        let empty = AesKey::parse("   ").expect_err("empty");
        assert!(empty.to_string().contains("empty"), "{empty}");

        let not_base64 = AesKey::parse("!!!not base64!!!").expect_err("not base64");
        assert!(not_base64.to_string().contains("base64"), "{not_base64}");

        // Valid base64, wrong length: 16 bytes, which is AES-128, not AES-256.
        let short = AesKey::parse("AAECAwQFBgcICQoLDA0ODw").expect_err("short");
        assert!(short.to_string().contains("16 bytes"), "{short}");
        assert!(short.to_string().contains("not 32"), "{short}");
    }

    #[test]
    fn a_ciphertext_that_is_not_whole_blocks_is_rejected() {
        let key = AesKey::parse(ENCODING_KEY).expect("parse");
        let empty = decrypt(&key, &[]).expect_err("empty");
        assert!(empty.to_string().contains("0 bytes"), "{empty}");

        let ragged = decrypt(&key, &[0u8; 20]).expect_err("ragged");
        assert!(ragged.to_string().contains("20 bytes"), "{ragged}");
    }

    #[test]
    fn a_wrong_key_is_reported_as_padding_damage() {
        let key = AesKey::parse(ENCODING_KEY).expect("parse");
        let sealed = encrypt(&key, b"a message that is longer than one block");
        let other = AesKey::parse(OTHER_KEY).expect("parse");
        let error = decrypt(&other, &sealed).expect_err("a wrong key must not decrypt");
        let message = error.to_string();
        assert!(
            message.contains("padding") || message.contains("plaintext"),
            "a wrong key must be reported as padding damage, not silence: {message}"
        );
    }

    #[test]
    fn every_malformed_padding_shape_has_its_own_message() {
        // Built by hand so each branch is reached deliberately rather than by
        // hoping a wrong key lands on it.
        let mut oversized = [0u8; BLOCK];
        oversized[BLOCK - 1] = (BLOCK + 1) as u8;
        let error = unpad(oversized.to_vec()).expect_err("padding larger than a block");
        assert!(error.to_string().contains("17"), "{error}");

        let mut zero = [0u8; BLOCK];
        zero[BLOCK - 1] = 0;
        let error = unpad(zero.to_vec()).expect_err("zero padding");
        assert!(error.to_string().contains("padding byte is 0"), "{error}");

        let mut ragged = [0u8; BLOCK];
        ragged[BLOCK - 1] = 3;
        ragged[BLOCK - 2] = 9; // not part of the run
        let error = unpad(ragged.to_vec()).expect_err("non-uniform padding");
        assert!(error.to_string().contains("not uniform"), "{error}");

        let error = unpad(Vec::new()).expect_err("empty plaintext");
        assert!(error.to_string().contains("empty"), "{error}");

        // Padding that claims more bytes than the buffer holds.
        let error = unpad(vec![5u8; 4]).expect_err("padding longer than the data");
        assert!(error.to_string().contains("claims 5 bytes"), "{error}");
    }

    #[test]
    fn padding_fills_to_the_block_boundary() {
        assert_eq!(pad(b"").len(), BLOCK);
        assert_eq!(pad(&[0u8; BLOCK]).len(), BLOCK * 2);
        assert_eq!(pad(b"abc").len(), BLOCK);
        assert_eq!(unpad(pad(b"abc")).expect("unpad"), b"abc");
        assert_eq!(unpad(pad(b"")).expect("unpad"), b"");
    }

    #[test]
    fn formatting_a_key_never_reveals_it() {
        // A key is a shared secret; a derived `Debug` would put it in any log
        // that formats this type, and logs outlive the incident.
        let key = AesKey::parse(ENCODING_KEY).expect("parse");
        let rendered = format!("{key:?}");
        assert_eq!(rendered, "AesKey(<redacted>)");
        assert!(!rendered.contains("AAECAwQ"), "{rendered}");
    }

    #[test]
    fn the_sizes_are_the_ones_the_platform_uses() {
        assert_eq!(BLOCK, 16);
        assert_eq!(KEY_BYTES, 32);
        assert_eq!(
            key_bytes(&AesKey::parse(ENCODING_KEY).expect("parse")).len(),
            KEY_BYTES
        );
    }
}
