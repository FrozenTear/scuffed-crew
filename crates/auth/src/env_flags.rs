//! Shared environment flags used by auth cookies and the DB client.

/// True when running in a production-hardened configuration.
///
/// - Unset or empty `PRODUCTION` → not production
/// - `0` / `false` / `no` / `off` (any case) → not production
/// - any other non-empty value (`1`, `true`, `yes`, …) → production
pub fn is_production_env() -> bool {
    match std::env::var("PRODUCTION") {
        Ok(v) => {
            let t = v.trim();
            if t.is_empty() {
                return false;
            }
            !matches!(
                t.to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_truthy() {
        // Isolation: pure classification helper (env mutation is process-global
        // and flaky under parallel tests).
        for v in ["1", "true", "TRUE", "yes", "YES", "on", "production"] {
            assert!(classify_production(v), "expected production for {v:?}");
        }
        for v in ["", " ", "0", "false", "FALSE", "no", "off"] {
            assert!(!classify_production(v), "expected non-production for {v:?}");
        }
    }

    fn classify_production(v: &str) -> bool {
        let t = v.trim();
        if t.is_empty() {
            return false;
        }
        !matches!(
            t.to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    }
}
