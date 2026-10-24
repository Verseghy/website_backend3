//! The database-target guard on a loaded configuration: every connection of
//! the configuration is checked, with the flags that act on it
//! (ADR 0024 §4, layer 3). `tests/cli/database_guard.rs` checks the same
//! guard end to end through the binary.

use backend3::app::enforce_database_guard;
use loco_rs::{
    config::{Config, QueueConfig},
    environment::Environment,
};

fn test_config() -> Config {
    Environment::Test.load().expect("config/test.yaml")
}

#[test]
fn a_non_local_database_uri_is_refused_in_test() {
    let mut config = test_config();
    config.database.uri = "postgres://loco:secret@db.example.org:5432/backend3_test".to_owned();
    let message = enforce_database_guard(&Environment::Test, &config)
        .expect_err("refused")
        .to_string();
    assert!(
        message.starts_with(
            "Refusing to start: LOCO_ENV=test, but the database host in database.uri is \
             'db.example.org'."
        ),
        "{message}"
    );
    assert!(
        !message.contains("secret"),
        "the password never appears: {message}"
    );
}

#[test]
fn a_non_local_queue_uri_is_refused_in_development() {
    let mut config = Environment::Development
        .load()
        .expect("config/development.yaml");
    match config.queue.as_mut() {
        Some(QueueConfig::Postgres(queue)) => {
            queue.uri = "postgres://queue.example.org/x".to_owned();
        }
        other => panic!("development queue must be Postgres, got {other:?}"),
    }
    let message = enforce_database_guard(&Environment::Development, &config)
        .expect_err("refused")
        .to_string();
    assert!(
        message.contains("queue.uri is 'queue.example.org'"),
        "{message}"
    );
}

#[test]
fn destructive_flags_are_refused_in_production() {
    let mut config = test_config();
    config.database.uri = "postgres://website_backend3@postgres:5432/verseghy".to_owned();
    config.database.dangerously_recreate = true;
    let message = enforce_database_guard(&Environment::Production, &config)
        .expect_err("refused")
        .to_string();
    assert!(
        message.contains("database.dangerously_recreate is true"),
        "{message}"
    );

    config.database.dangerously_recreate = false;
    enforce_database_guard(&Environment::Production, &config).expect("no flags, remote host");
}

#[test]
fn destructive_flags_on_a_local_test_database_are_allowed() {
    let mut config = test_config();
    config.database.uri = "postgres://loco:loco@localhost:5432/backend3_test".to_owned();
    config.database.dangerously_truncate = true;
    enforce_database_guard(&Environment::Test, &config).expect("local is fine");
}
