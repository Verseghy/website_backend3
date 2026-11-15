//! The image carries the repository's `data/` directory at `/app/data/`, and
//! the committed resolution file there is valid (ADR 0045; data-migration.md
//! §4.1, §8.2; research: DECISIONS R9, D12).
//!
//! The import's `resolutions` argument defaults to
//! `/app/data/import/resolutions.toml`, and the nightly rehearsal and the
//! cut-over run with that default, so the image must ship the file. Only
//! Pipelines-as-Code builds the image, after a merge (ADR 0022 §1): a PR
//! whose runtime stage stopped copying `data/` would pass every GitHub
//! Actions job and break the first import after it. These tests read the
//! `Containerfile` instead, the way `scripts/ci/check-toolchain.sh` checks
//! its static rules:
//!
//! - the runtime stage copies `/src/data/` from the rust stage to
//!   `/app/data/`, the rust stage works in `/src` and copies the whole build
//!   context there, and `.containerignore` keeps `data/` in the context;
//! - the default `resolutions` path is the committed
//!   `data/import/resolutions.toml` under `/app/`;
//! - the committed file passes the importer's own validation and its size
//!   limit, so a broken list fails the PR, not the nightly;
//! - `data/import/` holds only the files its README allows;
//! - data-migration.md §8.2 documents exactly the sections and fields of the
//!   schema (`domain::import_mysql::resolutions::Section`).
//!
//! The Containerfile check is a function, [`check_runtime_copies_data`],
//! with tests of its own on changed copies of the file, so it is known to
//! fail when the line goes.

use std::{collections::BTreeSet, fs};

use backend3::domain::import_mysql::{
    options::DEFAULT_RESOLUTIONS,
    resolutions::{self, MAX_FILE_BYTES, Section},
};

use crate::support::repository_root;

/// The instruction that ships `data/`, as [`instructions`] normalises it.
const COPY_DATA: &str = "COPY --from=rust /src/data/ /app/data/";

