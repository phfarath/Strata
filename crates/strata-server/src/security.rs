use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use sha2::{Digest, Sha256};
use strata_core::errors::StrataError;
use uuid::Uuid;

use crate::models::Claims;

/// Hash a plaintext password using Argon2id with random salt.
pub fn hash_password(password: &str) -> Result<String, StrataError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| StrataError::Validation(format!("Password hashing failed: {e}")))
}

/// Verify a plaintext password against a stored Argon2id hash.
pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, StrataError> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|e| StrataError::Validation(format!("Invalid password hash format: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Generate a cryptographically secure random API key.
/// Returns: (full_key, key_prefix, key_hash_hex)
pub fn generate_api_key() -> (String, String, String) {
    let mut rng = rand::thread_rng();
    let random_bytes: [u8; 24] = rng.gen();
    let hex_part = hex::encode(random_bytes);
    let full_key = format!("strata_live_{hex_part}");

    let prefix = full_key.chars().take(16).collect::<String>();
    let hash = hash_api_key(&full_key);

    (full_key, prefix, hash)
}

/// Compute SHA-256 hash of an API key for constant-time lookups.
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.trim().as_bytes());
    hex::encode(hasher.finalize())
}

/// Create a signed JWT token for user session authentication.
pub fn create_jwt(
    user_id: &Uuid,
    email: &str,
    secret: &str,
    duration_secs: u64,
) -> Result<String, StrataError> {
    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        email: email.to_string(),
        iat: now,
        exp: now + (duration_secs as usize),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| StrataError::Validation(format!("Failed to create JWT token: {e}")))
}

/// Verify and decode a JWT token string.
pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims, StrataError> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| StrataError::Validation(format!("Invalid or expired JWT token: {e}")))?;

    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hashing_and_verification() {
        let password = "super-secret-password-123";
        let hash = hash_password(password).expect("Hashing failed");
        assert!(verify_password(password, &hash).unwrap());
        assert!(!verify_password("wrong-password", &hash).unwrap());
    }

    #[test]
    fn test_api_key_generation_and_hashing() {
        let (key, prefix, hash) = generate_api_key();
        assert!(key.starts_with("strata_live_"));
        assert!(prefix.starts_with("strata_live_"));
        assert_eq!(hash, hash_api_key(&key));
    }

    #[test]
    fn test_jwt_token_lifecycle() {
        let user_id = Uuid::new_v4();
        let email = "user@strata.dev";
        let secret = "jwt-secret-key-987654";

        let token = create_jwt(&user_id, email, secret, 3600).expect("JWT creation failed");
        let claims = verify_jwt(&token, secret).expect("JWT verification failed");

        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.email, email);
    }
}
