//! Packing a generated project into a zip.
//!
//! The whole archive is built in memory rather than streamed. That is a
//! deliberate reversal of what the roadmap issue asked for: a generated
//! project is six text files and around 16 KB before compression, so a
//! streaming writer would add a self-referential body type and a failure mode
//! halfway through a response, to avoid holding a few kilobytes. Streaming
//! becomes the right call if a kit ever carries binary assets or a vendored
//! tree — the cap below is what would catch that drift.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use super::StarterKit;

/// Refuse to build an archive larger than this. Nothing the generator emits
/// comes close; if it ever does, that is a bug worth failing on rather than a
/// payload worth serving.
const MAX_ARCHIVE_BYTES: usize = 2 * 1024 * 1024;

/// A fixed timestamp for every entry.
///
/// Zip stores a modification time per file, so using "now" would make two
/// downloads of the same configuration differ byte for byte and defeat the
/// determinism the rest of the generator is careful about. The date is the
/// project's own epoch and means nothing else.
const FIXED_MTIME: (u16, u8, u8, u8, u8, u8) = (2026, 1, 1, 0, 0, 0);

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("failed to build the archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("failed to write into the archive: {0}")]
    Io(#[from] std::io::Error),
    #[error("the generated archive was unexpectedly large ({0} bytes)")]
    TooLarge(usize),
}

impl StarterKit {
    /// Pack this project into a zip, every file under a single directory.
    pub fn to_zip(&self) -> Result<Vec<u8>, ArchiveError> {
        let mtime = DateTime::from_date_and_time(
            FIXED_MTIME.0,
            FIXED_MTIME.1,
            FIXED_MTIME.2,
            FIXED_MTIME.3,
            FIXED_MTIME.4,
            FIXED_MTIME.5,
        )
        .expect("the fixed timestamp is a valid date");

        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(mtime)
            // Text files, readable by the owner and everyone else. Without this
            // the mode is whatever the platform default is, which on some
            // unzip implementations comes out as 000.
            .unix_permissions(0o644);

        let root = self.archive_root();
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));

        // Emitted in the order `generate` produced them, which is itself
        // deterministic, so the archive is byte-identical run to run.
        for file in &self.files {
            writer.start_file(format!("{root}/{}", file.path), options)?;
            writer.write_all(file.contents.as_bytes())?;
        }

        let bytes = writer.finish()?.into_inner();
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err(ArchiveError::TooLarge(bytes.len()));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo_config::DemoConfig;
    use crate::scenario::{ControlValue, ScenarioRegistry};
    use std::io::Read;

    fn registry() -> ScenarioRegistry {
        ScenarioRegistry::with_providers(vec!["github".to_string(), "google".to_string()])
    }

    fn base_kit() -> StarterKit {
        let registry = registry();
        StarterKit::generate(&DemoConfig::defaults_for(&registry), &registry)
    }

    fn full_kit() -> StarterKit {
        let registry = registry();
        let mut config = DemoConfig::defaults_for(&registry);
        config.set("passkeys", ControlValue::Toggle { enabled: true });
        config.set("totp", ControlValue::Toggle { enabled: true });
        config.set(
            "oauth",
            ControlValue::SelectMany {
                selected: vec!["github".to_string()],
            },
        );
        StarterKit::generate(&config, &registry)
    }

    /// Read the archive back, because asserting on the bytes we just wrote
    /// would only prove the writer agrees with itself.
    fn entries(bytes: &[u8]) -> Vec<(String, String)> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).expect("a valid zip");
        (0..archive.len())
            .map(|i| {
                let mut entry = archive.by_index(i).expect("entry");
                let name = entry.name().to_string();
                let mut contents = String::new();
                entry.read_to_string(&mut contents).expect("utf-8 contents");
                (name, contents)
            })
            .collect()
    }

    #[test]
    fn the_archive_contains_every_generated_file_under_one_directory() {
        let kit = full_kit();
        let entries = entries(&kit.to_zip().unwrap());
        let root = kit.archive_root();

        assert_eq!(entries.len(), kit.files.len());
        for file in &kit.files {
            let expected = format!("{root}/{}", file.path);
            let found = entries
                .iter()
                .find(|(name, _)| *name == expected)
                .unwrap_or_else(|| panic!("{expected} missing from {entries:?}"));
            assert_eq!(found.1, file.contents, "{expected} was altered in packing");
        }
    }

    /// The acceptance criterion. A generator that shipped a real secret would
    /// leak whatever the deployment happens to hold.
    #[test]
    fn the_archive_never_carries_a_filled_in_secret() {
        let entries = entries(&full_kit().to_zip().unwrap());

        assert!(
            !entries.iter().any(|(name, _)| name.ends_with("/.env")),
            "a .env file is in the archive"
        );

        let env = entries
            .iter()
            .find(|(name, _)| name.ends_with("/.env.example"))
            .expect("the example is shipped");

        for line in env.1.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            if name.contains("SECRET") || name.contains("CLIENT_ID") {
                assert!(value.is_empty(), "{name} is shipped with a value: {line:?}");
            }
        }
    }

    #[test]
    fn the_same_configuration_packs_to_identical_bytes() {
        assert_eq!(
            full_kit().to_zip().unwrap(),
            full_kit().to_zip().unwrap(),
            "the archive is not reproducible"
        );
    }

    /// Two selections that differ must not arrive under one filename, or the
    /// second download silently overwrites the first in a downloads folder.
    #[test]
    fn different_selections_get_different_names() {
        let registry = registry();
        let with = |providers: Vec<&str>| {
            let mut config = DemoConfig::defaults_for(&registry);
            config.set(
                "oauth",
                ControlValue::SelectMany {
                    selected: providers.iter().map(|p| p.to_string()).collect(),
                },
            );
            StarterKit::generate(&config, &registry).archive_name()
        };

        assert_ne!(with(vec!["github"]), with(vec!["google"]));
        assert_ne!(with(vec!["github"]), with(vec!["github", "google"]));
        assert_eq!(base_kit().archive_name(), "authkestra-starter-base.zip");
        assert_eq!(
            with(vec!["github", "google"]),
            "authkestra-starter-oauth-github-google.zip"
        );
    }

    /// A name that needs quoting in a `Content-Disposition` header, or escaping
    /// in a shell, is a name worth rejecting at the source.
    #[test]
    fn archive_names_need_no_quoting() {
        for kit in [base_kit(), full_kit()] {
            let name = kit.archive_name();
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
                "{name} contains a character that would need escaping"
            );
        }
    }

    #[test]
    fn the_archive_is_small_enough_to_hold_in_memory() {
        let bytes = full_kit().to_zip().unwrap();
        assert!(
            bytes.len() < 64 * 1024,
            "the archive grew to {} bytes — reconsider streaming",
            bytes.len()
        );
    }
}
