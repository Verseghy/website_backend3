//! The resolution file's parser takes time in proportion to the file, also
//! for the largest file the task reads (`MAX_FILE_BYTES`,
//! `domain::import_mysql::resolutions`).
//!
//! The size limit bounds the memory a wrong file can take, not the time: a
//! check that compares every id of an array with every other would make
//! one array of about 150,000 ids, which fits in 1 MiB, take seconds in a
//! release build and minutes in a test build. The pure module's own tests
//! cannot measure time (ADR 0008 rule 1 keeps the clock out of
//! `src/domain/`), so this test does, with a budget far above what a linear
//! parser needs and far below what a quadratic one takes.

use std::time::{Duration, Instant};

use backend3::domain::import_mysql::resolutions::{MAX_FILE_BYTES, parse};

/// The longest a parse of one largest file may take, in any build. A linear
/// parser takes well under a second for one in a test build.
const BUDGET: Duration = Duration::from_secs(20);

/// The ids `1, 2, 3, ...` as a TOML array body (without the brackets), as
/// many as fit in `bytes` bytes.
fn ids_within(bytes: usize) -> (String, usize) {
    let mut text = String::with_capacity(bytes);
    let mut count = 0_usize;
    loop {
        let next = if count == 0 {
            "1".to_owned()
        } else {
            format!(", {}", count + 1)
        };
        if text.len() + next.len() > bytes {
            return (text, count);
        }
        text.push_str(&next);
        count += 1;
    }
}

/// Parses `text`, which must be a valid file at most `MAX_FILE_BYTES` long,
/// and returns how long that took.
fn timed_parse(text: &str) -> Duration {
    let limit = usize::try_from(MAX_FILE_BYTES).expect("the limit fits in memory");
    assert!(text.len() <= limit, "{} bytes", text.len());
    let start = Instant::now();
    let parsed = parse(text);
    let elapsed = start.elapsed();
    parsed.unwrap_or_else(|errors| panic!("{errors}"));
    elapsed
}

#[test]
fn one_array_as_long_as_the_size_limit_allows_parses_within_the_budget() {
    let limit = usize::try_from(MAX_FILE_BYTES).expect("the limit fits in memory");
    // D8: one label group with every id of the file.
    let head = "version = 1\n[[label_duplicate]]\nkeep = 1\nexpect_ids = [";
    let tail = "]\n";
    let (ids, count) = ids_within(limit - head.len() - tail.len());
    assert!(count > 100_000, "{count} ids");
    let elapsed = timed_parse(&format!("{head}{ids}{tail}"));
    assert!(
        elapsed < BUDGET,
        "{count} ids took {elapsed:?}: the parser is not linear in the file"
    );

    // D9: two arrays of half the file each, the items and their order.
    let head = "version = 1\n[[menu_unpositioned]]\nparent_id = \"none\"\nexpect_null_lft = [";
    let middle = "]\nserved_order = [";
    let (ids, count) = ids_within((limit - head.len() - middle.len() - tail.len()) / 2);
    let elapsed = timed_parse(&format!("{head}{ids}{middle}{ids}{tail}"));
    assert!(
        elapsed < BUDGET,
        "two arrays of {count} ids took {elapsed:?}: the parser is not linear in the file"
    );
}

#[test]
fn as_many_entries_as_the_size_limit_allows_parse_within_the_budget() {
    let limit = usize::try_from(MAX_FILE_BYTES).expect("the limit fits in memory");
    // D1 and D5 together: every D5 post is checked against the D1 posts.
    let mut text = "version = 1\n".to_owned();
    let mut post = 1_u32;
    loop {
        let entry = if post.is_multiple_of(2) {
            format!("[[post_date_shift]]\npost_id = {post}\nexpect_date = 2019-05-02\n")
        } else {
            format!("[[post_sentinel_date]]\npost_id = {post}\nexpect_date = 1970-01-01\n")
        };
        if text.len() + entry.len() > limit {
            break;
        }
        text.push_str(&entry);
        post += 1;
    }
    let entries = post - 1;
    assert!(entries > 10_000, "{entries} entries");
    let elapsed = timed_parse(&text);
    assert!(
        elapsed < BUDGET,
        "{entries} entries took {elapsed:?}: the parser is not linear in the file"
    );
}
