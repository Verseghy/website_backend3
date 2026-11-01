//! ADR 0034, "What must be true in code and tests": every key value that a
//! committed configuration other than `config/production.yaml` holds, and
//! every key value under `tests/`, is one that production refuses at boot.
//!
//! Development, test and e2e may commit public throwaway keys (ADR 0034 §2
//! item 3), and production refuses each of them by value
//! (`Settings::refuse_published_keys`, §3). The refused list has to grow
//! with every new throwaway key; these tests catch a slice that commits a
//! key and forgets the refusal.
//!
//! What counts as a key value: any run of base64 characters in the files
//! that is canonical standard base64 (`=` padding) of at least 32 bytes, the
//! length of backend3's shortest key (the content MAC key, ADR 0023 §11),
//! with both upper- and lower-case letters. Production accepts nothing else
//! as a key, so nothing else could be a published key that it fails to
//! refuse. The case rule leaves out paths and hexadecimal digests, which
//! base64's alphabet also spells; base64 of random bytes or of a sentence
//! has both cases. The scan reads the text of the files, so it finds a key
//! wherever it is written: a value, a default, a comment or a test literal.
//!
//! "Refused" means refused by the published-key refusal itself, not merely
//! by the format checks: a key that production refuses only because it is
//! short would be published all the same once its kind allows that length.
//! A future kind of key joins [`listed_by_the_refusal`] in the slice that
//! adds it to the refusal.
//!
//! A developer's personal overrides, `config/local.yaml` and
//! `config/<env>.local.yaml`, are left out: `.gitignore` keeps them out of
//! the repository, so they publish nothing, and a developer may put a key of
//! their own there. The tests that load a configuration load the committed
//! file alone (`tests/support::load_committed_config`), because Loco would
//! merge such an override over it.
//!
//! The tests take every published value they need from the configuration
//! files and from the refusal's own constants, never from a literal, so this
//! file holds no key itself.

use std::{
    fs,
    path::{Path, PathBuf},
};

use backend3::{
    domain::{session_jwt::JwtSecret, totp_secret::PUBLISHED_LOCAL_KEY},
    settings::{Settings, SettingsError, check_session_jwt},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use loco_rs::environment::Environment;
use serde_json::Value;

use crate::support::{cli_jwt_secret, cli_totp_key, load_committed_config, repository_root};

/// The shortest decoded length of a backend3 key (ADR 0023 §11).
const SHORTEST_KEY_LEN: usize = 32;

/// The key values of `text`, with the 1-based number of the line each
/// starts on.
///
/// - `text`: the content of one file.
fn key_values(text: &str) -> Vec<(usize, String)> {
    let is_base64 =
        |character: char| character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=');
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for run in line.split(|character: char| !is_base64(character)) {
            if is_key_value(run) {
                found.push((index + 1, run.to_owned()));
            }
        }
    }
    found
}

/// Whether `run`, a maximal run of base64 characters, is a key value as the
/// module documentation defines it.
///
/// - `run`: the candidate.
fn is_key_value(run: &str) -> bool {
    run.len().is_multiple_of(4)
        && run.chars().any(|character| character.is_ascii_uppercase())
        && run.chars().any(|character| character.is_ascii_lowercase())
        && STANDARD
            .decode(run)
            .is_ok_and(|decoded| decoded.len() >= SHORTEST_KEY_LEN)
}

/// The `settings:` section of `config/test.yaml` with `totp_key` as its TOTP
/// encryption key, or `None` when that is not a usable TOTP key.
///
/// - `totp_key`: the configured value to put in.
fn settings_with_totp_key(totp_key: &str) -> Option<Settings> {
    let mut section: Value = load_committed_config(&Environment::Test)
        .settings
        .expect("settings");
    section["admin_auth"]["totp_encryption_key"] = totp_key.into();
    Settings::from_section(Some(&section)).ok()
}

