//! Argon2id password hashing for local (username/password) accounts.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use thiserror::Error;

/// Minimum length for setup / local login passwords.
pub const MIN_PASSWORD_LEN: usize = 12;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password hashing failed")]
    Hash,
    #[error("password verification failed")]
    Verify,
    #[error("invalid password hash format")]
    InvalidHash,
}

/// Hash a password with Argon2id (PHC string includes salt).
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| PasswordError::Hash)
}

/// Verify a password against a stored Argon2 PHC hash.
pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(password_hash).map_err(|_| PasswordError::InvalidHash)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// A precomputed Argon2 hash of a fixed dummy password.
///
/// Built lazily via [`hash_password`], so its cost parameters always match the
/// real login path (both use `Argon2::default()`). Used only to spend
/// equivalent verification time on the "no such user" branch — see
/// [`verify_dummy`].
static DUMMY_HASH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    hash_password("dr1-auth-002-constant-time-dummy-password")
        .expect("dummy Argon2 hash generation must succeed")
});

/// Spend Argon2 verify time against a fixed dummy hash and discard the result.
///
/// Login must call this on the branch where the username does not exist so that
/// path costs roughly the same as a real password verify. Without it, an
/// existing username incurs the (tens-of-ms) Argon2 cost while a missing one
/// returns in sub-millisecond time, leaking username existence via a timing
/// side-channel (DR1-AUTH-002). The boolean result is intentionally ignored.
pub fn verify_dummy(password: &str) {
    // Discard the outcome; the point is the elapsed work, not the answer.
    let _ = verify_password(password, &DUMMY_HASH);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash_password("correct-horse-battery").unwrap();
        assert!(verify_password("correct-horse-battery", &h).unwrap());
        assert!(!verify_password("wrong-password-xx", &h).unwrap());
    }

    #[test]
    fn dummy_verify_does_real_argon2_work() {
        use std::time::Instant;

        // Warm the lazy dummy hash + a real hash so neither first-call cost skews
        // the comparison.
        verify_dummy("warmup");
        let real = hash_password("a-real-user-password").unwrap();

        // The no-user path (verify_dummy) must spend comparable Argon2 time to a
        // real verify — not return instantly. We assert it is within an order of
        // magnitude of a real verify (generous bound to stay non-flaky in CI).
        let t0 = Instant::now();
        let _ = verify_password("some-guess", &real).unwrap();
        let real_dt = t0.elapsed();

        let t1 = Instant::now();
        verify_dummy("some-guess");
        let dummy_dt = t1.elapsed();

        // Dummy path must not short-circuit to ~0: at least a third of a real
        // verify. (Both run full Argon2, so they are normally near-identical.)
        assert!(
            dummy_dt * 3 >= real_dt,
            "dummy verify too fast ({dummy_dt:?}) vs real ({real_dt:?}) — timing oracle not closed"
        );
    }
}
