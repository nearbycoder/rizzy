use chrono_tz::Tz;
use std::{collections::HashMap, str::FromStr, sync::OnceLock};

// Generated from ActiveSupport::TimeZone::MAPPING at the pinned Fizzy revision.
fn aliases() -> &'static HashMap<String, String> {
    static ALIASES: OnceLock<HashMap<String, String>> = OnceLock::new();
    ALIASES.get_or_init(|| {
        serde_json::from_str(include_str!("../static/timezone_aliases.json"))
            .expect("valid pinned ActiveSupport timezone mapping")
    })
}

pub(crate) fn resolve(name: &str) -> Option<Tz> {
    let iana = aliases().get(name).map(String::as_str).unwrap_or(name);
    Tz::from_str(iana).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pinned_alias_resolves() {
        assert_eq!(aliases().len(), 154);
        for (alias, iana) in aliases() {
            assert_eq!(resolve(alias), Tz::from_str(iana).ok(), "{alias}");
            assert!(
                resolve(alias).is_some(),
                "{alias} maps to unsupported {iana}"
            );
        }
        assert_eq!(
            resolve("America/New_York"),
            Some(chrono_tz::America::New_York)
        );
        assert_eq!(
            resolve("Eastern Time (US & Canada)"),
            Some(chrono_tz::America::New_York)
        );
        assert_eq!(resolve("Not/A_Time_Zone"), None);
    }
}
