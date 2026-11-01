//! The test oracle of the depth scan: html5ever's own tree builder, run
//! with a recorder between it and the tokenizer.
//!
//! The scan is only as good as its answers to the tokenizer (the parent
//! module explains why). This module drives the real tree builder over the
//! same input and records, at every token, what it told the tokenizer and
//! whether its current element was foreign. [`check`] compares that with the
//! scan's own trace. The tree builder runs twice, once with a tree that
//! treats an HTML-encoded `annotation-xml` as an integration point, as
//! ammonia's tree does, and once with one that does not, as scraper's own
//! tree does, so the scan's answers hold whichever of the two a parse builds
//! (both are [`IntegrationPointSink`], which can answer either way).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use html5ever::interface::{TreeSink, create_element};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::tree_builder::{TreeBuilder, TreeBuilderOpts};
use html5ever::{QualName, TokenizerResult, local_name, ns};
use scraper::HtmlTreeSink;

use super::super::report::IntegrationPointSink;

/// The tokenizer state a start tag leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Next {
    Data,
    Rcdata,
    Rawtext,
    ScriptData,
    Plaintext,
    Other,
}

/// One thing the tokenizer's sink decided, at a token index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Observation {
    /// The state a start tag left the tokenizer in.
    Start(usize, Next),
    /// Whether the current element is foreign after the token.
    Current(usize, bool),
    /// The answer to the tokenizer's CDATA question before the token.
    Cdata(usize, bool),
}

impl Observation {
    /// The observation for a start tag that answered `result`.
    pub(super) fn start<Handle>(tokens: usize, result: &TokenSinkResult<Handle>) -> Self {
        let next = match result {
            TokenSinkResult::Continue => Next::Data,
            TokenSinkResult::RawData(RawKind::Rcdata) => Next::Rcdata,
            TokenSinkResult::RawData(RawKind::Rawtext) => Next::Rawtext,
            TokenSinkResult::RawData(RawKind::ScriptData) => Next::ScriptData,
            TokenSinkResult::Plaintext => Next::Plaintext,
            _ => Next::Other,
        };
        Self::Start(tokens, next)
    }

    /// The key two traces are matched on: the token index and the kind.
    fn key(self) -> (usize, u8) {
        match self {
            Self::Start(index, _) => (index, 0),
            Self::Current(index, _) => (index, 1),
            Self::Cdata(index, _) => (index, 2),
        }
    }
}

/// Checks that every observation of the scan over `input` matches the tree
/// builder's, for both trees.
///
/// - `input`: the HTML fragment to compare on.
///
/// # Errors
///
/// A message naming the first observation that differs.
pub(super) fn check(input: &str) -> Result<(), String> {
    let model = super::scan(input, usize::MAX).trace;
    for annotation_points in [false, true] {
        let real: HashMap<_, _> = record(input, annotation_points)
            .0
            .into_iter()
            .map(|observation| (observation.key(), observation))
            .collect();
        for observation in &model {
            let expected = real.get(&observation.key());
            if expected != Some(observation) {
                return Err(format!(
                    "the scan said {observation:?}, the tree builder {expected:?} \
                     (annotation-xml integration points: {annotation_points})"
                ));
            }
        }
    }
    Ok(())
}

/// The number of start tags the tokenizer emits for `input` when the tree
/// builder drives it.
///
/// - `input`: the HTML fragment.
pub(super) fn start_tags(input: &str) -> usize {
    record(input, true).1
}

/// Runs the tree builder over `input` as `parse_fragment` does for a `div`
/// context, and returns its observations and the number of start tags.
fn record(input: &str, annotation_points: bool) -> (Vec<Observation>, usize) {
    let sink = IntegrationPointSink::new(annotation_points);
    let context = create_element(
        &sink,
        QualName::new(None, ns!(html), local_name!("div")),
        Vec::new(),
    );
    let builder = TreeBuilder::new_for_fragment(sink, context, None, TreeBuilderOpts::default());
    let opts = TokenizerOpts {
        initial_state: Some(builder.tokenizer_state_for_context_elem(false)),
        ..TokenizerOpts::default()
    };
    let recorder = Recorder {
        builder,
        cdata_possible: input.contains("<![CDATA["),
        trace: RefCell::new(Vec::new()),
        tokens: Cell::new(0),
        start_tags: Cell::new(0),
    };
    let tokenizer = Tokenizer::new(recorder, opts);
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(input));
    // The tree builder returns `Script` after `</script>`; a parser feeds
    // the rest of the input after it.
    while let TokenizerResult::Script(_) = tokenizer.feed(&queue) {}
    tokenizer.end();
    let recorder = tokenizer.sink;
    (recorder.trace.into_inner(), recorder.start_tags.get())
}

