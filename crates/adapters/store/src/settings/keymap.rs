//! The user's key bindings: the `keybindings` table, and `keybindings.json`, the file the user
//! edits, in VS Code's format: `[{ "key": "mod+k", "command": "quickie.palette.goto", "when":
//! "pageFocused" }]`. Both hand over the entries as text; the keymap use case parses them, so an
//! entry that does not parse is reported there, one entry at a time, not lost here. The keymap
//! editor writes the file back, one entry per line; a field that was not a string comes back as
//! the empty text it read as.

use std::io::Write;
use std::path::{Path, PathBuf};

use quickie_application::{KeybindingText, KeymapFile, KeymapRepo, StoreError};
use rusqlite::Connection;
use serde_json::Value;

use crate::failed;

/// The user's entries in the table, in the order they were written.
pub(crate) fn load(db: &Connection) -> Result<Vec<KeybindingText>, StoreError> {
    db.prepare("SELECT key, command, when_clause FROM keybindings ORDER BY position")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(KeybindingText {
                    key: row.get(0)?,
                    command: row.get(1)?,
                    when: row.get(2)?,
                })
            })?
            .collect()
        })
        .map_err(failed)
}

/// `keybindings.json` in the profile. A missing file is no entries; a file that is not a JSON
/// list is an error, and the keymap keeps its defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeybindingsFile {
    path: PathBuf,
}

impl KeybindingsFile {
    /// The file at `path`; nothing is read until the keymap loads.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Creates the file with an empty list when it is missing, so an editor can open it.
    ///
    /// # Errors
    ///
    /// The file could not be written.
    pub fn create_if_missing(&self) -> Result<(), StoreError> {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
        {
            Ok(mut file) => file
                .write_all(b"[\n]\n")
                .map_err(|error| self.error(&error)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(self.error(&error)),
        }
    }

    fn error(&self, error: &dyn std::fmt::Display) -> StoreError {
        StoreError(format!("{}: {error}", self.path.display()))
    }
}

impl KeymapRepo for KeybindingsFile {
    fn load_keybindings(&self) -> Result<Vec<KeybindingText>, StoreError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(self.error(&error)),
        };
        parse(&text).map_err(|problem| self.error(&problem))
    }
}

impl KeymapFile for KeybindingsFile {
    /// Writes the entries next to the file and renames them over it, so a crash halfway leaves
    /// the old file whole.
    fn save_keybindings(&self, entries: &[KeybindingText]) -> Result<(), StoreError> {
        let mut staged = self.path.clone().into_os_string();
        staged.push(".saving");
        let staged = PathBuf::from(staged);
        std::fs::write(&staged, write(entries))
            .and_then(|()| std::fs::rename(&staged, &self.path))
            .map_err(|error| self.error(&error))
    }
}

/// `entries` as a `keybindings.json` text: one entry per line, a clause only where there is
/// one.
fn write(entries: &[KeybindingText]) -> String {
    let lines: Vec<String> = entries
        .iter()
        .map(|entry| {
            let mut fields = vec![
                format!("\"key\": {}", Value::from(entry.key.as_str())),
                format!("\"command\": {}", Value::from(entry.command.as_str())),
            ];
            if let Some(when) = &entry.when {
                fields.push(format!("\"when\": {}", Value::from(when.as_str())));
            }
            format!("  {{ {} }}", fields.join(", "))
        })
        .collect();
    if lines.is_empty() {
        "[\n]\n".to_owned()
    } else {
        format!("[\n{}\n]\n", lines.join(",\n"))
    }
}

