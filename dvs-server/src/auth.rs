//! Authentication and authorization.
//!
//! Provides API key-based authentication for the DVS server.
//!
//! ## Authentication Flow
//!
//! 1. Client sends `Authorization: Bearer <api_key>` header
//! 2. Server validates the key against configured keys
//! 3. If valid, request proceeds with the key's permissions
//! 4. If invalid or missing (when auth enabled), returns 401 Unauthorized
//!
//! ## Configuration
//!
//! ```yaml
//! auth:
//!   enabled: true
//!   api_keys:
//!     - key: "secret-key-1"
//!       name: "CI Pipeline"
//!       permissions: [Read, Write]
//!     - key: "admin-key"
//!       name: "Admin"
//!       permissions: [Admin]
//! ```

use crate::ServerError;

/// Authentication configuration.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    /// Whether authentication is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// API keys for authentication.
    #[serde(default)]
    pub api_keys: Vec<ApiKey>,
}

impl AuthConfig {
    /// Create an auth config with authentication disabled.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Create an auth config with authentication enabled and the given keys.
    pub fn with_keys(keys: Vec<ApiKey>) -> Self {
        Self {
            enabled: true,
            api_keys: keys,
        }
    }
}

/// An API key with associated permissions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiKey {
    /// The API key value.
    pub key: String,
    /// Human-readable name.
    pub name: String,
    /// Permissions granted to this key.
    pub permissions: Vec<Permission>,
}

impl ApiKey {
    /// Create a new API key.
    pub fn new(key: impl Into<String>, name: impl Into<String>, permissions: Vec<Permission>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            permissions,
        }
    }

    /// Create a read-only API key.
    pub fn read_only(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name, vec![Permission::Read])
    }

    /// Create a read-write API key.
    pub fn read_write(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name, vec![Permission::Read, Permission::Write])
    }

    /// Create an admin API key with all permissions.
    pub fn admin(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name, vec![Permission::Admin])
    }
}

/// Permission types.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum Permission {
    /// Can read/download objects.
    Read,
    /// Can upload objects.
    Write,
    /// Can delete objects.
    Delete,
    /// Full admin access (implies all other permissions).
    Admin,
}

/// Authenticated user context.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// User or key identifier (the key name).
    pub identity: String,
    /// Permissions for this user.
    pub permissions: Vec<Permission>,
}

impl AuthContext {
    /// Create an anonymous context (no authentication).
    pub fn anonymous() -> Self {
        Self {
            identity: "anonymous".to_string(),
            permissions: vec![Permission::Read, Permission::Write],
        }
    }

    /// Check if this context has a specific permission.
    pub fn has_permission(&self, permission: Permission) -> bool {
        // Admin has all permissions
        if self.permissions.contains(&Permission::Admin) {
            return true;
        }
        self.permissions.contains(&permission)
    }

    /// Check if this context can read objects.
    pub fn can_read(&self) -> bool {
        self.has_permission(Permission::Read)
    }

    /// Check if this context can write objects.
    pub fn can_write(&self) -> bool {
        self.has_permission(Permission::Write)
    }

    /// Check if this context can delete objects.
    pub fn can_delete(&self) -> bool {
        self.has_permission(Permission::Delete)
    }
}

/// Validate an API key and return the auth context.
///
/// Returns `Ok(AuthContext)` if the key is valid, or `Err(AuthError)` if not.
pub fn validate_api_key(config: &AuthConfig, key: &str) -> Result<AuthContext, ServerError> {
    for api_key in &config.api_keys {
        if api_key.key == key {
            return Ok(AuthContext {
                identity: api_key.name.clone(),
                permissions: api_key.permissions.clone(),
            });
        }
    }
    Err(ServerError::AuthError("invalid API key".to_string()))
}

/// Extract authentication from request headers.
///
/// Looks for `Authorization: Bearer <key>` header.
///
/// Returns:
/// - `Ok(Some(ctx))` if valid auth found
/// - `Ok(None)` if no auth header present
/// - `Err(_)` if auth header present but invalid
pub fn extract_auth(
    config: &AuthConfig,
    headers: &axum::http::HeaderMap,
) -> Result<Option<AuthContext>, ServerError> {
    let auth_header = match headers.get(axum::http::header::AUTHORIZATION) {
        Some(h) => h,
        None => return Ok(None),
    };

    let auth_str = auth_header
        .to_str()
        .map_err(|_| ServerError::AuthError("invalid authorization header".to_string()))?;

    // Check for Bearer token
    if let Some(key) = auth_str.strip_prefix("Bearer ") {
        let ctx = validate_api_key(config, key.trim())?;
        return Ok(Some(ctx));
    }

    Err(ServerError::AuthError("unsupported authorization scheme".to_string()))
}

