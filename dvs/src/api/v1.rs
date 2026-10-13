//! `/api/v1` request and response bodies, and `/api/server` discovery.
//!
//! Doc comments become the OpenAPI descriptions: change them knowing the server's
//! checked-in `openapi.json` changes too.

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// A group's role on a project. A user's role is the highest among their groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// See, browse and download
    Read,
    /// And upload
    Write,
    /// And change access, lock, settings; rename or delete while empty
    Admin,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Read, Role::Write, Role::Admin];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Read => "read",
            Role::Write => "write",
            Role::Admin => "admin",
        }
    }

    pub fn parse(value: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.as_str() == value)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A page of a list: `next_cursor` is null on the last page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    /// Only with `count=true`: how many items the whole list has
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
}

/// The error body, for the OpenAPI document only. Other fields depend on the code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ErrorDetail {
    /// Stable, machine-readable: clients branch on this, never on the status alone
    pub code: String,
    /// For people
    pub message: String,
    /// Finds the server log line
    pub request_id: String,
}

/// One grant as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrantView {
    pub group_id: Uuid,
    pub group: String,
    pub role: Role,
    /// The group was deleted from the directory, so this grant no longer gives anyone
    /// access. A group created again under the same name is a different group: remove
    /// this grant, or grant the new one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProjectView {
    pub name: String,
    /// `zstd` or `none`; fixed at creation
    pub compression: String,
    /// Uploads are refused while locked; downloads still work
    pub locked: bool,
    /// The ETag of the settings and the access list
    pub version: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: Timestamp,
    pub created_by: String,
    pub your_role: Role,
    pub grants: Vec<GrantView>,
    /// Turns HEAD commits into links; contains `{commit}`
    pub repo_commit_url: Option<String>,
    /// On a single project, and on lists with `stats=true`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ProjectStats>,
}

/// A grant in a request: groups by name, roles validated here so a bad one is
/// `invalid_role` rather than a generic body error.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub group: String,
    /// `read`, `write` or `admin`
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct CreateProject {
    pub name: String,
    /// `zstd` (the default) or `none`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<String>,
    pub grants: Vec<GrantRequest>,
}

