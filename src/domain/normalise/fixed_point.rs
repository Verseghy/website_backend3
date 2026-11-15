//! N9: the fixed-point checks on a normalised body (data-migration.md §7.1,
//! §10 V13; ADR 0013 §10).
//!
//! Every body the import writes must be a fixed point three ways:
//! normalising it again, sanitising it again, and loading and saving it in
//! the admin editor each return it byte for byte. [`check_fixed_point`]
//! runs the first two, the ones the server can run; the editor round trip
//! runs in the admin's test suite over the normaliser's fixture outputs,
//! and over every body in the rehearsals (ADR 0013 §10, editor.md §13.4).
//!
//! A body that fails a check is not written: the import stops on it, and
//! its dry run lists it with the reason (data-migration.md §4.2, editor.md
//! §13.6 P7(f)). The errors say which check failed and from which byte the
//! bytes differ, and carry the report of the second run, which names what
//! that run would still change. Until N7 joins the normaliser, a legacy body
//! that holds markup outside content-v1 fails the sanitiser check, which is
//! what N7 is for; a content-v1 body passes both checks.

use super::{NormaliseError, NormaliseReport, normalise};
use crate::domain::rich_text::{RemovalReport, SanitizeError, Sanitizer};

/// Why a normalised body is not at its fixed point.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FixedPointError {
    /// Normalising the body again was refused.
    #[error("normalising the body again was refused: {0}")]
    NormaliseRefused(NormaliseError),
    /// Normalising the body again changed it: `normalise(out) != out`.
    #[error("normalising the body again changes it from byte {at}")]
    Renormalised {
        /// The byte offset of the first difference.
        at: usize,
        /// The report of the second run: the rules that changed it.
        report: NormaliseReport,
    },
    /// The sanitiser refused the body.
    #[error("the sanitiser refused the body: {0}")]
    SanitiseRefused(SanitizeError),
    /// Sanitising the body changed it: `S(out) != out`.
    #[error("sanitising the body changes it from byte {at}")]
    Resanitised {
        /// The byte offset of the first difference.
        at: usize,
        /// What the sanitiser removed or rewrote.
        report: RemovalReport,
    },
}

/// Checks that `out` is a fixed point of the normaliser and of the
/// sanitiser (N9).
///
/// - `out`: a body as the normaliser wrote it.
/// - `sanitizer`: the deployment's sanitiser, the one every write path
///   uses (ADR 0013 §5).
///
/// # Errors
///
/// The first check that fails, as a [`FixedPointError`]: the normaliser
/// check runs first.
pub fn check_fixed_point(out: &str, sanitizer: &Sanitizer) -> Result<(), FixedPointError> {
    let again = normalise(out).map_err(FixedPointError::NormaliseRefused)?;
    if again.html != out {
        return Err(FixedPointError::Renormalised {
            at: first_difference(out, &again.html),
            report: again.report,
        });
    }
    let result = sanitizer
        .sanitize(out)
        .map_err(FixedPointError::SanitiseRefused)?;
    if result.html.as_str() != out {
        return Err(FixedPointError::Resanitised {
            at: first_difference(out, result.html.as_str()),
            report: result.report,
        });
    }
    Ok(())
}

