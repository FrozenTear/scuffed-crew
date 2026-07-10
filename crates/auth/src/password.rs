//! Argon2id password hashing for local (username/password) accounts.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify() {
        let h = hash_password("correct-horse-battery").unwrap();
        assert!(verify_password("correct-horse-battery", &h).unwrap());
        assert!(!verify_password("wrong-password-xx", &h).unwrap());
    }
}
