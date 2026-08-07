use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use url::Url;

use crate::audit::AuditEntry;
use crate::{Backend, Compression, Hashes, RetrieveRequest, StoreRequest};

#[derive(Serialize, Deserialize)]
pub struct InitPayload {
    name: String,
    group: String,
    compression: Compression,
}
