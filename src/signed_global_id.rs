//! Rails 8.1 / GlobalID 1.4 Action Text signed IDs for the pinned Fizzy app.
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::Sha256;

pub(crate) struct Signer {
    global_id_key: [u8; 64],
    active_storage_key: [u8; 64],
}

impl Signer {
    pub(crate) fn new(secret_key_base: &str) -> Self {
        let mut global_id_key = [0; 64];
        let mut active_storage_key = [0; 64];
        // Rails.application.key_generator.generate_key("signed_global_ids")
        // uses 1,000 PBKDF2-HMAC-SHA256 rounds and a 64-byte output.
        pbkdf2_hmac::<Sha256>(
            secret_key_base.as_bytes(),
            b"signed_global_ids",
            1000,
            &mut global_id_key,
        );
        pbkdf2_hmac::<Sha256>(
            secret_key_base.as_bytes(),
            b"ActiveStorage",
            1000,
            &mut active_storage_key,
        );
        Self {
            global_id_key,
            active_storage_key,
        }
    }

    pub(crate) fn user(&self, user_id: &str) -> String {
        self.sign(
            &self.global_id_key,
            &format!("gid://fizzy/User/{user_id}"),
            "attachable",
        )
    }

    pub(crate) fn blob(&self, blob_id: &str) -> String {
        self.sign(
            &self.global_id_key,
            &format!("gid://fizzy/ActiveStorage::Blob/{blob_id}"),
            "attachable",
        )
    }

    pub(crate) fn blob_signed_id(&self, blob_id: &str) -> String {
        self.sign(&self.active_storage_key, blob_id, "blob_id")
    }

    fn sign(&self, key: &[u8; 64], data: &str, purpose: &str) -> String {
        let envelope = json!({"_rails": {"data": data, "pur": purpose}});
        let encoded = URL_SAFE.encode(serde_json::to_vec(&envelope).expect("verifier envelope"));
        let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC key");
        mac.update(encoded.as_bytes());
        format!("{encoded}--{}", hex::encode(mac.finalize().into_bytes()))
    }

    pub(crate) fn parse_user(&self, signed: &str) -> Option<String> {
        let (encoded, digest) = signed.rsplit_once("--")?;
        let bytes = hex::decode(digest).ok()?;
        let mut mac = Hmac::<Sha1>::new_from_slice(&self.global_id_key).ok()?;
        mac.update(encoded.as_bytes());
        mac.verify_slice(&bytes).ok()?;
        let envelope: Value = serde_json::from_slice(&URL_SAFE.decode(encoded).ok()?).ok()?;
        if envelope["_rails"]["pur"] != "attachable" || envelope["_rails"].get("exp").is_some() {
            return None;
        }
        let id = envelope["_rails"]["data"]
            .as_str()?
            .strip_prefix("gid://fizzy/User/")?;
        if id.is_empty() || id.contains('/') || id.contains('?') || id.contains('#') {
            return None;
        }
        Some(id.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::Signer;

    #[test]
    fn matches_fizzy_rails_verifier_and_rejects_tampering() {
        // Produced by pinned Fizzy's SignedGlobalID.verifier with this secret.
        let signer =
            Signer::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
        let token = signer.user("test");
        assert_eq!(
            token,
            "eyJfcmFpbHMiOnsiZGF0YSI6ImdpZDovL2Zpenp5L1VzZXIvdGVzdCIsInB1ciI6ImF0dGFjaGFibGUifX0=--1f5ec34b9a28ad7e8409913493ec6898f05c21ee"
        );
        assert_eq!(signer.parse_user(&token).as_deref(), Some("test"));
        let tampered = token.replacen('e', "f", 1);
        assert!(signer.parse_user(&tampered).is_none());
        assert!(signer.parse_user(&format!("{token}x")).is_none());
        assert_eq!(
            signer.blob("test"),
            "eyJfcmFpbHMiOnsiZGF0YSI6ImdpZDovL2Zpenp5L0FjdGl2ZVN0b3JhZ2U6OkJsb2IvdGVzdCIsInB1ciI6ImF0dGFjaGFibGUifX0=--d8df1975d0b338192728eb82a29a4054783d89dd"
        );
        assert_eq!(
            signer.blob_signed_id("test"),
            "eyJfcmFpbHMiOnsiZGF0YSI6InRlc3QiLCJwdXIiOiJibG9iX2lkIn19--e5c891d5395c0dc305c03cd2c1777213e8dc687f"
        );
    }
}