/// Present but null and absent are different for `repo_commit_url`: null clears it.
fn double_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PatchProject {
    /// Lock or unlock uploads
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    /// A URL containing `{commit}`, or null to clear it
    #[serde(
        default,
        deserialize_with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub repo_commit_url: Option<Option<String>>,
    /// A new name; only while the project is empty
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AccessView {
    pub grants: Vec<GrantView>,
    /// Null when a confirmed change removed your own access
    pub your_role: Option<Role>,
    /// The project version; send it back as `If-Match`
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PutAccess {
    /// The complete list; groups not in it lose access
    pub grants: Vec<GrantRequest>,
    /// Needed when the change removes your own admin role
    #[serde(default)]
    pub confirm_self_demotion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PutGrant {
    /// `read`, `write` or `admin`
    pub role: String,
    #[serde(default)]
    pub confirm_self_demotion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GroupView {
    pub id: Uuid,
    pub name: String,
    /// Whether you belong to it
    pub is_member: bool,
    /// The configured global admin group: admin on every project
    pub is_admin_group: bool,
    /// How many members the directory has; member lists are not exposed.
    pub known_members: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MeView {
    pub sub: String,
    pub username: Option<String>,
    pub email: Option<String>,
    /// Your directory groups, by name
    pub groups: Vec<String>,
    pub global_admin: bool,
    /// The token this request used
    pub token: Option<MeToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MeToken {
    pub id: Uuid,
    /// `login`: issued by `dvs login`. Scoped tokens come later.
    pub source: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: Timestamp,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub expires_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ServerInfo {
    pub server_version: String,
    pub api_versions: Vec<u32>,
    /// Older clients must upgrade
    pub min_client_version: String,
    /// What this server supports, for clients that adapt
    pub features: Vec<String>,
    pub limits: ServerLimits,
    pub auth: ServerAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ServerLimits {
    /// Largest blob in bytes; 0 means no limit
    pub max_blob_size: u64,
    pub max_page_size: usize,
    pub max_body_bytes: usize,
    /// Entries per manifest call
    pub max_manifest_entries: usize,
    /// Longest path, in UTF-8 bytes
    pub max_path_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ServerAuth {
    pub device_flow: bool,
}

/// What a project holds, from maintained counters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProjectStats {
    /// Paths with at least one visible version
    pub paths: u64,
    /// Distinct (path, content) pairs
    pub versions: u64,
    /// Visible placements: every time content was placed at a path
    pub placements: u64,
    /// Size of the latest version of every path. Not disk usage.
    pub logical_bytes: u64,
    /// Stored size of the distinct contents those latest versions hold. Not disk usage.
    pub unique_stored_bytes: u64,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_upload_at: Option<Timestamp>,
    /// The latest upload, download or project event
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_activity_at: Option<Timestamp>,
}

/// `upload` (`dvs add`) or `download` (`dvs get`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum BatchKind {
    Upload,
    Download,
}

impl BatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BatchKind::Upload => "upload",
            BatchKind::Download => "download",
        }
    }
}

/// The client's checkout when the batch opened, read without running Git. These are
/// client-asserted aids for finding work, never provenance: a commit says where the
/// checkout was, not that it contains the file. Invalid values are dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GitContext {
    /// `branch`, `detached`, `unborn`, `no_repo` or `unavailable`
    pub state: String,
    /// Short branch name when `state` is `branch` or `unborn`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Full HEAD object id (40 or 64 hex) whenever HEAD points at a commit
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct OpenBatch {
    pub kind: BatchKind,
    /// The `dvs add` message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BatchView {
    pub id: Uuid,
    pub kind: BatchKind,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: Timestamp,
    /// After this the batch is closed; its pending placements are abandoned
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub expires_at: Timestamp,
}

/// One file being added: content `hash` of `size` bytes placed at `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    /// Relative to the repository root, `/`-separated, at most `max_path_bytes`
    pub path: String,
    /// blake3, 64 lowercase hex characters
    pub hash: String,
    /// Uncompressed, at most `max_blob_size` when that is not 0
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// At most `max_manifest_entries`; send more in further calls
    pub entries: Vec<ManifestEntry>,
}

/// Content the project already has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PresentContent {
    pub hash: String,
    pub size: u64,
    pub stored_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ManifestResult {
    /// Hashes the project has no bytes for: `PUT` each once
    pub missing: Vec<String>,
    /// Hashes it has; their placements are already visible
    pub present: Vec<PresentContent>,
    /// The project's compression, `zstd` or `none`
    pub compression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct StoredBlob {
    pub size: u64,
    pub stored_size: u64,
    /// How the server stored it, `zstd` or `none`
    pub compression: String,
}

/// Client-reported counts, labelled that way wherever they are shown.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(deny_unknown_fields)]
pub struct CompleteBatch {
    pub ok: u32,
    pub failed: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CompletedBatch {
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub completed_at: Timestamp,
    /// Placements whose bytes never arrived; they will never be published
    pub abandoned: u64,
}

/// One version of a path: a hash placed there, newest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VersionView {
    pub hash: String,
    pub size: u64,
    pub stored_size: u64,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub first_uploaded_at: Timestamp,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub last_uploaded_at: Timestamp,
    /// Times this hash was placed at this path
    pub placements: u64,
    /// Downloads of this hash requested for this path
    pub downloads: u64,
    pub yanked: bool,
    /// Other paths in the project holding the same content
    pub other_paths: u64,
}

/// RFC 8628 device authorization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DeviceAuthorization {
    pub device_code: String,
    /// What the person checks on the approval page
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    /// Seconds until the codes expire, approved or not
    pub expires_in: u64,
    /// Seconds to wait between polls
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DeviceTokenRequest {
    pub device_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DeviceToken {
    pub access_token: String,
    /// Always `Bearer`
    pub token_type: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub expires_at: Timestamp,
}

/// RFC 8628 error body of the device routes, which do not use the error envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DeviceError {
    /// `authorization_pending`, `slow_down`, `expired_token`, `access_denied`,
    /// `invalid_grant` or `too_many_pending`
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_project_keeps_null_apart_from_absent() {
        let absent: PatchProject = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.repo_commit_url, None);
        let null: PatchProject = serde_json::from_str(r#"{"repo_commit_url":null}"#).unwrap();
        assert_eq!(null.repo_commit_url, Some(None));

        let clear = PatchProject {
            repo_commit_url: Some(None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&clear).unwrap(),
            r#"{"repo_commit_url":null}"#
        );
        assert_eq!(
            serde_json::to_string(&PatchProject::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn roles_order_and_parse() {
        assert!(Role::Read < Role::Write && Role::Write < Role::Admin);
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Some(role));
            assert_eq!(serde_json::to_string(&role).unwrap(), format!("\"{role}\""));
        }
        assert_eq!(Role::parse("owner"), None);
    }
}
