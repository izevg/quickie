//! CRX3 digests and signatures through the system OpenSSL library.

use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{Id, PKey};
use openssl::rsa::Padding;
use openssl::sha::Sha256;
use openssl::sign::Verifier;

#[derive(Clone, Copy)]
pub(super) enum Scheme {
    RsaSha256,
    EcdsaSha256,
}

pub(super) fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
    }
    digest.finish()
}

pub(super) fn verifies(scheme: Scheme, key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    let Ok(key) = PKey::public_key_from_der(key) else {
        return false;
    };
    let matching_scheme = match scheme {
        Scheme::RsaSha256 => key.id() == Id::RSA,
        Scheme::EcdsaSha256 => key
            .ec_key()
            .is_ok_and(|key| key.group().curve_name() == Some(Nid::X9_62_PRIME256V1)),
    };
    if !matching_scheme {
        return false;
    }
    let Ok(mut verifier) = Verifier::new(MessageDigest::sha256(), &key) else {
        return false;
    };
    if matches!(scheme, Scheme::RsaSha256) && verifier.set_rsa_padding(Padding::PKCS1).is_err() {
        return false;
    }
    verifier.verify_oneshot(signature, message).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use openssl::ec::{EcGroup, EcKey};
    use openssl::pkey::Private;
    use openssl::rsa::Rsa;
    use openssl::sign::Signer;
    use quickie_application::{Expected, ExtensionPackages, PackageSource};
    use quickie_domain_settings::ExtensionId;

    use super::*;
    use crate::packages::archive::tests::test_zip;
    use crate::packages::{ExtensionFiles, hex};

    #[test]
    fn hashes_the_complete_package() {
        assert_eq!(
            hex(&sha256(&[b"ab", b"c"])),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn verifies_rsa_and_p256_signatures_only_for_the_correct_scheme_and_bytes() {
        let rsa = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let p256 = ec_key(Nid::X9_62_PRIME256V1);
        for (private, expected, wrong) in [
            (&rsa, Scheme::RsaSha256, Scheme::EcdsaSha256),
            (&p256, Scheme::EcdsaSha256, Scheme::RsaSha256),
        ] {
            let key = private.public_key_to_der().unwrap();
            let signature = sign(private);

            assert!(verifies(expected, &key, b"extension", &signature));
            assert!(!verifies(wrong, &key, b"extension", &signature));
            assert!(!verifies(expected, &key, b"changed", &signature));
            assert!(!verifies(expected, b"", b"extension", &signature));
        }
    }

    #[test]
    fn rejects_other_ecdsa_curves() {
        let private = ec_key(Nid::SECP384R1);
        assert!(!verifies(
            Scheme::EcdsaSha256,
            &private.public_key_to_der().unwrap(),
            b"extension",
            &sign(&private)
        ));
    }

    #[test]
    fn keeps_only_the_store_package_signed_for_the_expected_extension() {
        let root = tempfile::tempdir().unwrap();
        let files = ExtensionFiles::new(root.path().to_owned());
        let developer = ec_key(Nid::X9_62_PRIME256V1);
        let key = developer.public_key_to_der().unwrap();
        let id: [u8; 16] = sha256(&[&key])[..16].try_into().unwrap();
        let archive = test_zip(&[("manifest.json", b"{}"), ("content.js", b"fixture")]);
        let signed = field(&[0x0a], &id);
        let signed_len = u32::try_from(signed.len()).unwrap().to_le_bytes();
        let message = [&b"CRX3 SignedData\0"[..], &signed_len, &signed, &archive].concat();
        let signature = Signer::new(MessageDigest::sha256(), &developer)
            .unwrap()
            .sign_oneshot_to_vec(&message)
            .unwrap();
        let proof = [field(&[0x0a], &key), field(&[0x12], &signature)].concat();
        let header = [field(&[0x1a], &proof), field(&[0x82, 0xf1, 0x04], &signed)].concat();
        let header_len = u32::try_from(header.len()).unwrap().to_le_bytes();
        let package = [&b"Cr24\x03\0\0\0"[..], &header_len, &header, &archive].concat();
        let expected = Expected {
            id: ExtensionId::from_digest(&id),
            sha256: Some(hex(&sha256(&[&package]))),
        };

        let installed = files
            .unpack(&PackageSource::Crx(package.clone()), Some(&expected))
            .unwrap();
        let mut changed = package;
        *changed.last_mut().unwrap() ^= 1;

        assert_eq!(installed.id, expected.id);
        let installed_path = std::path::Path::new(&installed.location);
        assert_eq!(
            std::fs::read(installed_path.join("manifest.json")).unwrap(),
            b"{}"
        );
        assert_eq!(
            std::fs::read(installed_path.join("content.js")).unwrap(),
            b"fixture"
        );
        assert!(
            files
                .unpack(&PackageSource::Crx(changed.clone()), Some(&expected))
                .is_err()
        );
        assert!(files.unpack(&PackageSource::Crx(changed), None).is_err());
    }

    fn field(tag: &[u8], value: &[u8]) -> Vec<u8> {
        let len = u8::try_from(value.len()).unwrap();
        let length = if len < 128 {
            vec![len]
        } else {
            vec![len | 0x80, 1]
        };
        [tag, &length, value].concat()
    }

    fn ec_key(curve: Nid) -> PKey<Private> {
        let group = EcGroup::from_curve_name(curve).unwrap();
        PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap()
    }

    fn sign(private: &PKey<Private>) -> Vec<u8> {
        Signer::new(MessageDigest::sha256(), private)
            .unwrap()
            .sign_oneshot_to_vec(b"extension")
            .unwrap()
    }
}
