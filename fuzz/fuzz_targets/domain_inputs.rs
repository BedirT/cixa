#![no_main]

use agent_treasury_domain::{canonicalize_domain, redact_sensitive, validate_https_url};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 4_096 {
        return;
    }
    if let Ok(value) = std::str::from_utf8(data) {
        let _ = canonicalize_domain(value);
        let _ = validate_https_url(value);
        let redacted = redact_sensitive(value);
        assert!(redacted.len() <= value.len().saturating_mul(4).saturating_add(64));
    }
});
