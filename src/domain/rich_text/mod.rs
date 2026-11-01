//! The server side of the rich-text pipeline: content-v1 and its sanitiser
//! (ADR 0013, docs/design/editor.md §3 and §7).
//!
//! Both public sites insert post and page bodies as trusted HTML, so the
//! server decides what readers' browsers run. Every body that backend3
//! stores is content-v1, one HTML dialect defined as data in [`policy`], and
//! the [`Sanitizer`] is the barrier that enforces it on every write path.
//!
//! The invariants the finished pipeline holds (editor.md §7):
//!
//! - **The allow-list equals editor output.** `S(e) == e` for every body the
//!   editor emits, so saving an unchanged body changes no byte.
//! - **Idempotence.** `S(S(x)) == S(x)`, byte for byte: the fixed-point loop
//!   returns only an output that one more pass leaves unchanged.
//! - **Determinism.** Two sanitiser instances produce the same bytes: the
//!   structural pass writes every attribute in canonical order, whatever
//!   order ammonia's hash maps left.
//! - **No forgery.** The content write service takes only
//!   [`SanitizedHtml`], which only [`Sanitizer::sanitize`] constructs.
//! - **Bounded work.** Input size and nesting depth are checked before any
//!   parse, and each pass's output size after it.
//!
//! This module holds the policy tables, the URL rules, the attribute filter,
//! the per-element CSS grammar with the canonical style string of §3.1 and
//! the cases of `css-vectors.jsonl` ([`css`]), the ammonia builder, the
//! structural pass with its tree-level style rules, the depth scan and the
//! removal report.
//!
//! Like everything under `src/domain/`, the module is pure (ADR 0008): the
//! policy comes in as an argument, built once at boot from the settings.
//!
//! ```
//! use backend3::domain::rich_text::{RemovalRule, SanitizePolicy, Sanitizer};
//!
//! let sanitizer = Sanitizer::new(SanitizePolicy::content_v1()).expect("a valid policy");
//! let result = sanitizer
//!     .sanitize(r#"<p onclick="x">Hello <img src="data:image/png;base64,AA" alt="kép"></p>"#)
//!     .expect("a small input");
//! assert_eq!(result.html.as_str(), "<p>Hello </p>");
//! assert_eq!(result.report.count(RemovalRule::ImageSrcRejected), 1);
//! assert_eq!(result.report.count(RemovalRule::ImageWithoutSrcRemoved), 1);
//! assert_eq!(result.report.count(RemovalRule::AttributeNotAllowed), 1);
//! ```

pub mod attrs;
pub mod css;
pub mod depth;
pub mod policy;
pub mod report;
pub mod sanitize;
pub mod structure;
pub mod url_rules;

pub use policy::{POLICY_VERSION, PolicyError, SanitizePolicy};
pub use report::{Removal, RemovalReport, RemovalRule};
pub use sanitize::{MAX_PASSES, SanitizeError, Sanitized, SanitizedHtml, Sanitizer};
pub use structure::Violation;