/// Whether production's published-key refusal refuses `value` as one of the
/// keys it covers: as the session secret next to a TOTP key made now, or as
/// the TOTP encryption key next to a session secret made now.
///
/// - `value`: the key value, as the file holds it.
fn listed_by_the_refusal(value: &str) -> bool {
    let as_jwt_secret = JwtSecret::from_base64(value).is_ok_and(|secret| {
        let settings = settings_with_totp_key(&cli_totp_key()).expect("a key made now is usable");
        matches!(
            settings.refuse_published_keys(&Environment::Production, &secret),
            Err(SettingsError::PublishedJwtSecret { .. })
        )
    });
    let as_totp_key = settings_with_totp_key(value).is_some_and(|settings| {
        let secret =
            JwtSecret::from_base64(&cli_jwt_secret()).expect("a secret made now is usable");
        matches!(
            settings.refuse_published_keys(&Environment::Production, &secret),
            Err(SettingsError::PublishedTotpKey)
        )
    });
    as_jwt_secret || as_totp_key
}

/// Whether `name`, a file name in `config/`, is one of the personal
/// overrides that `.gitignore` keeps out of the repository: `local.yaml` and
/// `<anything>.local.yaml`, which loco-rs 1.2.0 merges over `<env>.yaml`.
///
/// - `name`: the file name, without its directory.
fn is_personal_override(name: &str) -> bool {
    name == "local.yaml" || name.ends_with(".local.yaml")
}

/// Every committed configuration other than `config/production.yaml`: each
/// YAML file in `config/` but that one and the personal overrides
/// ([`is_personal_override`]).
fn local_config_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(repository_root().join("config"))
        .expect("config/")
        .map(|entry| entry.expect("config/ entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml" || extension == "yml")
                && path.file_stem().is_some_and(|stem| stem != "production")
                && path
                    .file_name()
                    .is_some_and(|name| !is_personal_override(&name.to_string_lossy()))
        })
        .collect();
    files.sort();
    files
}

