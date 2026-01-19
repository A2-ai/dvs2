//! Authentication and authorization.

use crate::ServerError;

/// Authentication configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    /// Whether authentication is enabled.
    pub enabled: bool,
    /// API keys for authentication.
    pub api_keys: Vec<ApiKey>,
    /// JWT secret for token-based auth.
    pub jwt_secret: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_keys: vec![],
            jwt_secret: None,
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

/// Permission types.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum Permission {
    /// Can read/download files.
    Read,
    /// Can upload files.
    Write,
    /// Can delete files.
    Delete,
    /// Full admin access.
    Admin,
}

/// Authenticated user context.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// User or key identifier.
    pub identity: String,
    /// Permissions for this user.
    pub permissions: Vec<Permission>,
}

/// Validate an API key and return the auth context.
pub fn validate_api_key(_config: &AuthConfig, _key: &str) -> Result<AuthContext, ServerError> {
    todo!("Validate API key")
}

/// Validate a JWT token and return the auth context.
pub fn validate_jwt(_config: &AuthConfig, _token: &str) -> Result<AuthContext, ServerError> {
    todo!("Validate JWT token")
}

/// Check if an auth context has a specific permission.
pub fn has_permission(_ctx: &AuthContext, _permission: Permission) -> bool {
    todo!("Check permission")
}

/// Middleware for extracting authentication from request.
pub async fn auth_middleware(
    _config: &AuthConfig,
    _headers: &axum::http::HeaderMap,
) -> Result<Option<AuthContext>, ServerError> {
    todo!("Extract auth from request")
}