/// Require authentication for a request.
///
/// If auth is enabled and no valid auth context provided, returns an error.
/// If auth is disabled, returns an anonymous context.
pub fn require_auth(
    config: &AuthConfig,
    headers: &axum::http::HeaderMap,
) -> Result<AuthContext, ServerError> {
    if !config.enabled {
        return Ok(AuthContext::anonymous());
    }

    match extract_auth(config, headers)? {
        Some(ctx) => Ok(ctx),
        None => Err(ServerError::AuthError("authentication required".to_string())),
    }
}

/// Require a specific permission for a request.
///
/// First authenticates the request, then checks for the required permission.
pub fn require_permission(
    config: &AuthConfig,
    headers: &axum::http::HeaderMap,
    permission: Permission,
) -> Result<AuthContext, ServerError> {
    let ctx = require_auth(config, headers)?;

    if ctx.has_permission(permission) {
        Ok(ctx)
    } else {
        Err(ServerError::NotAuthorized(format!(
            "permission denied: {:?} required",
            permission
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn test_auth_config_default() {
        let config = AuthConfig::default();
        assert!(!config.enabled);
        assert!(config.api_keys.is_empty());
    }

    #[test]
    fn test_api_key_constructors() {
        let read_only = ApiKey::read_only("key1", "Read Only");
        assert_eq!(read_only.permissions, vec![Permission::Read]);

        let read_write = ApiKey::read_write("key2", "Read Write");
        assert_eq!(read_write.permissions, vec![Permission::Read, Permission::Write]);

        let admin = ApiKey::admin("key3", "Admin");
        assert_eq!(admin.permissions, vec![Permission::Admin]);
    }

    #[test]
    fn test_auth_context_permissions() {
        let ctx = AuthContext {
            identity: "test".to_string(),
            permissions: vec![Permission::Read, Permission::Write],
        };
        assert!(ctx.can_read());
        assert!(ctx.can_write());
        assert!(!ctx.can_delete());

        // Admin has all permissions
        let admin_ctx = AuthContext {
            identity: "admin".to_string(),
            permissions: vec![Permission::Admin],
        };
        assert!(admin_ctx.can_read());
        assert!(admin_ctx.can_write());
        assert!(admin_ctx.can_delete());
    }

    #[test]
    fn test_validate_api_key() {
        let config = AuthConfig::with_keys(vec![
            ApiKey::read_only("valid-key", "Test Key"),
        ]);

        let ctx = validate_api_key(&config, "valid-key").unwrap();
        assert_eq!(ctx.identity, "Test Key");
        assert!(ctx.can_read());
        assert!(!ctx.can_write());

        let err = validate_api_key(&config, "invalid-key");
        assert!(err.is_err());
    }

    #[test]
    fn test_extract_auth() {
        let config = AuthConfig::with_keys(vec![
            ApiKey::read_write("secret", "Test"),
        ]);

        // No auth header
        let headers = HeaderMap::new();
        let result = extract_auth(&config, &headers).unwrap();
        assert!(result.is_none());

        // Valid Bearer token
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );
        let ctx = extract_auth(&config, &headers).unwrap().unwrap();
        assert_eq!(ctx.identity, "Test");

        // Invalid Bearer token
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer wrong".parse().unwrap(),
        );
        let err = extract_auth(&config, &headers);
        assert!(err.is_err());
    }

    #[test]
    fn test_require_auth_disabled() {
        let config = AuthConfig::disabled();
        let headers = HeaderMap::new();

        let ctx = require_auth(&config, &headers).unwrap();
        assert_eq!(ctx.identity, "anonymous");
    }

    #[test]
    fn test_require_auth_enabled() {
        let config = AuthConfig::with_keys(vec![
            ApiKey::read_only("key", "User"),
        ]);

        // No auth header when required
        let headers = HeaderMap::new();
        let err = require_auth(&config, &headers);
        assert!(err.is_err());

        // Valid auth header
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer key".parse().unwrap(),
        );
        let ctx = require_auth(&config, &headers).unwrap();
        assert_eq!(ctx.identity, "User");
    }

    #[test]
    fn test_require_permission() {
        let config = AuthConfig::with_keys(vec![
            ApiKey::read_only("reader", "Reader"),
        ]);

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer reader".parse().unwrap(),
        );

        // Has Read permission
        let ctx = require_permission(&config, &headers, Permission::Read).unwrap();
        assert!(ctx.can_read());

        // Lacks Write permission
        let err = require_permission(&config, &headers, Permission::Write);
        assert!(err.is_err());
    }
}
