//! Rails 8.1 / GlobalID 1.4 signed IDs for the pinned Fizzy app.
use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use chrono::{DateTime, SecondsFormat, Utc};
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

    pub(crate) fn email_change(
        &self,
        user_id: &str,
        old_email: &str,
        new_email: &str,
        expires_at: DateTime<Utc>,
    ) -> String {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("old_email_address", old_email)
            .append_pair("new_email_address", new_email)
            .append_pair("expires_in", "1800")
            .finish();
        let gid = format!("gid://fizzy/User/{user_id}?{query}");
        let envelope = json!({"_rails": {
            "data": gid,
            "exp": expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
            "pur": "change_email_address"
        }});
        // ActiveSupport's JSON encoder escapes ampersands in the URI before
        // MessageVerifier computes the signature.
        let serialized = serde_json::to_string(&envelope)
            .expect("email change verifier envelope")
            .replace('&', "\\u0026");
        self.sign_serialized(&self.global_id_key, serialized.as_bytes())
    }

    pub(crate) fn parse_email_change(
        &self,
        signed: &str,
        now: DateTime<Utc>,
    ) -> Option<(String, String, String)> {
        let (encoded, digest) = signed.rsplit_once("--")?;
        let bytes = hex::decode(digest).ok()?;
        let mut mac = Hmac::<Sha1>::new_from_slice(&self.global_id_key).ok()?;
        mac.update(encoded.as_bytes());
        mac.verify_slice(&bytes).ok()?;
        let envelope: Value = serde_json::from_slice(&URL_SAFE.decode(encoded).ok()?).ok()?;
        if envelope["_rails"]["pur"] != "change_email_address" {
            return None;
        }
        let expiration = DateTime::parse_from_rfc3339(envelope["_rails"]["exp"].as_str()?)
            .ok()?
            .with_timezone(&Utc);
        if now > expiration {
            return None;
        }
        let gid = url::Url::parse(envelope["_rails"]["data"].as_str()?).ok()?;
        if gid.scheme() != "gid" || gid.host_str()? != "fizzy" {
            return None;
        }
        let user_id = gid.path().strip_prefix("/User/")?;
        if user_id.is_empty() || user_id.contains('/') {
            return None;
        }
        let pairs: std::collections::HashMap<String, String> = gid
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        let old_email = pairs.get("old_email_address")?.to_owned();
        let new_email = pairs.get("new_email_address")?.to_owned();
        Some((user_id.to_owned(), old_email, new_email))
    }

    pub(crate) fn parse_variation(&self, signed: &str) -> Option<Value> {
        let (encoded, digest) = signed.rsplit_once("--")?;
        let bytes = hex::decode(digest).ok()?;
        let mut mac = Hmac::<Sha1>::new_from_slice(&self.active_storage_key).ok()?;
        mac.update(encoded.as_bytes());
        mac.verify_slice(&bytes).ok()?;
        let envelope: Value = serde_json::from_slice(&URL_SAFE.decode(encoded).ok()?).ok()?;
        if envelope["_rails"]["pur"] != "variation" || envelope["_rails"].get("exp").is_some() {
            return None;
        }
        let transformations = envelope["_rails"]["data"].as_object()?;
        Some(Value::Object(transformations.clone()))
    }

    fn sign(&self, key: &[u8; 64], data: &str, purpose: &str) -> String {
        let envelope = json!({"_rails": {"data": data, "pur": purpose}});
        self.sign_serialized(
            key,
            &serde_json::to_vec(&envelope).expect("verifier envelope"),
        )
    }

    fn sign_serialized(&self, key: &[u8; 64], serialized: &[u8]) -> String {
        let encoded = URL_SAFE.encode(serialized);
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
    use chrono::{TimeZone, Utc};

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
        let variation = "eyJfcmFpbHMiOnsiZGF0YSI6eyJsb2FkZXIiOnsibiI6LTF9LCJyZXNpemVfdG9fbGltaXQiOlsxMDAsMTAwXX0sInB1ciI6InZhcmlhdGlvbiJ9fQ==--9af012acd08c49a4255b2a56539cb4fc6cce8278";
        assert_eq!(
            signer.parse_variation(variation),
            Some(serde_json::json!({"loader":{"n":-1},"resize_to_limit":[100,100]}))
        );
        assert!(signer.parse_variation(&format!("{variation}x")).is_none());
    }

    #[test]
    fn email_change_token_matches_pinned_fizzy() {
        let signer =
            Signer::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
        let expires_at = Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 0).unwrap();
        let token = signer.email_change("test", "old@example.com", "new@example.com", expires_at);
        // Generated by pinned Fizzy's User#to_sgid with time frozen 30 minutes earlier.
        let fizzy = "eyJfcmFpbHMiOnsiZGF0YSI6ImdpZDovL2Zpenp5L1VzZXIvdGVzdD9vbGRfZW1haWxfYWRkcmVzcz1vbGQlNDBleGFtcGxlLmNvbVx1MDAyNm5ld19lbWFpbF9hZGRyZXNzPW5ldyU0MGV4YW1wbGUuY29tXHUwMDI2ZXhwaXJlc19pbj0xODAwIiwiZXhwIjoiMjAyNi0wOS0yOVQxMjowMDowMC4wMDBaIiwicHVyIjoiY2hhbmdlX2VtYWlsX2FkZHJlc3MifX0=--67a7bd423620e9ea0f37a7f000b462c828c42886";
        assert_eq!(token, fizzy);
        assert_eq!(
            signer.parse_email_change(fizzy, expires_at),
            Some((
                "test".into(),
                "old@example.com".into(),
                "new@example.com".into()
            ))
        );
        assert!(
            signer
                .parse_email_change(fizzy, expires_at + chrono::Duration::milliseconds(1))
                .is_none()
        );
        assert!(
            signer
                .parse_email_change(&format!("{fizzy}x"), expires_at)
                .is_none()
        );
    }
}