/// A token sink that forwards to the tree builder and records its answers.
struct Recorder {
    builder: TreeBuilder<<HtmlTreeSink as TreeSink>::Handle, IntegrationPointSink>,
    cdata_possible: bool,
    trace: RefCell<Vec<Observation>>,
    tokens: Cell<usize>,
    start_tags: Cell<usize>,
}

impl TokenSink for Recorder {
    type Handle = <HtmlTreeSink as TreeSink>::Handle;

    fn process_token(&self, token: Token, line_number: u64) -> TokenSinkResult<Self::Handle> {
        let start = matches!(&token, Token::TagToken(tag) if tag.kind == TagKind::StartTag);
        let end_of_input = matches!(token, Token::EOFToken);
        let result = self.builder.process_token(token, line_number);
        let index = self.tokens.get();
        let mut trace = self.trace.borrow_mut();
        if start {
            self.start_tags.set(self.start_tags.get() + 1);
            trace.push(Observation::start(index, &result));
        }
        if !end_of_input {
            trace.push(Observation::Current(
                index,
                self.builder
                    .adjusted_current_node_present_but_not_in_html_namespace(),
            ));
        }
        self.tokens.set(index + 1);
        result
    }

    fn end(&self) {
        self.builder.end();
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        let answer = self
            .builder
            .adjusted_current_node_present_but_not_in_html_namespace();
        if self.cdata_possible {
            self.trace
                .borrow_mut()
                .push(Observation::Cdata(self.tokens.get(), answer));
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observations(input: &str, annotation_points: bool) -> Vec<Observation> {
        record(input, annotation_points).0
    }

    #[test]
    fn the_recorder_sees_raw_text_and_cdata_decisions() {
        // `<style>` is raw text in HTML and an SVG element in `svg`.
        assert!(observations("<style>", false).contains(&Observation::Start(0, Next::Rawtext)));
        assert!(observations("<svg><style>", false).contains(&Observation::Start(1, Next::Data)));
        // CDATA only in foreign content.
        assert!(observations("<svg><![CDATA[x]]>", false).contains(&Observation::Cdata(1, true)));
        assert!(observations("<p><![CDATA[x]]>", false).contains(&Observation::Cdata(1, false)));
        assert!(
            observations("<svg><p><![CDATA[x]]>", false).contains(&Observation::Cdata(2, false))
        );
        // The CDATA question is recorded only when it can matter.
        assert!(
            !observations("<svg><!x>", false)
                .iter()
                .any(|o| matches!(o, Observation::Cdata(..)))
        );
    }

    #[test]
    fn the_two_trees_differ_only_at_an_html_annotation_xml() {
        let input = r#"<math><annotation-xml encoding="text/html"><style>"#;
        assert!(observations(input, true).contains(&Observation::Start(2, Next::Rawtext)));
        assert!(observations(input, false).contains(&Observation::Start(2, Next::Data)));
        let plain = "<math><annotation-xml><style>";
        assert_eq!(observations(plain, true), observations(plain, false));
    }

    #[test]
    fn check_finds_a_wrong_answer() {
        // A trace that claims HTML content inside `svg` is rejected.
        let real: HashMap<_, _> = observations("<svg><g>", false)
            .into_iter()
            .map(|o| (o.key(), o))
            .collect();
        assert_eq!(real.get(&(1, 1)), Some(&Observation::Current(1, true)));
        assert_ne!(real.get(&(1, 1)), Some(&Observation::Current(1, false)));
        assert_eq!(check("<svg><g><p><![CDATA[x]]><style>"), Ok(()));
    }

    #[test]
    fn start_tags_counts_what_the_tokenizer_emits() {
        assert_eq!(start_tags(""), 0);
        assert_eq!(start_tags("<p><b>x</b></p>"), 2);
        // Raw text and CDATA hide tags.
        assert_eq!(start_tags("<style><p><b></style>"), 1);
        assert_eq!(start_tags("<svg><![CDATA[<p>]]></svg>"), 1);
        assert_eq!(start_tags("<p><![CDATA[<b>]]>"), 1);
    }
}
