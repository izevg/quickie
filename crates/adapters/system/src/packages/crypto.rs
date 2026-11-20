//! The two primitives a package check needs, from the system: SHA-256 from `CommonCrypto` and
//! signature checks from Security.framework. No cryptography is implemented here.

use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use security_framework::key::{Algorithm, SecKey};
use security_framework_sys::item::{
    kSecAttrKeyClass, kSecAttrKeyClassPublic, kSecAttrKeyType, kSecAttrKeyTypeECSECPrimeRandom,
    kSecAttrKeyTypeRSA,
};
use security_framework_sys::key::SecKeyCreateWithData;

/// `CommonCrypto`'s SHA-256 state (`CC_SHA256_CTX`): two counters, eight hash words, and a
/// sixteen-word buffer.
#[repr(C)]
struct Sha256Context {
    count: [u32; 2],
    hash: [u32; 8],
    wbuf: [u32; 16],
}

// SAFETY: the declarations match CommonCrypto's `CommonDigest.h`; `libSystem` exports them.
unsafe extern "C" {
    fn CC_SHA256_Init(context: *mut Sha256Context) -> i32;
    fn CC_SHA256_Update(context: *mut Sha256Context, data: *const c_void, len: u32) -> i32;
    fn CC_SHA256_Final(digest: *mut u8, context: *mut Sha256Context) -> i32;
}

/// The SHA-256 digest of `parts`, one after the other.
pub(super) fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut context = Sha256Context {
        count: [0; 2],
        hash: [0; 8],
        wbuf: [0; 16],
    };
    // SAFETY: `context` is a valid, writable `CC_SHA256_CTX`.
    unsafe { CC_SHA256_Init(&raw mut context) };
    for part in parts {
        // `CC_LONG` is 32 bits: a part larger than that goes in pieces.
        for chunk in part.chunks(u32::MAX as usize) {
            let len = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
            // SAFETY: `chunk` is readable for `len` bytes and `context` was initialized.
            unsafe { CC_SHA256_Update(&raw mut context, chunk.as_ptr().cast(), len) };
        }
    }
    let mut digest = [0_u8; 32];
    // SAFETY: `digest` holds the 32 bytes SHA-256 writes; `context` was initialized.
    unsafe { CC_SHA256_Final(digest.as_mut_ptr(), &raw mut context) };
    digest
}

/// Which signature a key proves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scheme {
    /// RSASSA-PKCS1-v1_5 with SHA-256.
    RsaSha256,
    /// ECDSA on P-256 with SHA-256, the signature DER-encoded.
    EcdsaSha256,
}

/// Whether `signature` is `key`'s signature of `message` under `scheme`. `key` is a DER
/// `SubjectPublicKeyInfo`, as a CRX header carries it. Anything malformed does not verify.
pub(super) fn verifies(scheme: Scheme, key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    let Some(raw) = subject_public_key(key) else {
        return false;
    };
    let (kind, algorithm) = match scheme {
        // SAFETY: an immutable constant Security.framework exports.
        Scheme::RsaSha256 => (
            unsafe { kSecAttrKeyTypeRSA },
            Algorithm::RSASignatureMessagePKCS1v15SHA256,
        ),
        // SAFETY: as above.
        Scheme::EcdsaSha256 => (
            unsafe { kSecAttrKeyTypeECSECPrimeRandom },
            Algorithm::ECDSASignatureMessageX962SHA256,
        ),
    };
    // SAFETY: immutable constants Security.framework exports.
    let (type_key, class_key, public) =
        unsafe { (kSecAttrKeyType, kSecAttrKeyClass, kSecAttrKeyClassPublic) };
    let attributes = CFDictionary::from_CFType_pairs(&[
        (constant(type_key), constant(kind).as_CFType()),
        (constant(class_key), constant(public).as_CFType()),
    ]);
    let data = CFData::from_buffer(raw);
    let mut error = std::ptr::null_mut();
    // SAFETY: `data` and `attributes` are valid for the call; on failure the error is ours
    // to release, which wrapping it does.
    let key = unsafe {
        SecKeyCreateWithData(
            data.as_concrete_TypeRef(),
            attributes.as_concrete_TypeRef(),
            &raw mut error,
        )
    };
    if key.is_null() {
        if !error.is_null() {
            // SAFETY: a created error we own.
            drop(unsafe { core_foundation::error::CFError::wrap_under_create_rule(error) });
        }
        return false;
    }
    // SAFETY: a key created for us, released when the wrapper drops.
    let key = unsafe { SecKey::wrap_under_create_rule(key) };
    key.verify_signature(algorithm, message, signature)
        .unwrap_or(false)
}

/// A `CFString` constant Security.framework exports, retained for as long as it is used.
fn constant(raw: core_foundation::string::CFStringRef) -> CFString {
    // SAFETY: `raw` is one of Security.framework's constants, valid for the process's life;
    // the get rule retains it.
    unsafe { CFString::wrap_under_get_rule(raw) }
}

/// The key inside a DER `SubjectPublicKeyInfo`: `SEQUENCE { SEQUENCE algorithm, BIT STRING
/// key }`. For RSA that is the PKCS #1 `RSAPublicKey`, for EC the uncompressed point, as
/// `SecKeyCreateWithData` wants them.
pub(super) fn subject_public_key(der: &[u8]) -> Option<&[u8]> {
    let (tag, info, rest) = element(der)?;
    if tag != 0x30 || !rest.is_empty() {
        return None;
    }
    let (tag, _algorithm, after) = element(info)?;
    if tag != 0x30 {
        return None;
    }
    let (tag, bits, rest) = element(after)?;
    if tag != 0x03 || !rest.is_empty() {
        return None;
    }
    // The first byte counts unused bits; a key is whole bytes.
    match bits.split_first()? {
        (0, key) if !key.is_empty() => Some(key),
        _ => None,
    }
}

/// The first DER element of `der`: its tag, its content, and what follows it.
fn element(der: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = der.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let (bytes, rest) = rest.split_at(count);
        let len = bytes
            .iter()
            .fold(0_usize, |len, &byte| (len << 8) | usize::from(byte));
        (len, rest)
    };
    if rest.len() < len {
        return None;
    }
    let (content, after) = rest.split_at(len);
    Some((tag, content, after))
}
