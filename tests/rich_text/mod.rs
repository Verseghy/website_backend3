//! Tests of the rich-text sanitiser through its public API (editor.md §13.6).
//!
//! The unit tests and proptests of each part live next to the code in
//! `src/domain/rich_text/`. These tests treat the sanitiser as a black box,
//! as every write path will: the properties P1 (idempotence) and P5
//! (determinism across instances) over generated HTML, the runner of the
//! permanent regression fixtures under `tests/fixtures/regressions/`, the
//! snapshot of `css-vectors.jsonl`, and the admin-write validator and the
//! search text over the generated HTML and the fixtures (`validate`). The
//! fixture runner, the snapshot and the validator's fixture tests read files
//! and therefore cannot live in the pure `src/domain/` (ADR 0008).

mod css_vectors;
mod properties;
mod regressions;
mod validate;

use backend3::domain::rich_text::{SanitizePolicy, Sanitizer};

/// A sanitiser with the content-v1 policy. Every call builds a new instance,
/// with its own hash seeds, which is what P5 compares.
fn sanitizer() -> Sanitizer {
    Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
}
