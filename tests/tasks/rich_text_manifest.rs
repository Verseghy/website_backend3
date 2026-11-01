//! Tests of the `rich_text_manifest` task (editor.md §3.9, ADR 0016 §4).
//!
//! The task runs as the CLI runs it, through `loco_rs::boot::run_task` with
//! the app's own task registry, on a per-test database (the task lock needs
//! PostgreSQL). Every test writes into a scratch directory of its own below
//! Cargo's per-target scratch directory, never into `admin/src/rich-text/`:
//! the committed copies are compared with the renderer by
//! `tests/rich_text/manifest.rs`, and a test must not change the source
//! tree.

use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use backend3::{
    app::App,
    domain::rich_text::{
        SanitizePolicy,
        manifest::{ManifestFile, PolicyTables, manifest_files},
    },
    tasks::rich_text_manifest::{
        FileOutcome, TASK_NAME, replace_file, temporary_path, write_manifest,
    },
};
use loco_rs::{Result, app::AppContext, boot::run_task, task::Vars};

use crate::support::{hold_task_lock, with_app};

/// A fresh, empty directory that is removed again when the guard drops,
/// also when the test panics.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("rich-text-manifest-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create the scratch directory");
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// The names in the directory, sorted.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .expect("read the scratch directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .into_string()
                    .expect("UTF-8")
            })
            .collect();
        names.sort_unstable();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A test may have made the directory read-only.
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The two files the task must write.
fn expected() -> [ManifestFile; 2] {
    manifest_files(&PolicyTables::CONTENT_V1, &SanitizePolicy::content_v1())
        .expect("content-v1 renders")
}

