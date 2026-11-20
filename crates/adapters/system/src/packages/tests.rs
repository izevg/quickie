//! Packages signed in the test with keys made for it, so no real extension is ever fetched.

use std::fs;

use quickie_application::{Expected, ExtensionPackages, PackageSource};
use quickie_domain_settings::ExtensionId;
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey};

use super::crx::{CrxError, verify};
use super::crypto::{sha256, subject_public_key};
use super::{ExtensionFiles, hex};

const ARCHIVE: &[u8] = b"PK\x03\x04 a zip archive, as far as the check cares";

/// `SEQUENCE { SEQUENCE { OID rsaEncryption, NULL }, BIT STRING }` around a PKCS #1 key.
const RSA_ALGORITHM: &[u8] = &[
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
];

/// `SEQUENCE { OID id-ecPublicKey, OID prime256v1 }`.
const EC_ALGORITHM: &[u8] = &[
    0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48,
    0xce, 0x3d, 0x03, 0x01, 0x07,
];

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(u8::try_from(len).unwrap());
    } else {
        let bytes: Vec<u8> = len
            .to_be_bytes()
            .into_iter()
            .skip_while(|&byte| byte == 0)
            .collect();
        out.push(0x80 | u8::try_from(bytes.len()).unwrap());
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

fn varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = u8::try_from(value & 0x7f).unwrap();
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn field(number: u64, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    varint(number << 3 | 2, &mut out);
    varint(u64::try_from(bytes.len()).unwrap(), &mut out);
    out.extend_from_slice(bytes);
    out
}

/// A signing key and its `SubjectPublicKeyInfo`.
struct Signer {
    private: SecKey,
    spki: Vec<u8>,
    algorithm: Algorithm,
    field: u64,
}

impl Signer {
    fn rsa() -> Self {
        let private = SecKey::new(
            GenerateKeyOptions::default()
                .set_key_type(KeyType::rsa())
                .set_size_in_bits(2048),
        )
        .unwrap();
        let pkcs1 = private
            .public_key()
            .unwrap()
            .external_representation()
            .unwrap()
            .to_vec();
        Self::wrap(
            private,
            RSA_ALGORITHM,
            &pkcs1,
            Algorithm::RSASignatureMessagePKCS1v15SHA256,
            2,
        )
    }

    fn ecdsa() -> Self {
        let private =
            SecKey::new(GenerateKeyOptions::default().set_key_type(KeyType::ec_sec_prime_random()))
                .unwrap();
        let point = private
            .public_key()
            .unwrap()
            .external_representation()
            .unwrap()
            .to_vec();
        Self::wrap(
            private,
            EC_ALGORITHM,
            &point,
            Algorithm::ECDSASignatureMessageX962SHA256,
            3,
        )
    }

    fn wrap(private: SecKey, algorithm: &[u8], key: &[u8], signing: Algorithm, field: u64) -> Self {
        let mut bits = vec![0];
        bits.extend_from_slice(key);
        let mut info = algorithm.to_vec();
        info.extend(der(0x03, &bits));
        Self {
            private,
            spki: der(0x30, &info),
            algorithm: signing,
            field,
        }
    }

    fn id(&self) -> [u8; 16] {
        sha256(&[&self.spki])[..16].try_into().unwrap()
    }
}

/// A CRX3 of `archive` whose signed data names `crx_id`, signed by each of `signers`.
fn crx(crx_id: &[u8], signers: &[&Signer], archive: &[u8]) -> Vec<u8> {
    let signed = field(1, crx_id);
    let mut message = b"CRX3 SignedData\0".to_vec();
    message.extend(u32::try_from(signed.len()).unwrap().to_le_bytes());
    message.extend(&signed);
    message.extend(archive);
    let mut header = Vec::new();
    for signer in signers {
        let signature = signer
            .private
            .create_signature(signer.algorithm, &message)
            .unwrap();
        let mut proof = field(1, &signer.spki);
        proof.extend(field(2, &signature));
        header.extend(field(signer.field, &proof));
    }
    header.extend(field(10_000, &signed));
    let mut out = b"Cr24".to_vec();
    out.extend(3_u32.to_le_bytes());
    out.extend(u32::try_from(header.len()).unwrap().to_le_bytes());
    out.extend(header);
    out.extend(archive);
    out
}

#[test]
fn sha256_is_the_standard_digest() {
    assert_eq!(
        hex(&sha256(&[b"ab", b"c"])),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&sha256(&[])),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn a_key_comes_out_of_its_public_key_info() {
    let signer = Signer::ecdsa();

    let key = subject_public_key(&signer.spki).unwrap();

    assert_eq!(key.len(), 65);
    assert_eq!(key[0], 0x04);
    assert_eq!(subject_public_key(&[0x30, 0x01]), None);
    assert_eq!(subject_public_key(&der(0x30, &der(0x30, &[]))), None);
}

#[test]
fn a_package_signed_by_the_developer_s_key_proves_its_id() {
    let developer = Signer::rsa();
    let store = Signer::rsa();

    let package = crx(&developer.id(), &[&developer, &store], ARCHIVE);
    let verified = verify(&package).unwrap();

    assert_eq!(verified.id, ExtensionId::from_digest(&developer.id()));
    assert_eq!(verified.archive, ARCHIVE);
}

#[test]
fn an_ecdsa_signature_counts_too() {
    let developer = Signer::ecdsa();

    let package = crx(&developer.id(), &[&developer], ARCHIVE);

    assert_eq!(
        verify(&package).map(|verified| verified.id),
        Ok(ExtensionId::from_digest(&developer.id()))
    );
}

#[test]
fn a_changed_archive_breaks_the_signature() {
    let developer = Signer::rsa();
    let mut package = crx(&developer.id(), &[&developer], ARCHIVE);
    let last = package.len() - 1;
    package[last] ^= 1;

    assert_eq!(verify(&package), Err(CrxError::BadSignature));
}

#[test]
fn an_id_no_signing_key_owns_is_refused() {
    let developer = Signer::rsa();

    let package = crx(&[7; 16], &[&developer], ARCHIVE);

    assert_eq!(verify(&package), Err(CrxError::NotTheDevelopersKey));
}

#[test]
fn malformed_packages_are_refused() {
    let developer = Signer::rsa();
    let good = crx(&developer.id(), &[&developer], ARCHIVE);
    let mut version_two = good.clone();
    version_two[4] = 2;
    let mut long_header = good.clone();
    long_header[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut no_proofs = b"Cr24".to_vec();
    let signed_only = field(10_000, &field(1, &developer.id()));
    no_proofs.extend(3_u32.to_le_bytes());
    no_proofs.extend(u32::try_from(signed_only.len()).unwrap().to_le_bytes());
    no_proofs.extend(&signed_only);
    let mut no_id = b"Cr24".to_vec();
    no_id.extend(3_u32.to_le_bytes());
    no_id.extend(0_u32.to_le_bytes());
    let mut short_id = b"Cr24".to_vec();
    let signed = field(10_000, &field(1, &[1; 8]));
    short_id.extend(3_u32.to_le_bytes());
    short_id.extend(u32::try_from(signed.len()).unwrap().to_le_bytes());
    short_id.extend(&signed);
    let mut bad_varint = b"Cr24".to_vec();
    bad_varint.extend(3_u32.to_le_bytes());
    bad_varint.extend(1_u32.to_le_bytes());
    bad_varint.push(0x80);

    assert_eq!(verify(b"PK\x03\x04"), Err(CrxError::NotCrx));
    assert_eq!(verify(b"Cr24"), Err(CrxError::NotCrx));
    assert_eq!(verify(&version_two), Err(CrxError::Version(2)));
    assert_eq!(verify(&long_header), Err(CrxError::Header));
    assert_eq!(verify(&no_proofs), Err(CrxError::Unsigned));
    assert_eq!(verify(&no_id), Err(CrxError::NoId));
    assert_eq!(verify(&short_id), Err(CrxError::NoId));
    assert_eq!(verify(&bad_varint), Err(CrxError::Header));
}

#[test]
fn a_verified_package_is_kept_once_under_its_id() {
    let root = tempfile::tempdir().unwrap();
    let files = ExtensionFiles::new(root.path().join("Extensions"));
    let developer = Signer::rsa();
    let package = crx(&developer.id(), &[&developer], ARCHIVE);
    let id = ExtensionId::from_digest(&developer.id());
    let expected = Expected {
        id: id.clone(),
        sha256: Some(hex(&sha256(&[&package]))),
    };

    let first = files
        .unpack(&PackageSource::Crx(package.clone()), Some(&expected))
        .unwrap();
    let again = files
        .unpack(&PackageSource::Crx(package), Some(&expected))
        .unwrap();

    assert_eq!(first, again);
    assert_eq!(first.id, id);
    let kept = std::path::Path::new(&first.location);
    assert_eq!(kept.extension().and_then(|e| e.to_str()), Some("zip"));
    assert!(first.location.contains(id.as_str()));
    assert_eq!(fs::read(&first.location).unwrap(), ARCHIVE);
}

#[test]
fn a_package_that_is_not_the_announced_one_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let files = ExtensionFiles::new(root.path().to_path_buf());
    let developer = Signer::rsa();
    let package = crx(&developer.id(), &[&developer], ARCHIVE);
    let other = ExtensionId::from_digest(&[0; 16]);

    let wrong_hash = files.unpack(
        &PackageSource::Crx(package.clone()),
        Some(&Expected {
            id: ExtensionId::from_digest(&developer.id()),
            sha256: Some("0".repeat(64)),
        }),
    );
    let wrong_id = files.unpack(
        &PackageSource::Crx(package),
        Some(&Expected {
            id: other,
            sha256: None,
        }),
    );

    assert!(wrong_hash.is_err());
    assert!(wrong_id.is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn files_and_folders_take_the_id_of_where_they_are() {
    let root = tempfile::tempdir().unwrap();
    let files = ExtensionFiles::new(root.path().join("Extensions"));
    let zip = root.path().join("test.zip");
    fs::write(&zip, ARCHIVE).unwrap();
    let folder = root.path().join("unpacked");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("manifest.json"), "{}").unwrap();
    let text = root.path().join("notes.txt");
    fs::write(&text, "hello").unwrap();

    let from_zip = files
        .unpack(&PackageSource::File(zip.clone()), None)
        .unwrap();
    let from_folder = files
        .unpack(&PackageSource::Folder(folder.clone()), None)
        .unwrap();

    assert_eq!(
        from_zip,
        files.unpack(&PackageSource::File(zip), None).unwrap()
    );
    assert_ne!(from_zip.id, from_folder.id);
    assert_eq!(
        from_folder.location,
        folder.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(files.unpack(&PackageSource::File(text), None).is_err());
    assert!(
        files
            .unpack(&PackageSource::Folder(root.path().to_path_buf()), None)
            .is_err()
    );
}

#[test]
fn pruning_keeps_only_what_is_named() {
    let root = tempfile::tempdir().unwrap();
    let files = ExtensionFiles::new(root.path().join("Extensions"));
    let developer = Signer::rsa();
    let kept = files
        .unpack(
            &PackageSource::Crx(crx(&developer.id(), &[&developer], ARCHIVE)),
            None,
        )
        .unwrap();
    let dropped = files
        .unpack(
            &PackageSource::Crx(crx(&developer.id(), &[&developer], b"PK\x03\x04 v2")),
            None,
        )
        .unwrap();
    let other = Signer::rsa();
    let gone = files
        .unpack(
            &PackageSource::Crx(crx(&other.id(), &[&other], ARCHIVE)),
            None,
        )
        .unwrap();

    files.prune(std::slice::from_ref(&kept.location)).unwrap();

    assert!(fs::metadata(&kept.location).is_ok());
    assert!(fs::metadata(&dropped.location).is_err());
    assert!(fs::metadata(&gone.location).is_err());
    assert!(fs::metadata(root.path().join("Extensions").join(gone.id.as_str())).is_err());
    ExtensionFiles::new(root.path().join("missing"))
        .prune(&[])
        .unwrap();
}