/// Every Rust file under `dir`, recursively.
///
/// - `dir`: the directory to walk.
/// - `files`: where the paths are collected.
fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())) {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// Scans `files` and returns how many key values it found, with one
/// `<file>:<line>` for each that the refusal does not list. The value itself
/// is not repeated.
///
/// - `files`: the files to scan.
fn unrefused_key_values(files: &[PathBuf]) -> (usize, Vec<String>) {
    let root = repository_root();
    let mut scanned = 0;
    let mut failures = Vec::new();
    for file in files {
        let text =
            fs::read_to_string(file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        for (line, value) in key_values(&text) {
            scanned += 1;
            if !listed_by_the_refusal(&value) {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                failures.push(format!("{}:{line}", shown.display()));
            }
        }
    }
    (scanned, failures)
}

/// The fix that a failure message names.
const FIX: &str = "production does not refuse it (ADR 0034 §3). Add a committed key to the \
                   refusal in the same PR (Settings::refuse_published_keys; a JWT secret goes \
                   into PUBLISHED_JWT_SECRETS), or, in a test that starts the production \
                   configuration, make the key at run time (tests/support cli_jwt_secret and \
                   cli_totp_key)";

#[test]
fn every_key_value_of_the_local_configurations_is_refused_in_production() {
    let files = local_config_files();
    let names: Vec<String> = files
        .iter()
        .map(|file| {
            file.file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    for expected in ["development.yaml", "e2e.yaml", "test.yaml"] {
        assert!(names.iter().any(|name| name == expected), "{names:?}");
    }
    let (scanned, failures) = unrefused_key_values(&files);
    // Each of the three files holds its JWT secret and the TOTP key.
    assert!(scanned >= 6, "only {scanned} key values found in {names:?}");
    assert!(
        failures.is_empty(),
        "key values at {}: {FIX}",
        failures.join(", ")
    );
}

#[test]
fn every_key_value_under_tests_is_refused_in_production() {
    let mut files = Vec::new();
    rust_files(&repository_root().join("tests"), &mut files);
    assert!(files.len() > 10, "{files:?}");
    let (_, failures) = unrefused_key_values(&files);
    assert!(
        failures.is_empty(),
        "key values at {}: {FIX}",
        failures.join(", ")
    );
}

/// The two keys the refusal covers, read where Loco reads them: the loaded
/// configuration, after its templates are rendered, of the committed file
/// alone. The text scan above sees them too; this test also covers a value
/// that only rendering produces.
#[test]
fn the_loaded_keys_of_every_local_configuration_are_refused_in_production() {
    for file in local_config_files() {
        let stem = file
            .file_stem()
            .expect("stem")
            .to_string_lossy()
            .into_owned();
        let environment = Environment::from(stem);
        let config = load_committed_config(&environment);
        let secret = config.get_jwt_config().expect("auth.jwt").secret.clone();
        check_session_jwt(config.auth.as_ref())
            .unwrap_or_else(|error| panic!("{environment}: {error}"));
        assert!(
            listed_by_the_refusal(&secret),
            "{environment}: auth.jwt.secret"
        );
        let totp_key =
            config.settings.as_ref().expect("settings")["admin_auth"]["totp_encryption_key"]
                .as_str()
                .expect("a string")
                .to_owned();
        assert!(
            listed_by_the_refusal(&totp_key),
            "{environment}: settings.admin_auth.totp_encryption_key"
        );
    }
}

#[test]
fn the_refusal_lists_the_published_keys_and_nothing_made_now() {
    assert!(listed_by_the_refusal(PUBLISHED_LOCAL_KEY));
    assert!(!listed_by_the_refusal(&cli_jwt_secret()));
    assert!(!listed_by_the_refusal(&cli_totp_key()));
    // A short value is refused by the format check, which is not the list.
    assert!(!listed_by_the_refusal(&STANDARD.encode([7_u8; 40])));
}

#[test]
fn the_scan_finds_key_values_wherever_they_are_written() {
    let key = cli_jwt_secret();
    let short_key = STANDARD.encode([0x5a_u8; 32]);
    let text = format!(
        "secret: \"{key}\"\n// a comment that quotes {key}\nconst K: &str = r#\"{short_key}\"#;\n\
         default=\"{key}\"\n"
    );
    let found = key_values(&text);
    assert_eq!(
        found,
        [(1, key.clone()), (2, key.clone()), (3, short_key), (4, key)]
    );
}

#[test]
fn the_scan_leaves_out_what_cannot_be_a_key() {
    let hex_digest = "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d";
    let path = "images/index/0f1e2d3c4b5a69788796a5b4c3d2e1f0aaaa";
    let upper = STANDARD.encode([0x5a_u8; 48]).to_ascii_uppercase();
    let short = STANDARD.encode([0x5a_u8; 31]);
    let not_canonical = STANDARD.encode([0x61_u8; 64]).replace("YQ==", "YR==");
    let identifier = "the_run_time_secret_is_valid_unpublished_and_new_each_time";
    let prose = "Every committed configuration other than production holds keys.";
    for text in [
        hex_digest,
        path,
        upper.as_str(),
        short.as_str(),
        not_canonical.as_str(),
        identifier,
        prose,
    ] {
        assert_eq!(key_values(text), [], "{text}");
    }
}

/// The scan leaves out exactly the overrides that `.gitignore` keeps out of
/// the repository, so a developer's own key there fails nothing, while a
/// new committed configuration (one `.gitignore` does not cover) is scanned.
#[test]
fn the_scan_leaves_out_only_the_git_ignored_overrides() {
    let gitignore = fs::read_to_string(repository_root().join(".gitignore")).expect(".gitignore");
    for pattern in ["**/config/local.yaml", "**/config/*.local.yaml"] {
        assert!(
            gitignore.lines().any(|line| line.trim() == pattern),
            ".gitignore no longer ignores {pattern}; update is_personal_override with it"
        );
    }
    for name in [
        "local.yaml",
        "development.local.yaml",
        "test.local.yaml",
        "e2e.local.yaml",
        "production.local.yaml",
    ] {
        assert!(is_personal_override(name), "{name}");
    }
    for name in [
        "development.yaml",
        "e2e.yaml",
        "staging.yaml",
        "local.yml",
        "development.local.yml",
        "localdev.yaml",
    ] {
        assert!(!is_personal_override(name), "{name}");
    }
}