/// The text of a file of the repository.
fn read(relative: &str) -> String {
    fs::read_to_string(repository_root().join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// The instructions of a Containerfile: comment lines and blank lines
/// dropped, continuation lines (`\` at the end) joined, and runs of white
/// space reduced to one space.
fn instructions(containerfile: &str) -> Vec<String> {
    let mut instructions = Vec::new();
    let mut current = String::new();
    for line in containerfile.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || (trimmed.is_empty() && current.is_empty()) {
            continue;
        }
        let (part, continues) = match trimmed.strip_suffix('\\') {
            Some(part) => (part, true),
            None => (trimmed, false),
        };
        current.push(' ');
        current.push_str(part);
        if !continues {
            let instruction = current.split_whitespace().collect::<Vec<_>>().join(" ");
            if !instruction.is_empty() {
                instructions.push(instruction);
            }
            current.clear();
        }
    }
    instructions
}

/// The stages of a Containerfile: each `FROM ... AS <name>` with the
/// instructions that follow it, up to the next `FROM`.
fn stages(containerfile: &str) -> Vec<(String, Vec<String>)> {
    let mut stages: Vec<(String, Vec<String>)> = Vec::new();
    for instruction in instructions(containerfile) {
        let words: Vec<&str> = instruction.split(' ').collect();
        if words[0].eq_ignore_ascii_case("FROM") {
            let name = match words.as_slice() {
                [.., keyword, name] if keyword.eq_ignore_ascii_case("AS") => (*name).to_owned(),
                _ => String::new(),
            };
            stages.push((name, Vec::new()));
        } else if let Some((_, body)) = stages.last_mut() {
            body.push(instruction);
        }
    }
    stages
}

/// Checks that `containerfile` ships `data/` at `/app/data/`: its last
/// stage is `runtime` and holds [`COPY_DATA`], and its `rust` stage works in
/// `/src` and copies the whole build context there (`COPY . .`), so that
/// `/src/data/` exists.
///
/// - `containerfile`: the text of a Containerfile.
///
/// Returns what is missing.
fn check_runtime_copies_data(containerfile: &str) -> Result<(), String> {
    let stages = stages(containerfile);
    let Some((last, runtime)) = stages.last() else {
        return Err("the Containerfile has no stage".to_owned());
    };
    if last != "runtime" {
        return Err(format!(
            "the last stage is {last:?}, not runtime, the stage that is the image"
        ));
    }
    if !runtime.iter().any(|instruction| instruction == COPY_DATA) {
        return Err(format!(
            "the runtime stage does not copy data/: it needs `{COPY_DATA}`, so that the image \
             holds {DEFAULT_RESOLUTIONS}, the default of the import's resolutions argument"
        ));
    }
    let Some((_, rust)) = stages.iter().find(|(name, _)| name == "rust") else {
        return Err("the Containerfile has no rust stage".to_owned());
    };
    let workdir = rust
        .iter()
        .rposition(|instruction| instruction == "WORKDIR /src");
    let copy_all = rust
        .iter()
        .position(|instruction| instruction == "COPY . .");
    match (workdir, copy_all) {
        (Some(workdir), Some(copy_all)) if workdir < copy_all => Ok(()),
        _ => Err(
            "the rust stage does not copy the build context into /src (`WORKDIR /src`, then \
             `COPY . .`), so /src/data/ is not there to copy"
                .to_owned(),
        ),
    }
}

#[test]
fn the_runtime_stage_copies_data_into_the_image() {
    check_runtime_copies_data(&read("Containerfile")).unwrap_or_else(|problem| panic!("{problem}"));
}

#[test]
fn the_check_fails_when_the_runtime_stage_stops_copying_data() {
    let containerfile = read("Containerfile");
    let line = format!("{COPY_DATA}\n");
    assert!(
        containerfile.contains(&line),
        "the Containerfile has no {COPY_DATA:?} line"
    );

    let without = containerfile.replace(&line, "");
    assert_eq!(
        check_runtime_copies_data(&without),
        Err(format!(
            "the runtime stage does not copy data/: it needs `{COPY_DATA}`, so that the image \
             holds /app/data/import/resolutions.toml, the default of the import's resolutions \
             argument"
        ))
    );

    // The same line in another stage does not put data/ into the image.
    let in_rust_stage = without.replace("WORKDIR /src\n", &format!("WORKDIR /src\n{line}"));
    assert!(check_runtime_copies_data(&in_rust_stage).is_err());

    // Another destination, or a part of data/ only, is not the line either.
    for changed in [
        "COPY --from=rust /src/data/import/ /app/data/import/\n",
        "COPY --from=rust /src/data/ /app/\n",
    ] {
        assert!(
            check_runtime_copies_data(&containerfile.replace(&line, changed)).is_err(),
            "{changed}"
        );
    }

    // A continuation line and extra spaces are still the line.
    let split = containerfile.replace(&line, "COPY   --from=rust \\\n    /src/data/ /app/data/\n");
    assert_eq!(check_runtime_copies_data(&split), Ok(()));

    // Without the build context in /src there is no /src/data/ to copy.
    let no_context = containerfile.replace("COPY . .\n", "COPY src/ src/\n");
    assert!(check_runtime_copies_data(&no_context).is_err());

    // A runtime stage that is not the last one is not the image.
    let later = format!("{containerfile}\nFROM scratch AS extra\n");
    assert!(check_runtime_copies_data(&later).is_err());
}

/// Whether the glob `pattern` matches all of `name`, one path segment, as
/// the build context's ignore files read a segment (Go's
/// `filepath.Match`): `*` is any run of characters, `?` any one character,
/// `[...]` one character of a class (ranges `a-z`, negated by a leading `^`
/// or `!`), and `\` makes the next character literal.
///
/// - `pattern`: one segment of a `.containerignore` pattern.
/// - `name`: the segment it is matched against.
///
/// A class that never closes cannot be read, and counts as a match, so the
/// check errs on the side of refusing a pattern.
fn segment_matches(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|skip| segment_matches(rest, &name[skip..])),
        Some(('?', rest)) => !name.is_empty() && segment_matches(rest, &name[1..]),
        Some(('[', rest)) => {
            let Some((&first, after)) = name.split_first() else {
                return false;
            };
            match class_matches(rest, first) {
                Some((matched, after_class)) => matched && segment_matches(after_class, after),
                None => true,
            }
        }
        Some(('\\', rest)) => match rest.split_first() {
            Some((literal, rest)) => {
                name.first() == Some(literal) && segment_matches(rest, &name[1..])
            }
            None => true,
        },
        Some((literal, rest)) => name.first() == Some(literal) && segment_matches(rest, &name[1..]),
    }
}

/// Reads the character class that starts `class` (just after its `[`):
/// whether `character` is in it, and the pattern after its `]`, or `None`
/// when it never closes. A `]` right after the `[` (or after the negation)
/// is a member, as in Go's `filepath.Match`.
fn class_matches(class: &[char], character: char) -> Option<(bool, &[char])> {
    let (negated, mut rest) = match class.split_first() {
        Some(('^' | '!', rest)) => (true, rest),
        _ => (false, class),
    };
    let mut matched = false;
    let mut first = true;
    loop {
        let (&start, after) = rest.split_first()?;
        if start == ']' && !first {
            return Some((matched != negated, after));
        }
        first = false;
        let (start, after) = match (start, after.split_first()) {
            ('\\', Some((&escaped, after))) => (escaped, after),
            _ => (start, after),
        };
        let (end, after) = match after {
            ['-', end, after @ ..] if *end != ']' => match (end, after.split_first()) {
                ('\\', Some((&escaped, after))) => (escaped, after),
                _ => (*end, after),
            },
            _ => (start, after),
        };
        matched |= (start..=end).contains(&character);
        rest = after;
    }
}