/// The byte offset of the first byte where `a` and `b` differ, or the
/// length of the shorter one when it is a prefix of the other.
///
/// - `a`, `b`: the two texts.
fn first_difference(a: &str, b: &str) -> usize {
    a.bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or_else(|| a.len().min(b.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::normalise::Rule;
    use crate::domain::rich_text::{RemovalRule, SanitizePolicy};
    use proptest::prelude::*;
    use std::fmt::Write as _;

    fn sanitizer() -> Sanitizer {
        Sanitizer::new(SanitizePolicy::content_v1()).expect("content-v1 is a valid policy")
    }

    #[test]
    fn a_content_v1_body_passes_both_checks() {
        let body = concat!(
            r#"<h2 style="text-align: center">Eredmények</h2>"#,
            r#"<p>Szöveg <strong>félkövér</strong> &amp; <a href="https://example.org/" target="_blank" rel="noopener noreferrer">link</a>&nbsp;…</p>"#,
            r#"<p><img src="/storage/uploads/posts/12/kep.jpg" alt="A díjazottak" width="350" height="495" style="display: block; height: auto; margin: 0 auto; max-width: 100%; width: 350px"></p>"#,
        );
        assert_eq!(check_fixed_point(body, &sanitizer()), Ok(()));
        assert_eq!(check_fixed_point("", &sanitizer()), Ok(()));
    }

    #[test]
    fn a_body_the_normaliser_still_changes_fails_the_first_check() {
        let result = check_fixed_point("<p>&eacute;</p><center>x</center>", &sanitizer());
        let Err(FixedPointError::Renormalised { at, report }) = result else {
            panic!("{result:?}");
        };
        assert_eq!(at, 3, "the reference");
        assert_eq!(report.count(Rule::CenterUnwrapped), 1);
    }

    #[test]
    fn a_body_outside_content_v1_fails_the_sanitiser_check() {
        let result = check_fixed_point(r#"<p class="x">a</p>"#, &sanitizer());
        let Err(FixedPointError::Resanitised { at, report }) = result else {
            panic!("{result:?}");
        };
        assert_eq!(at, 2);
        assert_eq!(report.count(RemovalRule::AttributeNotAllowed), 1);
        let error = FixedPointError::Resanitised {
            at,
            report: RemovalReport::default(),
        };
        assert_eq!(
            error.to_string(),
            "sanitising the body changes it from byte 2"
        );
    }

    #[test]
    fn refusals_are_passed_on() {
        let deep = "<div>".repeat(300);
        let result = check_fixed_point(&deep, &sanitizer());
        assert!(
            matches!(
                result,
                Err(FixedPointError::Renormalised { .. } | FixedPointError::SanitiseRefused(_))
            ),
            "{result:?}"
        );
        let deep = normalise(&deep)
            .expect("the normaliser has no depth limit")
            .html;
        let result = check_fixed_point(&deep, &sanitizer());
        let Err(FixedPointError::SanitiseRefused(error)) = result else {
            panic!("{result:?}");
        };
        assert_eq!(error.code(), Some("content_too_deep"));
        let result = check_fixed_point("<plaintext>x", &sanitizer());
        assert!(
            matches!(result, Err(FixedPointError::NormaliseRefused(_))),
            "{result:?}"
        );
        assert_eq!(
            FixedPointError::NormaliseRefused(NormaliseError::NotNormalisable { passes: 3 })
                .to_string(),
            "normalising the body again was refused: the body did not reach a fixed point in 3 normaliser passes"
        );
    }

    #[test]
    fn the_first_difference_is_found() {
        assert_eq!(first_difference("abc", "abc"), 3);
        assert_eq!(first_difference("abc", "abd"), 2);
        assert_eq!(first_difference("ab", "abc"), 2);
        assert_eq!(first_difference("", "a"), 0);
        assert_eq!(first_difference("é", "è"), 1, "a byte offset");
    }

    proptest! {
        #[test]
        fn the_first_difference_splits_a_common_prefix(common in "\\PC{0,10}", a in "\\PC{0,5}", b in "\\PC{0,5}") {
            let left = format!("{common}{a}");
            let right = format!("{common}{b}");
            let at = first_difference(&left, &right);
            prop_assert!(at >= common.len());
            prop_assert_eq!(&left.as_bytes()[..at], &right.as_bytes()[..at]);
            prop_assert!(at == left.len() || at == right.len() || left.as_bytes()[at] != right.as_bytes()[at]);
        }

        // Whatever the sanitiser writes and the normaliser keeps, N9
        // accepts: a body that went through both is at both fixed points.
        #[test]
        fn sanitised_and_normalised_bodies_pass(words in prop::collection::vec("[a-zé]{1,6}", 0..6), tags in prop::collection::vec(prop::sample::select(&["p", "h2", "strong", "em", "li", "center", "div", "b"][..]), 6)) {
            let sanitizer = sanitizer();
            let mut body = String::new();
            for (word, tag) in words.iter().zip(&tags) {
                let _ = write!(body, "<{tag}>{word}</{tag}>");
            }
            let clean = sanitizer.sanitize(&body).expect("a small body").html.into_string();
            let out = normalise(&clean).expect("sanitised markup settles").html;
            if sanitizer.sanitize(&out).expect("a small body").html.as_str() == out {
                prop_assert_eq!(check_fixed_point(&out, &sanitizer), Ok(()));
            }
        }
    }
}
