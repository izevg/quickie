//! The application's `Entropy`: the system's cryptographic generator, for what a page must not
//! be able to predict, such as the marker of the fence its text travels in.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

use quickie_application::Entropy;
use quickie_domain_agents::FenceMarker;
use security_framework::random::SecRandom;

/// Random bytes from `SecRandomCopyBytes`.
#[derive(Debug, Default)]
pub struct SystemEntropy;

impl Entropy for SystemEntropy {
    fn fence_marker(&self) -> FenceMarker {
        let mut bytes = [0_u8; 6];
        if SecRandom::default().copy_bytes(&mut bytes).is_err() {
            // The generator does not fail in practice. If it did, std's hasher keys, which
            // come from the system's randomness too, are still unknown to the page.
            let fallback = RandomState::new().hash_one(0_u8).to_le_bytes();
            bytes.copy_from_slice(&fallback[..6]);
        }
        FenceMarker::from_random(bytes)
    }
}

#[cfg(test)]
mod tests {
    use quickie_application::Entropy;

    use super::SystemEntropy;

    #[test]
    fn a_marker_is_twelve_hexadecimal_digits() {
        let marker = SystemEntropy.fence_marker();

        assert_eq!(marker.as_str().len(), 12);
        assert!(marker.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn markers_differ_from_draw_to_draw() {
        let markers: std::collections::HashSet<String> = (0..64)
            .map(|_| SystemEntropy.fence_marker().to_string())
            .collect();

        assert_eq!(markers.len(), 64);
    }
}
