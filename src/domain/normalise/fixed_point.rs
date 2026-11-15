//! N9: the fixed-point checks on a normalised body (data-migration.md §7.1,
//! §10 V13; ADR 0013 §10).
//!
//! Every body the import writes must be a fixed point three ways:
//! normalising it again, sanitising it again, and loading and saving it in
//! the admin editor each return it byte for byte. [`super::normalise`]
//! itself ends with the first two for its own output: its loop stops only
//! at a pass that returns its input, and it then sanitises the output once
//! more. [`check_fixed_point`] runs both on a body from elsewhere, such as
//! a stored one that `content_normalize` reads. The editor round trip runs
//! in the admin's test suite over the normaliser's fixture outputs, and
//! over every body in the rehearsals (ADR 0013 §10, editor.md §13.4).
//!
//! A body that fails a check is not written: the import stops on it, and
//! its dry run lists it with the reason (data-migration.md §4.2, editor.md
//! §13.6 P7(f)). The errors say which check failed and from which byte the
//! bytes differ, and carry the report of the second run, which names what
//! that run would still change. Since N7 runs inside the normaliser, a body
//! outside content-v1 fails the first check: normalising it again
//! sanitises it.

use super::{NormaliseError, NormaliseReport, normalise};
use crate::domain::rich_text::Sanitizer;

/// Why a body is not at its fixed point.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FixedPointError {
    /// Normalising the body again was refused, by the normaliser's own
    /// passes, by the sanitiser of N7, or by N9's sanitiser check.
    #[error("normalising the body again was refused: {0}")]
    NormaliseRefused(NormaliseError),
    /// Normalising the body again changed it: `normalise(out) != out`. As
    /// the normaliser's output is the sanitiser's fixed point, this also
    /// covers a body that the sanitiser would change.
    #[error("normalising the body again changes it from byte {at}")]
    Renormalised {
        /// The byte offset of the first difference.
        at: usize,
        /// The report of the second run: the rules that changed it, and
        /// what N7 removed. Boxed, so that the error stays small beside
        /// the `Ok` of a fixed point.
        report: Box<NormaliseReport>,
    },
}

/// Checks that `out` is a fixed point of the normaliser and of the
/// sanitiser (N9).
///
/// - `out`: a body as the normaliser wrote it, or as it is stored.
/// - `sanitizer`: the deployment's sanitiser, the one every write path
///   uses (ADR 0013 §5).
///
/// # Errors
///
/// [`FixedPointError::NormaliseRefused`] when normalising `out` is refused,
/// and [`FixedPointError::Renormalised`] when it changes `out`. Normalising
/// ends with the sanitiser check `S(normalise(out)) == normalise(out)`, so
/// when `normalise(out) == out` holds, `S(out) == out` holds too.
pub fn check_fixed_point(out: &str, sanitizer: &Sanitizer) -> Result<(), FixedPointError> {
    let again = normalise(out, sanitizer).map_err(FixedPointError::NormaliseRefused)?;
    if again.html != out {
        return Err(FixedPointError::Renormalised {
            at: first_difference(out, &again.html),
            report: Box::new(again.report),
        });
    }
    Ok(())
}

/// The byte offset of the first byte where `a` and `b` differ, or the
/// length of the shorter one when it is a prefix of the other.
///
/// - `a`, `b`: the two texts.
pub(super) fn first_difference(a: &str, b: &str) -> usize {
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
    fn a_body_outside_content_v1_fails_and_names_what_n7_removes() {
        let result = check_fixed_point(r#"<p class="x">a</p>"#, &sanitizer());
        let Err(FixedPointError::Renormalised { at, report }) = result else {
            panic!("{result:?}");
        };
        assert_eq!(at, 2);
        let removals: Vec<_> = report
            .unplanned_removals()
            .map(|(removal, count)| (removal.rule, removal.to_string(), count))
            .collect();
        assert_eq!(
            removals,
            [(
                RemovalRule::AttributeNotAllowed,
                "attribute_not_allowed p[class]".to_owned(),
                1
            )]
        );
        let error = FixedPointError::Renormalised {
            at,
            report: Box::default(),
        };
        assert_eq!(
            error.to_string(),
            "normalising the body again changes it from byte 2"
        );
    }

    #[test]
    fn refusals_are_passed_on() {
        let deep = "<div>".repeat(300);
        let result = check_fixed_point(&deep, &sanitizer());
        let Err(FixedPointError::NormaliseRefused(NormaliseError::SanitiseRefused(error))) = result
        else {
            panic!("{result:?}");
        };
        assert_eq!(error.code(), Some("content_too_deep"));
        // The normaliser's own refusal: no known body still changes after
        // three passes now that N7 runs in them (`settle` is tested with a
        // pass that never settles), so only its message is checked here.
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

        // Whatever the normaliser writes, N9 accepts: its output is at both
        // fixed points.
        #[test]
        fn normalised_bodies_pass(words in prop::collection::vec("[a-zé]{1,6}", 0..6), tags in prop::collection::vec(prop::sample::select(&["p", "h2", "strong", "em", "li", "center", "div", "b"][..]), 6)) {
            let sanitizer = sanitizer();
            let mut body = String::new();
            for (word, tag) in words.iter().zip(&tags) {
                let _ = write!(body, "<{tag}>{word}</{tag}>");
            }
            let out = normalise(&body, &sanitizer).expect("simple markup settles").html;
            prop_assert_eq!(check_fixed_point(&out, &sanitizer), Ok(()));
        }
    }
}
