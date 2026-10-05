//! Shared environment flags used by auth cookies and the DB client.

/// True when running in a production-hardened configuration.
///
/// - Unset or empty `PRODUCTION` → not production
/// - `0` / `false` / `no` / `off` (any case) → not production
/// - any other non-empty value (`1`, `true`, `yes`, …) → production
pub fn is_production_env() -> bool {
    match std::env::var("PRODUCTION") {
        Ok(v) => production_value_enabled(&v),
        Err(_) => false,
    }
}

/// Classify one `PRODUCTION` value the same way [`is_production_env`] does.
///
/// Empty or whitespace-only is not production. `0` / `false` / `no` / `off`
/// (any ASCII case) are not production. Any other non-empty value is,
/// including `on`, `True`, and `yes`.
pub fn production_value_enabled(value: &str) -> bool {
    let t = value.trim();
    if t.is_empty() {
        return false;
    }
    !matches!(
        t.to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

#[cfg(test)]
mod tests {
    use super::production_value_enabled;

    #[test]
    fn production_truthy() {
        // Isolation: classify the value directly. Mutating `PRODUCTION` is
        // process-global and flaky under parallel tests.
        for v in [
            "1",
            "true",
            "TRUE",
            "True",
            "yes",
            "YES",
            "on",
            "ON",
            " yes ",
            "production",
        ] {
            assert!(production_value_enabled(v), "expected production for {v:?}");
        }
        for v in ["", " ", "0", "false", "FALSE", "False", "no", "off", "OFF"] {
            assert!(
                !production_value_enabled(v),
                "expected non-production for {v:?}"
            );
        }
    }
}
