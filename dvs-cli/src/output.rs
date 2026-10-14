//! How results are printed: tables for people, `--json` and `--json-lines` for programs.

use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use tabled::Tabled;
use tabled::settings::{Alignment, Modify, location::ByColumnName, object::Rows};

use dvs::api::v1::Page;

/// `--json`: one document, a list as `{items, next_cursor}`. `--json-lines`: one item a
/// line, every page followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Text,
    Json,
    JsonLines,
}

impl Mode {
    pub fn is_text(self) -> bool {
        self == Mode::Text
    }
}

/// Print a table, numeric columns (`size`, `files`, …) aligned right.
pub fn table<T: Tabled>(rows: impl IntoIterator<Item = T>, right: &[&str]) {
    let mut table = tabled::Table::new(rows);
    for column in right {
        table.with(Modify::new(ByColumnName::new(*column)).with(Alignment::right()));
    }
    table.with(Modify::new(Rows::first()).with(Alignment::left()));
    println!("{table}");
}

/// Print `value` as JSON on one line.
pub fn json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

/// A list from the server. Text: `text` draws the first page, and a note says how to see
/// more. `--json`: that page with its cursor. `--json-lines`: every item of every page,
/// fetching the next with `next`.
pub fn list<T: Serialize>(
    mode: Mode,
    first: Page<T>,
    mut next: impl FnMut(&str) -> Result<Page<T>>,
    text: impl FnOnce(&[T]),
) -> Result<()> {
    match mode {
        Mode::Text => {
            text(&first.items);
            if first.next_cursor.is_some() {
                eprintln!("There are more: raise --limit, or use --json-lines to list them all.");
            }
        }
        Mode::Json => json(&json!({ "items": first.items, "next_cursor": first.next_cursor }))?,
        Mode::JsonLines => {
            let mut page = first;
            loop {
                for item in &page.items {
                    json(item)?;
                }
                match page.next_cursor.as_deref() {
                    Some(cursor) => page = next(cursor)?,
                    None => break,
                }
            }
        }
    }
    Ok(())
}

/// Every item of a list, following its pages, for commands that always show it all.
pub fn all<T>(first: Page<T>, mut next: impl FnMut(&str) -> Result<Page<T>>) -> Result<Vec<T>> {
    let mut items = first.items;
    let mut cursor = first.next_cursor;
    while let Some(c) = cursor {
        let page = next(&c)?;
        items.extend(page.items);
        cursor = page.next_cursor;
    }
    Ok(items)
}

/// An optional value for a table cell.
pub fn or_blank(value: Option<&str>) -> String {
    value.unwrap_or("").to_string()
}