/// The entries of a `keybindings.json` text. A field that is missing or not a string reads as
/// empty, so the keymap rejects that entry alone and says which field is wrong.
fn parse(text: &str) -> Result<Vec<KeybindingText>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let Value::Array(entries) = value else {
        return Err("expected a list of entries, [ … ]".to_owned());
    };
    Ok(entries
        .iter()
        .map(|entry| {
            let field = |name: &str| entry.get(name).and_then(Value::as_str).map(str::to_owned);
            KeybindingText {
                key: field("key").unwrap_or_default(),
                command: field("command").unwrap_or_default(),
                when: field("when"),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use quickie_application::{KeybindingText, KeymapFile, KeymapRepo};

    use super::{KeybindingsFile, parse, write};

    fn entry(key: &str, command: &str, when: Option<&str>) -> KeybindingText {
        KeybindingText {
            key: key.to_owned(),
            command: command.to_owned(),
            when: when.map(str::to_owned),
        }
    }

    #[test]
    fn reads_entries_in_order_with_their_clauses() {
        let text = r#"[
            { "key": "mod+k", "command": "quickie.palette.goto" },
            { "key": "f", "command": "-quickie.hints.toggle", "when": "pageFocused" }
        ]"#;

        assert_eq!(
            parse(text).unwrap(),
            [
                entry("mod+k", "quickie.palette.goto", None),
                entry("f", "-quickie.hints.toggle", Some("pageFocused")),
            ]
        );
    }

    #[test]
    fn a_bad_entry_reads_as_empty_fields_for_the_keymap_to_reject() {
        let text = r#"[{ "command": "quickie.tab.new" }, 7, { "key": 3, "command": "x" }]"#;

        assert_eq!(
            parse(text).unwrap(),
            [
                entry("", "quickie.tab.new", None),
                entry("", "", None),
                entry("", "x", None),
            ]
        );
    }

    #[test]
    fn a_file_that_is_no_list_is_an_error() {
        assert!(parse("{}").unwrap_err().contains("list of entries"));
        assert!(parse("[ { ").is_err());
    }

    #[test]
    fn a_missing_file_is_no_entries_until_it_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeybindingsFile::new(dir.path().join("keybindings.json"));

        assert_eq!(file.load_keybindings().unwrap(), []);

        file.create_if_missing().unwrap();
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "[\n]\n");
        assert_eq!(file.load_keybindings().unwrap(), []);
    }

    #[test]
    fn creating_keeps_what_the_user_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeybindingsFile::new(dir.path().join("keybindings.json"));
        let written = r#"[{ "key": "mod+k", "command": "quickie.tab.new" }]"#;
        std::fs::write(file.path(), written).unwrap();

        file.create_if_missing().unwrap();

        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), written);
        assert_eq!(
            file.load_keybindings().unwrap(),
            [entry("mod+k", "quickie.tab.new", None)]
        );
    }

    #[test]
    fn saved_entries_read_back_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeybindingsFile::new(dir.path().join("keybindings.json"));
        let entries = [
            entry("cmd+r", "-quickie.tab.reload", None),
            entry("g", "quickie.hints.toggle", Some("pageFocused && \"x\"")),
        ];

        file.save_keybindings(&entries).unwrap();

        assert_eq!(file.load_keybindings().unwrap(), entries);
        assert!(!dir.path().join("keybindings.json.saving").exists());
    }

    #[test]
    fn the_file_lists_one_entry_per_line() {
        assert_eq!(write(&[]), "[\n]\n");
        assert_eq!(
            write(&[
                entry("cmd+e", "quickie.page.find", None),
                entry("f", "-quickie.hints.toggle", Some("pageFocused")),
            ]),
            "[\n  { \"key\": \"cmd+e\", \"command\": \"quickie.page.find\" },\n  { \"key\": \
             \"f\", \"command\": \"-quickie.hints.toggle\", \"when\": \"pageFocused\" }\n]\n"
        );
    }

    #[test]
    fn a_failed_save_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeybindingsFile::new(dir.path().join("missing").join("keybindings.json"));

        let error = file.save_keybindings(&[]).unwrap_err();

        assert!(error.0.contains("keybindings.json"), "{error}");
    }

    #[test]
    fn an_unreadable_file_names_itself() {
        let dir = tempfile::tempdir().unwrap();
        let file = KeybindingsFile::new(dir.path().to_path_buf());

        let error = file.load_keybindings().unwrap_err();

        assert!(
            error.0.contains(&dir.path().display().to_string()),
            "{error}"
        );
    }
}
