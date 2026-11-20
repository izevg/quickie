//! CRX3, the Chrome Web Store's package format, checked the way Chrome checks it
//! (`components/crx_file/crx_verifier.cc`): the magic `Cr24`, version 3, a protobuf
//! `CrxFileHeader`, then a ZIP archive. The header's signed data names the extension id; every
//! key proof in it must sign `"CRX3 SignedData\0"`, the signed data's length (four bytes,
//! little-endian), the signed data, and the archive; and one proof's key must be the id's: its
//! SHA-256 starts with the id's 16 bytes.
//!
//! ```text
//! message CrxFileHeader {
//!   repeated AsymmetricKeyProof sha256_with_rsa = 2;
//!   repeated AsymmetricKeyProof sha256_with_ecdsa = 3;
//!   optional bytes signed_header_data = 10000;   // a SignedData
//! }
//! message AsymmetricKeyProof { optional bytes public_key = 1; optional bytes signature = 2; }
//! message SignedData { optional bytes crx_id = 1; }
//! ```

use quickie_domain_settings::ExtensionId;

use super::crypto::{Scheme, sha256, verifies};

/// What every CRX starts with.
const MAGIC: &[u8; 4] = b"Cr24";

/// What the signatures sign before the signed data.
const SIGNED_PREFIX: &[u8; 16] = b"CRX3 SignedData\0";

/// A package whose signatures hold: the id they prove, and its ZIP archive.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Verified<'a> {
    pub(super) id: ExtensionId,
    pub(super) archive: &'a [u8],
}

/// Why a CRX is refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum CrxError {
    #[error("not a CRX package")]
    NotCrx,
    #[error("CRX version {0} is not supported; only CRX3 is")]
    Version(u32),
    #[error("the CRX header is malformed")]
    Header,
    #[error("the CRX header names no extension id")]
    NoId,
    #[error("the CRX has no signature")]
    Unsigned,
    #[error("a CRX signature does not hold")]
    BadSignature,
    #[error("no CRX signature is by the extension's own key")]
    NotTheDevelopersKey,
}

/// Checks a CRX3 package's signatures; returns the id they prove and the archive.
pub(super) fn verify(crx: &[u8]) -> Result<Verified<'_>, CrxError> {
    let rest = crx.strip_prefix(MAGIC).ok_or(CrxError::NotCrx)?;
    let (version, rest) = le_u32(rest).ok_or(CrxError::NotCrx)?;
    if version != 3 {
        return Err(CrxError::Version(version));
    }
    let (header_len, rest) = le_u32(rest).ok_or(CrxError::Header)?;
    let header_len = usize::try_from(header_len).map_err(|_| CrxError::Header)?;
    if rest.len() < header_len {
        return Err(CrxError::Header);
    }
    let (header, archive) = rest.split_at(header_len);
    let mut proofs = Vec::new();
    let mut signed = None;
    for field in Fields::new(header) {
        match field.ok_or(CrxError::Header)? {
            (2, Value::Bytes(proof)) => proofs.push((Scheme::RsaSha256, key_proof(proof)?)),
            (3, Value::Bytes(proof)) => proofs.push((Scheme::EcdsaSha256, key_proof(proof)?)),
            (10_000, Value::Bytes(data)) => signed = Some(data),
            _ => {}
        }
    }
    let signed = signed.ok_or(CrxError::NoId)?;
    let mut crx_id = None;
    for field in Fields::new(signed) {
        if let (1, Value::Bytes(id)) = field.ok_or(CrxError::Header)? {
            crx_id = Some(id);
        }
    }
    let crx_id: &[u8; 16] = crx_id
        .and_then(|id| id.try_into().ok())
        .ok_or(CrxError::NoId)?;
    if proofs.is_empty() {
        return Err(CrxError::Unsigned);
    }
    let signed_len = u32::try_from(signed.len()).map_err(|_| CrxError::Header)?;
    let mut message = Vec::with_capacity(SIGNED_PREFIX.len() + 4 + signed.len() + archive.len());
    message.extend_from_slice(SIGNED_PREFIX);
    message.extend_from_slice(&signed_len.to_le_bytes());
    message.extend_from_slice(signed);
    message.extend_from_slice(archive);
    let mut developer = false;
    for (scheme, (key, signature)) in proofs {
        if !verifies(scheme, key, &message, signature) {
            return Err(CrxError::BadSignature);
        }
        developer |= sha256(&[key]).starts_with(crx_id);
    }
    if !developer {
        return Err(CrxError::NotTheDevelopersKey);
    }
    Ok(Verified {
        id: ExtensionId::from_digest(crx_id),
        archive,
    })
}

/// A key proof's public key and signature.
fn key_proof(proof: &[u8]) -> Result<(&[u8], &[u8]), CrxError> {
    let (mut key, mut signature) = (None, None);
    for field in Fields::new(proof) {
        match field.ok_or(CrxError::Header)? {
            (1, Value::Bytes(bytes)) => key = Some(bytes),
            (2, Value::Bytes(bytes)) => signature = Some(bytes),
            _ => {}
        }
    }
    key.zip(signature).ok_or(CrxError::Header)
}

fn le_u32(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let (head, rest) = bytes.split_first_chunk::<4>()?;
    Some((u32::from_le_bytes(*head), rest))
}

/// A protobuf field's value; only length-delimited ones are read, the rest skipped.
#[derive(Debug, PartialEq, Eq)]
enum Value<'a> {
    Bytes(&'a [u8]),
    Other,
}

/// The fields of a protobuf message, in order; `None` for a malformed one, after which the
/// iteration ends.
struct Fields<'a> {
    rest: &'a [u8],
    failed: bool,
}

impl<'a> Fields<'a> {
    fn new(message: &'a [u8]) -> Self {
        Self {
            rest: message,
            failed: false,
        }
    }

    fn field(&mut self) -> Option<(u64, Value<'a>)> {
        let key = self.varint()?;
        let value = match key & 7 {
            0 => {
                self.varint()?;
                Value::Other
            }
            1 => {
                self.take(8)?;
                Value::Other
            }
            2 => {
                let len = usize::try_from(self.varint()?).ok()?;
                Value::Bytes(self.take(len)?)
            }
            5 => {
                self.take(4)?;
                Value::Other
            }
            _ => return None,
        };
        Some((key >> 3, value))
    }

    fn varint(&mut self) -> Option<u64> {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = self.rest.split_first()?;
            self.rest = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        if self.rest.len() < len {
            return None;
        }
        let (taken, rest) = self.rest.split_at(len);
        self.rest = rest;
        Some(taken)
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = Option<(u64, Value<'a>)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.rest.is_empty() {
            return None;
        }
        let field = self.field();
        self.failed = field.is_none();
        Some(field)
    }
}