/// Runs the task through the app's registry with `key:value` arguments.
async fn run(ctx: &AppContext, arguments: &[(&str, &str)]) -> Result<()> {
    let vars = Vars::from_cli_args(
        arguments
            .iter()
            .map(|&(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
    );
    run_task::<App>(ctx, Some(&TASK_NAME.to_owned()), &vars).await
}

fn dir_argument(scratch: &Scratch) -> String {
    scratch.0.to_str().expect("a UTF-8 path").to_owned()
}

#[tokio::test]
async fn the_task_writes_both_files_into_the_directory() {
    with_app(|_router, ctx| async move {
        let scratch = Scratch::new("writes");
        run(&ctx, &[("dir", &dir_argument(&scratch))])
            .await
            .expect("the task succeeds");

        assert_eq!(
            scratch.names(),
            ["css-vectors.jsonl", "policy.json"],
            "no temporary file is left"
        );
        for file in expected() {
            let written = fs::read_to_string(scratch.file(file.name)).expect("the file exists");
            assert_eq!(written, file.contents, "{}", file.name);
        }
    })
    .await;
}

#[tokio::test]
async fn a_second_run_leaves_equal_files_untouched() {
    with_app(|_router, ctx| async move {
        let scratch = Scratch::new("idempotent");
        let dir = dir_argument(&scratch);
        run(&ctx, &[("dir", &dir)]).await.expect("the first run");
        let inode = |name: &str| fs::metadata(scratch.file(name)).expect("the file").ino();
        let before = (inode("policy.json"), inode("css-vectors.jsonl"));

        run(&ctx, &[("dir", &dir)]).await.expect("the second run");
        // A rewrite renames a new file into place, which changes the inode.
        assert_eq!((inode("policy.json"), inode("css-vectors.jsonl")), before);
        for file in expected() {
            assert_eq!(
                fs::read_to_string(scratch.file(file.name)).unwrap(),
                file.contents
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_stale_file_is_replaced_and_other_files_are_left_alone() {
    with_app(|_router, ctx| async move {
        let scratch = Scratch::new("stale");
        fs::write(
            scratch.file("policy.json"),
            "{\"policy_version\": \"content-v0\"}\n",
        )
        .unwrap();
        fs::write(scratch.file("canonical-style.ts"), "export {};\n").unwrap();

        run(&ctx, &[("dir", &dir_argument(&scratch))])
            .await
            .expect("the task succeeds");

        let [policy, vectors] = expected();
        assert_eq!(
            fs::read_to_string(scratch.file("policy.json")).unwrap(),
            policy.contents
        );
        assert_eq!(
            fs::read_to_string(scratch.file("css-vectors.jsonl")).unwrap(),
            vectors.contents
        );
        assert_eq!(
            fs::read_to_string(scratch.file("canonical-style.ts")).unwrap(),
            "export {};\n"
        );
        assert_eq!(
            scratch.names(),
            ["canonical-style.ts", "css-vectors.jsonl", "policy.json"]
        );
    })
    .await;
}

/// The lock-held test every task has (ADR 0016, Consequences; recipe 6):
/// while another run holds the lock, the task exits successfully and writes
/// nothing.
#[tokio::test]
async fn the_task_writes_nothing_while_another_run_holds_the_lock() {
    with_app(|_router, ctx| async move {
        let scratch = Scratch::new("locked");
        let dir = dir_argument(&scratch);

        let other_run = hold_task_lock(&ctx.db, TASK_NAME).await;
        run(&ctx, &[("dir", &dir)])
            .await
            .expect("exit 0 while the lock is held");
        assert!(
            scratch.names().is_empty(),
            "nothing was written: {:?}",
            scratch.names()
        );

        other_run.rollback().await.expect("release the other run");
        run(&ctx, &[("dir", &dir)])
            .await
            .expect("a free lock runs the task");
        assert_eq!(scratch.names(), ["css-vectors.jsonl", "policy.json"]);
    })
    .await;
}

/// Bad arguments fail the run with a message that says what to do, and they
/// are checked before the lock: even while another run holds it, the run
/// fails instead of skipping.
#[tokio::test]
async fn bad_arguments_are_refused_before_the_lock_is_taken() {
    with_app(|_router, ctx| async move {
        let scratch = Scratch::new("refused");
        let file = scratch.file("a-file");
        fs::write(&file, "").unwrap();
        let file = file.to_str().unwrap().to_owned();
        let missing = scratch.file("missing").to_str().unwrap().to_owned();
        let other_run = hold_task_lock(&ctx.db, TASK_NAME).await;

        for (arguments, needle) in [
            (
                vec![("dri", missing.as_str())],
                "unknown argument [\"dri\"]",
            ),
            (vec![("dir", "")], "`dir:` needs a directory"),
            (
                vec![("dir", missing.as_str())],
                "does not exist; run the task from the repository root",
            ),
            (vec![("dir", file.as_str())], "is not a directory"),
        ] {
            let error = run(&ctx, &arguments).await.expect_err("refused");
            let message = error.to_string();
            assert!(
                message.starts_with("task rich_text_manifest: "),
                "{message}"
            );
            assert!(message.contains(needle), "{arguments:?}: {message}");
        }
        other_run.rollback().await.expect("release the other run");
        assert_eq!(scratch.names(), ["a-file"], "nothing was written");
        assert!(
            !Path::new(&missing).exists(),
            "the task never creates the directory"
        );
    })
    .await;
}

#[tokio::test]
async fn write_manifest_reports_each_file_in_order() {
    let scratch = Scratch::new("outcomes");
    let files = expected();
    let first = write_manifest(&scratch.0, &files).await.expect("written");
    assert_eq!(
        first,
        [
            ("policy.json", FileOutcome::Written),
            ("css-vectors.jsonl", FileOutcome::Written)
        ]
    );
    let second = write_manifest(&scratch.0, &files).await.expect("unchanged");
    assert_eq!(
        second,
        [
            ("policy.json", FileOutcome::Unchanged),
            ("css-vectors.jsonl", FileOutcome::Unchanged)
        ]
    );
    assert_eq!(FileOutcome::Written.as_str(), "written");
    assert_eq!(FileOutcome::Unchanged.as_str(), "unchanged");
}

#[tokio::test]
async fn write_manifest_names_the_file_it_could_not_read() {
    let scratch = Scratch::new("unreadable");
    // A directory where the file should be: reading it fails, and the error
    // names the path and asks for another run.
    fs::create_dir(scratch.file("policy.json")).unwrap();
    let error = write_manifest(&scratch.0, &expected())
        .await
        .expect_err("refused");
    let message = error.to_string();
    assert!(message.contains("could not read"), "{message}");
    assert!(message.contains("policy.json"), "{message}");
    assert!(message.contains("run the task again"), "{message}");
    assert_eq!(
        scratch.names(),
        ["policy.json"],
        "no temporary file is left"
    );
}

#[test]
fn the_temporary_file_is_a_hidden_sibling_named_after_the_process() {
    let path = temporary_path(Path::new("out"), "policy.json");
    assert_eq!(
        path,
        Path::new("out").join(format!(".policy.json.{}.tmp", std::process::id()))
    );
}

/// A directory the task may not write to: the temporary file cannot be
/// created, and the error names it and asks for another run.
#[tokio::test]
async fn write_manifest_names_the_temporary_file_it_could_not_write() {
    let scratch = Scratch::new("read-only");
    fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o555)).unwrap();
    // A process that may write anyway (root, or CAP_DAC_OVERRIDE) cannot
    // reach this failure.
    if fs::write(scratch.file("probe"), "").is_ok() {
        eprintln!("skipped: this process writes into read-only directories");
        return;
    }
    let error = write_manifest(&scratch.0, &expected())
        .await
        .expect_err("refused");
    let message = error.to_string();
    let temporary = temporary_path(&scratch.0, "policy.json");
    assert!(
        message.contains(&format!("could not write `{}`", temporary.display())),
        "{message}"
    );
    assert!(message.contains("run the task again"), "{message}");
    assert_eq!(scratch.names(), Vec::<String>::new(), "nothing is left");
}

/// A write that fails after the temporary file was created (here: the
/// temporary name is a link to `/dev/full`, where every write fails) removes
/// it again, and leaves the target alone.
#[tokio::test]
async fn a_failed_write_removes_the_temporary_file() {
    let full = Path::new("/dev/full");
    if !full.exists() {
        eprintln!("skipped: no /dev/full");
        return;
    }
    let scratch = Scratch::new("write-fails");
    let temporary = scratch.file(".policy.json.tmp");
    std::os::unix::fs::symlink(full, &temporary).unwrap();
    let error = replace_file(&temporary, &scratch.file("policy.json"), b"{}\n")
        .await
        .expect_err("refused");
    let message = error.to_string();
    assert!(
        message.contains(&format!("could not write `{}`", temporary.display())),
        "{message}"
    );
    assert!(message.contains("run the task again"), "{message}");
    assert_eq!(
        scratch.names(),
        Vec::<String>::new(),
        "the temporary file is removed, and no target was created"
    );
}

/// A rename that fails (here: the target is a directory that is not empty)
/// removes the written temporary file and leaves the target as it was.
#[tokio::test]
async fn a_failed_rename_removes_the_temporary_file() {
    let scratch = Scratch::new("rename-fails");
    let target = scratch.file("policy.json");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("kept"), "").unwrap();
    let temporary = scratch.file(".policy.json.tmp");
    let error = replace_file(&temporary, &target, b"{}\n")
        .await
        .expect_err("refused");
    let message = error.to_string();
    assert!(
        message.contains(&format!("could not replace `{}`", target.display())),
        "{message}"
    );
    assert!(message.contains("run the task again"), "{message}");
    assert_eq!(
        scratch.names(),
        ["policy.json"],
        "no temporary file is left"
    );
    assert!(target.join("kept").exists(), "the target is untouched");
}

#[tokio::test]
async fn replace_file_writes_and_replaces() {
    let scratch = Scratch::new("replace");
    let target = scratch.file("policy.json");
    let temporary = scratch.file(".policy.json.tmp");
    replace_file(&temporary, &target, b"one\n")
        .await
        .expect("created");
    fs::write(&temporary, "stale").unwrap();
    replace_file(&temporary, &target, b"two\n")
        .await
        .expect("replaced");
    assert_eq!(fs::read_to_string(&target).unwrap(), "two\n");
    assert_eq!(
        scratch.names(),
        ["policy.json"],
        "no temporary file is left"
    );
}
