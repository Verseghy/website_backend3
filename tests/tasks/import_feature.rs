//! The `import-mysql` Cargo feature decides whether the legacy import
//! exists (data-migration.md §3.3): the default build registers the
//! `import_mysql` task, and a build without the feature
//! (`--no-default-features`, a step of the CI Test job) registers no such
//! task. This test is compiled in both builds, so each proves its half.

use backend3::app::App;
use loco_rs::{app::Hooks, task::Tasks};

#[test]
fn the_import_task_is_registered_exactly_when_its_feature_is_on() {
    let mut tasks = Tasks::default();
    App::register_tasks(&mut tasks);
    let names = tasks.names();
    assert_eq!(
        names.iter().any(|name| name == "import_mysql"),
        cfg!(feature = "import-mysql"),
        "registered tasks: {names:?}"
    );
}