/// Whether a `.containerignore` pattern can leave `data/`, or a file below
/// it, out of the build context: its first path segment matches `data`.
///
/// - `pattern`: one line of `.containerignore`, without comments.
///
/// A re-include (`!`) is refused too, since `data/` needs none, and a
/// pattern that starts with `**` matches every path.
fn covers_data(pattern: &str) -> bool {
    let mut path = pattern.trim_start_matches('!').trim_start_matches('/');
    while let Some(rest) = path.strip_prefix("./") {
        path = rest.trim_start_matches('/');
    }
    let first: Vec<char> = path.split('/').next().unwrap_or_default().chars().collect();
    let data: Vec<char> = "data".chars().collect();
    segment_matches(&first, &data)
}

#[test]
fn the_build_context_keeps_data() {
    // .containerignore lists paths that never reach the builder; none of
    // them may cover data/ or a file under it.
    let ignored: Vec<String> = read(".containerignore")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    assert!(!ignored.is_empty(), ".containerignore lists nothing");
    for pattern in &ignored {
        assert!(
            !covers_data(pattern),
            ".containerignore pattern {pattern:?} can leave data/ out of the image"
        );
    }
}

#[test]
fn the_build_context_check_refuses_every_pattern_that_matches_data() {
    for pattern in [
        "data",
        "data/",
        "/data",
        "./data",
        "data/import/",
        "data/import/resolutions.toml",
        "data*",
        "d*",
        "d*/",
        "*",
        "*/",
        "**",
        "**/resolutions.toml",
        "*/import",
        "?ata",
        "????",
        "d?t?",
        "[d]ata",
        "[a-e]ata",
        "[^x]ata",
        "[!x]ata",
        "da[s-u]a",
        "d\\ata",
        "!data/import/",
        "[data",
    ] {
        assert!(covers_data(pattern), "{pattern:?} covers data/");
    }
    for pattern in [
        "target/",
        "admin/dist/",
        ".env.*",
        "config/*.local.yaml",
        "dat",
        "datas",
        "data.bak",
        "?",
        "???",
        "?????",
        "[^d]ata",
        "[a-c]ata",
        "da[^t]a",
        "x*",
        "*x",
        "d\\*",
        "docs/data",
    ] {
        assert!(!covers_data(pattern), "{pattern:?} does not cover data/");
    }
}

#[test]
fn the_default_resolutions_path_is_the_committed_file_in_the_image() {
    let in_repository = DEFAULT_RESOLUTIONS
        .strip_prefix("/app/")
        .expect("the default lies under /app/, where the image keeps data/");
    assert_eq!(in_repository, "data/import/resolutions.toml");
    assert!(
        repository_root().join(in_repository).is_file(),
        "{in_repository} is not committed"
    );
}

#[test]
fn the_committed_resolution_file_is_valid() {
    let path = repository_root().join("data/import/resolutions.toml");
    let length = fs::metadata(&path).expect("the resolution file").len();
    assert!(
        length <= MAX_FILE_BYTES,
        "the resolution file has {length} bytes, more than the task reads"
    );
    let text = fs::read_to_string(&path).expect("the resolution file is UTF-8");
    resolutions::parse(&text).unwrap_or_else(|errors| {
        panic!("data/import/resolutions.toml is not a valid resolution file: {errors}")
    });
}

#[test]
fn data_import_holds_only_the_files_its_readme_allows() {
    let listed: BTreeSet<String> = fs::read_dir(repository_root().join("data/import"))
        .expect("data/import")
        .map(|entry| {
            entry
                .expect("an entry of data/import")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let allowed = BTreeSet::from(["README.md".to_owned(), "resolutions.toml".to_owned()]);
    assert_eq!(
        listed, allowed,
        "data/import/ holds other files than README.md allows; change the README (and this \
         test) first if a new file belongs there"
    );
    let readme = read("data/import/README.md");
    for file in &allowed {
        assert!(
            readme.contains(&format!("| `{file}` |")),
            "README.md does not list {file}"
        );
    }
}

/// The rows of the schema table of data-migration.md §8.2: each section's
/// key, its defect id and its fields, in the table's order.
fn documented_schema() -> Vec<(String, String, Vec<String>)> {
    let document = read("docs/design/data-migration.md");
    let section = document
        .split_once("\n### 8.2 The resolution file\n")
        .expect("section 8.2")
        .1;
    let section = section
        .split_once("\n## ")
        .map_or(section, |(body, _)| body);
    section
        .lines()
        .filter_map(|line| line.strip_prefix("| `[["))
        .map(|row| {
            let (key, rest) = row.split_once("]]`").expect("a section key");
            let cells: Vec<&str> = rest.split('|').map(str::trim).collect();
            let fields = cells[2]
                .split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect();
            (key.to_owned(), cells[1].to_owned(), fields)
        })
        .collect()
}

#[test]
fn the_design_documents_every_section_and_field_of_the_schema() {
    let documented = documented_schema();
    let built: Vec<(String, String, Vec<String>)> = Section::ALL
        .iter()
        .map(|section| {
            (
                section.key().to_owned(),
                section.defect().to_owned(),
                section
                    .fields()
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect(),
            )
        })
        .collect();
    assert!(!documented.is_empty(), "no schema row found in section 8.2");
    assert_eq!(documented, built);
}
