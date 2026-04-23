use std::collections::BTreeMap;
use std::path::Path;

use dvs::{AddError, AddSuccess, GetError, GetSuccess, StatusError, StatusSuccess};

/// Print add results in errors-first order. Returns true when any errors exist.
pub fn print_add(
    ok: &[AddSuccess],
    err: &[AddError],
    dry_run: bool,
    format_size: impl Fn(u64) -> String,
) -> bool {
    if !err.is_empty() {
        print_add_failures(err);
        eprintln!();
    }

    for success in ok {
        let stored_info = match success.stored_size {
            Some(stored_size) => format!(" --> saved [{}]", format_size(stored_size)),
            None => String::new(),
        };

        match success.outcome {
            dvs::Outcome::Copied => {
                let verb = if dry_run { "To add" } else { "Added" };
                println!(
                    "{verb}: {} [{}]{stored_info} as {}",
                    success.path.display(),
                    format_size(success.size),
                    success.hash,
                );
            }
            dvs::Outcome::Present => {}
        }
    }

    !err.is_empty()
}

fn print_add_failures(err: &[AddError]) {
    let mut groups: BTreeMap<&'static str, Vec<&AddError>> = BTreeMap::new();
    for error in err {
        groups.entry(error.kind()).or_default().push(error);
    }

    eprintln!("Failed ({}):", err.len());
    for (kind, items) in &groups {
        eprintln!("  {kind} ({}):", items.len());
        let mut lines: Vec<String> = items
            .iter()
            .filter_map(|error| match error {
                AddError::GlobFailure { pattern, reason } => {
                    Some(format!("glob {pattern:?}: {reason}"))
                }
                _ => error.path().map(|path| path.display().to_string()),
            })
            .collect();
        lines.sort();
        for line in lines {
            eprintln!("    {line}");
        }
    }
}

#[allow(dead_code)]
pub fn print_get(
    ok: &[GetSuccess],
    err: &[GetError],
    format_size: impl Fn(u64) -> String,
) -> bool {
    if !err.is_empty() {
        print_get_failures(err);
        eprintln!();
    }

    let mut total_files = 0u64;
    let mut total_bytes = 0u64;
    for success in ok {
        if success.outcome == dvs::Outcome::Copied {
            println!("{} [{}]", success.path.display(), format_size(success.size));
            total_files += 1;
            total_bytes += success.size;
        }
    }

    if total_files > 0 {
        println!("Total: {} files, {}", total_files, format_size(total_bytes));
    }

    !err.is_empty()
}

#[allow(dead_code)]
fn print_get_failures(err: &[GetError]) {
    let mut groups: BTreeMap<&'static str, Vec<&GetError>> = BTreeMap::new();
    for error in err {
        groups.entry(error.kind()).or_default().push(error);
    }

    eprintln!("Failed ({}):", err.len());
    for (kind, items) in &groups {
        eprintln!("  {kind} ({}):", items.len());
        let mut lines: Vec<String> = items
            .iter()
            .filter_map(|error| match error {
                GetError::GlobFailure { pattern, reason } => {
                    Some(format!("glob {pattern:?}: {reason}"))
                }
                GetError::HashMismatch {
                    path,
                    expected,
                    got,
                } => Some(format!(
                    "{} (expected {}, got {})",
                    path.display(),
                    expected,
                    got
                )),
                _ => error.path().map(|path| path.display().to_string()),
            })
            .collect();
        lines.sort();
        for line in lines {
            eprintln!("    {line}");
        }
    }
}

#[allow(dead_code)]
pub fn print_status_failures(err: &[StatusError]) {
    if err.is_empty() {
        return;
    }

    let mut groups: BTreeMap<&'static str, Vec<&StatusError>> = BTreeMap::new();
    for error in err {
        groups.entry(error.kind()).or_default().push(error);
    }

    eprintln!("Failed ({}):", err.len());
    for (kind, items) in &groups {
        eprintln!("  {kind} ({}):", items.len());
        for error in items {
            let line = match error {
                StatusError::RelativePath {
                    metadata_path,
                    reason,
                } => format!("{}: {reason}", metadata_path.display()),
                StatusError::MetadataRead { path, reason } => {
                    format!("{}: {reason}", path.display())
                }
                StatusError::HashFailure { path, reason } => {
                    format!("{}: {reason}", path.display())
                }
            };
            eprintln!("    {line}");
        }
    }
    eprintln!();
}

#[allow(dead_code)]
fn _force_use_status_success(_: &StatusSuccess) -> &Path {
    Path::new("")
}
