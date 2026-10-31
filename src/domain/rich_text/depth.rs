//! The nesting-depth bound of the sanitiser (editor.md §7.2; research:
//! DECISIONS R11).
//!
//! html5ever's tree builder searches its stack of open elements for most
//! start and end tags, so its cost grows with the product of the number of
//! tags and the nesting depth: 20,000 nested `div` take seconds, and a body
//! within the 2 MiB input limit can take minutes. The sanitiser parses every
//! body at least twice (ammonia, then the structural pass), so the depth is
//! bounded before either parse, by [`exceeds_depth`]: one pass of html5ever's
//! tokenizer, which is linear, with a model of the open elements that stays
//! linear too.
//!
//! # Why the model follows the tree builder
//!
//! The tokenizer does not decide its own state. After `<style>` it reads raw
//! text, and after `<![CDATA[` either a CDATA section or a bogus comment,
//! because the tree builder tells it so from the element it is in. A scan
//! that answers differently reads different tags from then on: text that the
//! parser reads as markup, or markup that the parser reads as text. The
//! second case is the dangerous one, because the scan then counts too little
//! and lets a deep body through. So the model answers every question the
//! tokenizer asks exactly as html5ever 0.40.1's tree builder would, or it
//! stops following the tree builder and falls back to a count that cannot be
//! too low (below).
//!
//! # HTML content
//!
//! The model is a stack of tag names. A start tag pushes its name, unless the
//! element is void (`br`, `img`, …). An end tag pops only when it names the
//! element on top of the stack, and every implied or misnested end tag is
//! ignored (except `</template>`, with which the tree builder closes the
//! nearest `template` and everything above it, whatever the scope, as the
//! model does), so the count errs on the high side: an unclosed `<p>` per
//! paragraph counts once each, and `</span>` does not close a `span` that
//! still holds an open `div`, as it does not in the tree builder either.
//! Like the tree builder, the scan switches the tokenizer into the raw-text
//! states after `script`, `style`, `textarea`, `title`, `iframe`, `xmp`,
//! `noembed`, `noframes`, `noscript` and `plaintext` (html5ever parses with
//! scripting enabled, as ammonia does, so `noscript` holds raw text), never
//! reads a CDATA section, and ignores `/>` on a non-void element. In HTML
//! content these answers depend only on the tag, not on the elements the
//! model does not track, so they are exact: in a table's insertion modes,
//! too, the raw-text tags go to the rules that switch the tokenizer. The one
//! exception is a `template` whose content starts with `col`: the tree
//! builder is then "in column group" with the `template` as its current
//! element, and ignores every start tag but `col` and `template`, raw-text
//! ones included, until `</template>`. The model tracks that mode on the
//! `template` entry.
//!
//! # SVG and `MathML` content
//!
//! Inside `svg` and `math` the answers depend on the namespace of the
//! current element, so each entry also records its namespace and whether it
//! is an integration point (`foreignObject`, `desc`, `title`, `mi`, `mo`,
//! `mn`, `ms`, `mtext`, `annotation-xml`). The model mirrors
//! html5ever's rules for foreign content (`TreeBuilder::is_foreign` and
//! `step_foreign`): an element is foreign unless an integration point lets
//! HTML in; a start tag on the HTML breakout list (`div`, `p`, `b`, `table`,
//! … and `font` with `color`, `face` or `size`) pops the foreign elements and
//! is handled as HTML; `/>` closes a foreign element at once; an end tag
//! closes the nearest foreign element of its name, through the ones above
//! it; and only a foreign current element lets the tokenizer read CDATA.
//!
//! The model loses track of the tree builder in a few places, which the
//! tree builder decides from state the model does not keep:
//!
//! - an end tag that the foreign rules hand back to the HTML rules, which
//!   may or may not close the foreign elements;
//! - an integration point that HTML content has entered: a table cell's
//!   tags, implied end tags and re-opened formatting elements can then put
//!   another element on top of it, or take it off the stack. The model loses
//!   track at the next CDATA there, at an end tag on the integration point,
//!   and at an end tag in the HTML content that names an open SVG or `MathML`
//!   element;
//! - `mglyph` and `malignmark`, which are `MathML` only directly in a `MathML`
//!   text integration point;
//! - a start tag in an `annotation-xml` whose `encoding` is HTML, because
//!   ammonia's tree treats that element as an HTML integration point and
//!   scraper's, which the removal report and the structural pass use, does
//!   not.
//!
//! From there on the scan stops modelling and counts every `<` followed by an
//! ASCII letter as an element that stays open, on top of the depth it had
//! reached. Every start tag the tokenizer emits begins with exactly those two
//! characters, whatever state it is in, so this count is never below the
//! tree's. It can refuse a body that is not deep, but only one with SVG or
//! `MathML` of an unusual shape and more tags after it than the limit allows.
//! content-v1 has neither, the editor never writes them, and the paste
//! filter removes them.
//!
//! Both counts that can be too high, for implied end tags and after the
//! switch, are refused with the one code `content_too_deep`. The maintainer
//! confirmed that as built on 2026-10-25 ("Confirm all four as built", item
//! D87, research: DECISIONS R15). The importer's normaliser parses each
//! legacy body before it calls the sanitiser and hands it closed markup
//! (data-migration.md §7.1, rules N0 and N7), so implied end tags never count
//! too high there; the import's dry run shows any legacy body refused for its
//! real depth, or after the switch, before anything is stored (editor.md
//! §14, R26).
//!
//! # The bound
//!
//! The tree can be deeper than the count only by the elements the tree
//! builder inserts without a tag of their own (`tbody` and `tr` around a bare
//! cell, re-opened formatting elements). A proptest over tag soup, heavy on
//! SVG, `MathML`, tables and CDATA, checks that the tree stays within twice
//! the count plus two, and a second one checks every answer the model gives
//! the tokenizer against html5ever's tree builder, for both trees (the
//! `oracle` test module). Bodies that the editor, the paste filter or the
//! importer's normaliser write close every element, and for them the count
//! is the tree's depth.

use std::cell::RefCell;
use std::collections::HashMap;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::{LocalName, local_name, ns};

#[cfg(test)]
mod oracle;

/// Where the nesting of an input first went deeper than the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthExceeded {
    /// The 1-based line of the start tag that went one level too deep, or,
    /// when the scan stopped following the parser, the line where it did.
    pub line: u64,
}

/// Scans `input` once and reports the first start tag that opens more than
/// `limit` elements at the same time, or `None` when the nesting stays within
/// `limit`.
///
/// - `input`: the HTML fragment the sanitiser is about to parse.
/// - `limit`: the deepest nesting allowed; an element directly in the
///   fragment has depth 1.
#[must_use]
pub fn exceeds_depth(input: &str, limit: usize) -> Option<DepthExceeded> {
    let state = scan(input, limit);
    if state.exceeded.is_some() {
        return state.exceeded;
    }
    let fallback = state.fallback?;
    // Every tag before the fallback point consumed one candidate of its own,
    // so the rest of the input holds at most this many start tags.
    let remaining = start_tag_candidates(input).saturating_sub(fallback.start_tags);
    (fallback.depth.saturating_add(remaining) > limit).then_some(DepthExceeded {
        line: fallback.line,
    })
}

/// Counts the places where a start tag can begin: a `<` followed by an
/// ASCII letter (the tokenizer's tag open state). Both are single bytes in
/// UTF-8, so the count over bytes is exact.
fn start_tag_candidates(input: &str) -> usize {
    input
        .as_bytes()
        .windows(2)
        .filter(|pair| pair[0] == b'<' && pair[1].is_ascii_alphabetic())
        .count()
}

/// Runs the tokenizer over `input` with the model as its sink and returns
/// the model's final state.
fn scan(input: &str, limit: usize) -> State {
    let sink = DepthSink {
        limit,
        state: RefCell::new(State {
            // Only an exact `<![CDATA[` can open a CDATA section; without
            // one, the tokenizer's CDATA question has no effect.
            cdata_possible: input.contains("<![CDATA["),
            ..State::default()
        }),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(input));
    // The sink never returns `Script`, so `feed` consumes the whole queue.
    let _ = tokenizer.feed(&queue);
    tokenizer.end();
    tokenizer.sink.state.into_inner()
}

/// The namespace of an open element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Space {
    Html,
    Svg,
    MathMl,
}

/// What kind of integration point a foreign element is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Point {
    /// Not an integration point.
    None,
    /// SVG `foreignObject`, `desc` and `title`: start tags in them are HTML.
    SvgHtml,
    /// `MathML` `mi`, `mo`, `mn`, `ms` and `mtext`: start tags other than
    /// `mglyph` and `malignmark` are HTML.
    MathText,
    /// `MathML` `annotation-xml` without an HTML `encoding`: only `<svg>` is
    /// HTML.
    Annotation,
    /// `MathML` `annotation-xml` whose `encoding` is `text/html` or
    /// `application/xhtml+xml`: an HTML integration point in ammonia's tree,
    /// a plain `MathML` element in scraper's.
    AnnotationHtml,
}

/// One open element of the model.
#[derive(Debug)]
struct Entry {
    /// The tag name as the tokenizer wrote it (ASCII lower case).
    name: LocalName,
    space: Space,
    point: Point,
    /// Set on an integration point once HTML content entered it. From then
    /// on the tree builder may have put HTML elements on top of it, or taken
    /// it off the stack, without a tag the model can see.
    entered: bool,
    /// For an HTML `template`, the insertion mode its content has reached.
    template: Template,
}

/// The insertion mode of a `template`'s content, as far as the model needs
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Template {
    /// Not a `template`.
    No,
    /// No start tag has chosen the mode yet ("in template").
    Open,
    /// `col` chose it: "in column group" with the `template` as the current
    /// element, where the tree builder ignores every start tag but `col`,
    /// `template` and `html`, raw-text ones included.
    Columns,
    /// Any other start tag chose it.
    Chosen,
}

impl Entry {
    fn html(name: LocalName) -> Self {
        let template = if name == local_name!("template") {
            Template::Open
        } else {
            Template::No
        };
        Self {
            name,
            space: Space::Html,
            point: Point::None,
            entered: false,
            template,
        }
    }

    fn foreign(name: LocalName, space: Space, point: Point) -> Self {
        Self {
            name,
            space,
            point,
            entered: false,
            template: Template::No,
        }
    }
}

/// Where the model stopped following the tree builder.
#[derive(Debug, Clone, Copy)]
struct Fallback {
    /// The line of the token where it happened.
    line: u64,
    /// The number of open elements at that point.
    depth: usize,
    /// The start tags the tokenizer had emitted before that point.
    start_tags: usize,
}

/// The open-element model of the scan.
#[derive(Debug, Default)]
struct State {
    /// The open elements, innermost last.
    open: Vec<Entry>,
    /// How many entries of `open` are not in the HTML namespace.
    foreign: usize,
    /// The names of those entries, with their counts, so that an end tag
    /// can be checked against them without a walk over the stack.
    foreign_names: HashMap<LocalName, usize>,
    /// How many entries of `open` are entered integration points.
    entered: usize,
    /// How many entries of `open` are HTML `template` elements.
    templates: usize,
    /// Whether the input contains `<![CDATA[`.
    cdata_possible: bool,
    /// The start tags seen so far.
    start_tags: usize,
    /// The line of the latest token.
    line: u64,
    /// Set when the model stops following the tree builder; later tokens
    /// are only tokenized.
    fallback: Option<Fallback>,
    /// Set by the first start tag that goes deeper than the limit. Later
    /// tokens are only tokenized, so the scan stays linear and the stack
    /// stops growing.
    exceeded: Option<DepthExceeded>,
    /// What the model told the tokenizer, for the oracle tests.
    #[cfg(test)]
    trace: Vec<oracle::Observation>,
    /// The tokens seen so far, for the oracle tests.
    #[cfg(test)]
    tokens: usize,
}

impl State {
    /// Whether the model still follows the tree builder.
    fn following(&self) -> bool {
        self.fallback.is_none() && self.exceeded.is_none()
    }

    /// Stops following the tree builder at the current token.
    fn lose_track(&mut self) {
        self.fallback = Some(Fallback {
            line: self.line,
            depth: self.open.len(),
            start_tags: self.start_tags,
        });
    }

    /// Pushes `entry`, and records the depth error when it is one level too
    /// many.
    fn push(&mut self, entry: Entry, limit: usize) {
        if entry.template != Template::No {
            self.templates += 1;
        }
        if entry.space != Space::Html {
            self.foreign += 1;
            *self.foreign_names.entry(entry.name.clone()).or_default() += 1;
        }
        self.open.push(entry);
        if self.open.len() > limit {
            self.exceeded = Some(DepthExceeded { line: self.line });
        }
    }

    /// Pops the innermost entry.
    fn pop(&mut self) {
        let Some(entry) = self.open.pop() else {
            return;
        };
        if entry.entered {
            self.entered -= 1;
        }
        if entry.template != Template::No {
            self.templates -= 1;
        }
        if entry.space != Space::Html {
            self.foreign -= 1;
            if let Some(count) = self.foreign_names.get_mut(&entry.name) {
                *count -= 1;
                if *count == 0 {
                    self.foreign_names.remove(&entry.name);
                }
            }
        }
    }

    /// Pops entries until `len` are left.
    fn truncate(&mut self, len: usize) {
        while self.open.len() > len {
            self.pop();
        }
    }

    /// Pops foreign entries until the innermost one is HTML or an SVG or
    /// `MathML` text integration point, as html5ever's
    /// `unexpected_start_tag_in_foreign_content` does. It pops an
    /// `annotation-xml` too, whatever its `encoding`.
    fn break_out(&mut self) {
        while self.open.last().is_some_and(|entry| {
            entry.space != Space::Html && !matches!(entry.point, Point::SvgHtml | Point::MathText)
        }) {
            self.pop();
        }
    }

    /// Whether the innermost element is foreign, if the model knows it. It
    /// does not after HTML content entered an integration point.
    fn current_is_foreign(&self) -> Option<bool> {
        match self.open.last() {
            None => Some(false),
            Some(top) if top.space == Space::Html => (self.foreign == 0).then_some(false),
            Some(top) => (!top.entered).then_some(true),
        }
    }
}

/// The token sink that drives the model.
struct DepthSink {
    limit: usize,
    state: RefCell<State>,
}

impl TokenSink for DepthSink {
    type Handle = ();

    fn process_token(&self, token: Token, line_number: u64) -> TokenSinkResult<()> {
        let mut state = self.state.borrow_mut();
        state.line = line_number;
        if !state.following() {
            return TokenSinkResult::Continue;
        }
        // The tree builder closes what is open at the end of the input; nothing
        // is read after it, so the oracle compares no state there.
        #[cfg(test)]
        let end_of_input = matches!(token, Token::EOFToken);
        let result = match token {
            Token::TagToken(tag) => match tag.kind {
                TagKind::StartTag => {
                    let result = start(&mut state, &tag, self.limit);
                    state.start_tags += 1;
                    #[cfg(test)]
                    if state.following() {
                        let tokens = state.tokens;
                        state
                            .trace
                            .push(oracle::Observation::start(tokens, &result));
                    }
                    result
                }
                TagKind::EndTag => {
                    end(&mut state, &tag.name);
                    TokenSinkResult::Continue
                }
            },
            // The tokenizer asserts that every token other than a start tag
            // answers `Continue`.
            _ => TokenSinkResult::Continue,
        };
        #[cfg(test)]
        {
            if state.following()
                && !end_of_input
                && let Some(foreign) = state.current_is_foreign()
            {
                let tokens = state.tokens;
                state
                    .trace
                    .push(oracle::Observation::Current(tokens, foreign));
            }
            state.tokens += 1;
        }
        result
    }

    // The tokenizer asks this at every `<!` that does not open a comment or
    // a doctype: a CDATA section exists only in foreign content, and
    // elsewhere `<![CDATA[` starts a bogus comment that ends at the first
    // `>`.
    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        let mut state = self.state.borrow_mut();
        if !state.following() {
            return false;
        }
        let answer = state.current_is_foreign();
        if answer.is_none() && state.cdata_possible {
            state.lose_track();
        }
        #[cfg(test)]
        if state.following() && state.cdata_possible {
            let tokens = state.tokens;
            state
                .trace
                .push(oracle::Observation::Cdata(tokens, answer.unwrap_or(false)));
        }
        answer.unwrap_or(false)
    }
}

/// Handles a start tag and says which tokenizer state its content needs.
fn start(state: &mut State, tag: &Tag, limit: usize) -> TokenSinkResult<()> {
    let name = &tag.name;
    // Which rules the tree builder applies (html5ever's `is_foreign`).
    let foreign_rules = match state.open.last() {
        None => false,
        Some(top) if top.space == Space::Html => {
            // HTML content inside an integration point: the tree builder's
            // current element may be the integration point itself.
            if state.foreign > 0 && is_math_glyph(name) {
                state.lose_track();
                return TokenSinkResult::Continue;
            }
            false
        }
        Some(top) => match top.point {
            Point::None => true,
            Point::MathText if is_math_glyph(name) => {
                if top.entered {
                    state.lose_track();
                    return TokenSinkResult::Continue;
                }
                true
            }
            Point::SvgHtml | Point::MathText => false,
            Point::Annotation => *name != local_name!("svg"),
            Point::AnnotationHtml if *name == local_name!("svg") => false,
            Point::AnnotationHtml => {
                state.lose_track();
                return TokenSinkResult::Continue;
            }
        },
    };
    if foreign_rules {
        if !breaks_out(tag) {
            foreign_start(state, tag, limit);
            return TokenSinkResult::Continue;
        }
        state.break_out();
    }
    html_start(state, tag, limit)
}

/// A start tag under the foreign rules that does not break out: a foreign
/// element in the namespace of the current one, closed at once by `/>`.
fn foreign_start(state: &mut State, tag: &Tag, limit: usize) {
    if tag.self_closing {
        return;
    }
    let space = state.open.last().map_or(Space::Html, |top| top.space);
    let point = match space {
        Space::Svg if matches!(&*tag.name, "foreignobject" | "desc" | "title") => Point::SvgHtml,
        Space::MathMl if matches!(&*tag.name, "mi" | "mo" | "mn" | "ms" | "mtext") => {
            Point::MathText
        }
        Space::MathMl if &*tag.name == "annotation-xml" => {
            if has_html_encoding(tag) {
                Point::AnnotationHtml
            } else {
                Point::Annotation
            }
        }
        _ => Point::None,
    };
    state.push(Entry::foreign(tag.name.clone(), space, point), limit);
}

/// A start tag under the HTML rules, which the tree builder applies in HTML
/// content and in an integration point.
fn html_start(state: &mut State, tag: &Tag, limit: usize) -> TokenSinkResult<()> {
    let name = &tag.name;
    if let Some(top) = state.open.last_mut() {
        match top.template {
            // "In column group" with the `template` as the current element
            // (html5ever's `InColumnGroup`, anything else): ignored, so not
            // even a raw-text tag switches the tokenizer.
            Template::Columns if *name != local_name!("template") => {
                return TokenSinkResult::Continue;
            }
            // "In template": the head tags keep the mode, `col` chooses "in
            // column group", every other start tag another mode.
            Template::Open if !is_head_tag(name) => {
                top.template = if *name == local_name!("col") {
                    Template::Columns
                } else {
                    Template::Chosen
                };
            }
            _ => {}
        }
    }
    let at_point = state
        .open
        .last()
        .is_some_and(|top| top.space != Space::Html);
    // In a table's insertion modes these tags close the open cell, row or
    // section, and with it every element inside it, the integration point
    // included.
    if at_point && is_table_part(name) {
        enter_point(state);
    }
    if matches!(*name, local_name!("svg") | local_name!("math")) {
        if !tag.self_closing {
            let space = if *name == local_name!("svg") {
                Space::Svg
            } else {
                Space::MathMl
            };
            state.push(Entry::foreign(name.clone(), space, Point::None), limit);
        }
        return TokenSinkResult::Continue;
    }
    // An HTML parser ignores `/>` on a non-void HTML element.
    if is_void(name) {
        return TokenSinkResult::Continue;
    }
    if at_point {
        enter_point(state);
    }
    state.push(Entry::html(name.clone()), limit);
    if state.exceeded.is_some() {
        return TokenSinkResult::Continue;
    }
    match *name {
        local_name!("title") | local_name!("textarea") => TokenSinkResult::RawData(RawKind::Rcdata),
        local_name!("style")
        | local_name!("xmp")
        | local_name!("iframe")
        | local_name!("noembed")
        | local_name!("noframes")
        | local_name!("noscript") => TokenSinkResult::RawData(RawKind::Rawtext),
        local_name!("script") => TokenSinkResult::RawData(RawKind::ScriptData),
        local_name!("plaintext") => TokenSinkResult::Plaintext,
        _ => TokenSinkResult::Continue,
    }
}

/// Marks the innermost entry, an integration point, as entered by HTML.
fn enter_point(state: &mut State) {
    if let Some(top) = state.open.last_mut()
        && !top.entered
    {
        top.entered = true;
        state.entered += 1;
    }
}

/// Handles an end tag.
fn end(state: &mut State, name: &LocalName) {
    let Some(top) = state.open.last() else {
        return;
    };
    if top.space == Space::Html {
        // The HTML rules. Inside an integration point the tree builder's
        // current element may be the integration point, whose end tag rules
        // close the nearest foreign element of this name.
        if state.foreign > 0 && state.foreign_names.contains_key(name) {
            state.lose_track();
        } else if *name == local_name!("template") {
            // The "in head" rules close the nearest `template` and everything
            // above it, whatever the scope. Following them keeps the model's
            // `template` entries those of the tree builder, so a `template`
            // that is "in column group" is the current element in both.
            if state.templates > 0
                && let Some(index) = state
                    .open
                    .iter()
                    .rposition(|entry| entry.template != Template::No)
            {
                state.truncate(index);
            }
        } else if top.name == *name {
            state.pop();
        }
        return;
    }
    if top.entered {
        state.lose_track();
        return;
    }
    // `</br>` and `</p>` break out of foreign content like start tags.
    if matches!(*name, local_name!("br") | local_name!("p")) {
        state.break_out();
        if state
            .open
            .last()
            .is_some_and(|top| top.space == Space::Html && top.name == *name)
        {
            state.pop();
        }
        return;
    }
    // html5ever's `step_foreign` for any other end tag: walk down from the
    // current element to the nearest one of this name and close everything
    // from there, unless an HTML element comes first, which hands the tag to
    // the HTML rules.
    if !state.foreign_names.contains_key(name) {
        // No foreign element of this name: the walk ends at the first HTML
        // element, where the HTML rules take the tag, or at the root, where
        // it is ignored. Above an entered integration point the tree builder
        // may have an HTML element the model does not see.
        if state.foreign < state.open.len() || state.entered > 0 {
            state.lose_track();
        }
        return;
    }
    for index in (0..state.open.len()).rev() {
        let entry = &state.open[index];
        let first = index + 1 == state.open.len();
        // An entered integration point may have HTML elements on top of it
        // in the tree builder, which would end the walk there.
        if !first && (entry.space == Space::Html || entry.entered) {
            state.lose_track();
            return;
        }
        if entry.name == *name {
            state.truncate(index);
            return;
        }
    }
}

/// `mglyph` and `malignmark`, the two start tags that stay `MathML` in a
/// `MathML` text integration point.
fn is_math_glyph(name: &LocalName) -> bool {
    matches!(*name, local_name!("mglyph") | local_name!("malignmark"))
}

/// The start tags that "in template" hands to the "in head" rules, which
/// leave the `template`'s mode unchosen.
fn is_head_tag(name: &LocalName) -> bool {
    matches!(
        *name,
        local_name!("base")
            | local_name!("basefont")
            | local_name!("bgsound")
            | local_name!("link")
            | local_name!("meta")
            | local_name!("noframes")
            | local_name!("script")
            | local_name!("style")
            | local_name!("template")
            | local_name!("title")
    )
}

/// Whether a start tag in foreign content goes back to HTML (html5ever's
/// `step_foreign`).
fn breaks_out(tag: &Tag) -> bool {
    match tag.name {
        local_name!("b")
        | local_name!("big")
        | local_name!("blockquote")
        | local_name!("body")
        | local_name!("br")
        | local_name!("center")
        | local_name!("code")
        | local_name!("dd")
        | local_name!("div")
        | local_name!("dl")
        | local_name!("dt")
        | local_name!("em")
        | local_name!("embed")
        | local_name!("h1")
        | local_name!("h2")
        | local_name!("h3")
        | local_name!("h4")
        | local_name!("h5")
        | local_name!("h6")
        | local_name!("head")
        | local_name!("hr")
        | local_name!("i")
        | local_name!("img")
        | local_name!("li")
        | local_name!("listing")
        | local_name!("menu")
        | local_name!("meta")
        | local_name!("nobr")
        | local_name!("ol")
        | local_name!("p")
        | local_name!("pre")
        | local_name!("ruby")
        | local_name!("s")
        | local_name!("small")
        | local_name!("span")
        | local_name!("strong")
        | local_name!("strike")
        | local_name!("sub")
        | local_name!("sup")
        | local_name!("table")
        | local_name!("tt")
        | local_name!("u")
        | local_name!("ul")
        | local_name!("var") => true,
        local_name!("font") => tag.attrs.iter().any(|attribute| {
            attribute.name.ns == ns!()
                && matches!(
                    attribute.name.local,
                    local_name!("color") | local_name!("face") | local_name!("size")
                )
        }),
        _ => false,
    }
}

/// Whether an `annotation-xml` start tag makes an HTML integration point
/// (markup5ever's `create_element_with_flags`).
fn has_html_encoding(tag: &Tag) -> bool {
    tag.attrs.iter().any(|attribute| {
        attribute.name.ns == ns!()
            && attribute.name.local == local_name!("encoding")
            && (attribute.value.eq_ignore_ascii_case("text/html")
                || attribute
                    .value
                    .eq_ignore_ascii_case("application/xhtml+xml"))
    })
}

/// The table tags whose rules in a table's insertion modes close open cells,
/// rows, sections or captions.
fn is_table_part(name: &LocalName) -> bool {
    matches!(
        *name,
        local_name!("caption")
            | local_name!("col")
            | local_name!("colgroup")
            | local_name!("table")
            | local_name!("tbody")
            | local_name!("td")
            | local_name!("tfoot")
            | local_name!("th")
            | local_name!("thead")
            | local_name!("tr")
    )
}

/// The void elements of the HTML standard plus the legacy ones html5ever
/// still treats as void; the tree builder never keeps them open.
fn is_void(name: &LocalName) -> bool {
    matches!(
        *name,
        local_name!("area")
            | local_name!("base")
            | local_name!("basefont")
            | local_name!("bgsound")
            | local_name!("br")
            | local_name!("col")
            | local_name!("embed")
            | local_name!("frame")
            | local_name!("hr")
            | local_name!("image")
            | local_name!("img")
            | local_name!("input")
            | local_name!("keygen")
            | local_name!("link")
            | local_name!("meta")
            | local_name!("param")
            | local_name!("source")
            | local_name!("track")
            | local_name!("wbr")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::report::parse_like_ammonia;
    use proptest::prelude::*;

    fn nested(tag: &str, depth: usize) -> String {
        format!(
            "{}x{}",
            format!("<{tag}>").repeat(depth),
            format!("</{tag}>").repeat(depth)
        )
    }

    /// The depth of the deepest element of html5ever's tree for `input`,
    /// found with an explicit stack so a deep tree cannot overflow it.
    pub(super) fn tree_depth(input: &str) -> usize {
        let html = parse_like_ammonia(input);
        let mut deepest = 0;
        // The children of the fragment's `html` root are at depth 1.
        let mut stack: Vec<_> = html.root_element().children().map(|n| (n, 1)).collect();
        while let Some((node, depth)) = stack.pop() {
            if node.value().is_element() {
                deepest = deepest.max(depth);
                stack.extend(node.children().map(|child| (child, depth + 1)));
            }
        }
        deepest
    }

    /// The smallest limit the scan accepts `input` under. The scan is
    /// monotone in the limit, so a binary search finds it.
    fn measured(input: &str) -> usize {
        let (mut low, mut high) = (0, input.len() + 1);
        while low < high {
            let middle = usize::midpoint(low, high);
            if exceeds_depth(input, middle).is_none() {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        low
    }

    #[test]
    fn nesting_up_to_the_limit_passes_and_one_more_level_fails() {
        for tag in ["div", "blockquote", "span", "li", "td", "b"] {
            assert_eq!(exceeds_depth(&nested(tag, 256), 256), None, "{tag}");
            assert_eq!(
                exceeds_depth(&nested(tag, 257), 256),
                Some(DepthExceeded { line: 1 }),
                "{tag}"
            );
        }
        assert_eq!(exceeds_depth("", 1), None);
        assert_eq!(exceeds_depth("text only", 1), None);
        assert_eq!(exceeds_depth("<p>x</p>", 1), None);
        assert!(exceeds_depth("<p><b>x</b></p>", 1).is_some());
    }

    #[test]
    fn the_line_of_the_first_tag_too_deep_is_reported() {
        let input = "<div>\n<div>\n\n<div>x</div></div></div>";
        assert_eq!(exceeds_depth(input, 2), Some(DepthExceeded { line: 4 }));
        assert_eq!(exceeds_depth(input, 3), None);
    }

    #[test]
    fn closed_siblings_and_void_elements_do_not_add_up() {
        assert_eq!(exceeds_depth(&"<p>a<br>b</p>".repeat(10_000), 32), None);
        assert_eq!(
            exceeds_depth(
                &"<img src=x><hr><br/><wbr><input><image><col><embed>".repeat(1_000),
                32
            ),
            None
        );
        assert_eq!(
            exceeds_depth(
                &"<table><tbody><tr><td>x</td></tr></tbody></table>".repeat(1_000),
                32
            ),
            None
        );
    }

    #[test]
    fn raw_text_is_not_markup() {
        let soup = "<div>".repeat(1_000);
        for tag in [
            "script", "style", "textarea", "title", "iframe", "xmp", "noembed", "noframes",
            "noscript",
        ] {
            assert_eq!(
                exceeds_depth(&format!("<{tag}>{soup}</{tag}>"), 32),
                None,
                "{tag}"
            );
            // The raw text ends at the element's own end tag only.
            assert!(
                exceeds_depth(&format!("<{tag}>x</{tag}>{soup}"), 32).is_some(),
                "{tag}"
            );
        }
        assert_eq!(exceeds_depth(&format!("<plaintext>{soup}"), 32), None);
        // Inside `svg`, `style` and `title` are SVG elements with markup.
        assert!(exceeds_depth(&format!("<svg><style>{soup}</style></svg>"), 32).is_some());
        assert!(exceeds_depth(&format!("<svg><title>{soup}</title></svg>"), 32).is_some());
        // In an integration point they are HTML again: raw text.
        assert_eq!(
            exceeds_depth(&format!("<svg><foreignObject><style>{soup}</style>"), 32),
            None
        );
        assert_eq!(
            exceeds_depth(&format!("<math><mi><textarea>{soup}</textarea>"), 32),
            None
        );
    }

    #[test]
    fn self_closing_counts_only_in_html_content() {
        assert_eq!(
            exceeds_depth(&format!("<svg>{}</svg>", "<path/>".repeat(1_000)), 32),
            None
        );
        assert_eq!(
            exceeds_depth(&format!("<math>{}</math>", "<mspace/>".repeat(1_000)), 32),
            None
        );
        assert_eq!(exceeds_depth(&"<math/>".repeat(1_000), 32), None);
        assert_eq!(exceeds_depth(&"<svg/>".repeat(1_000), 32), None);
        // An HTML parser ignores `/>` on a non-void HTML element.
        assert!(exceeds_depth(&"<div/>".repeat(40), 32).is_some());
        // Inside `svg`, void HTML names are SVG elements that stay open.
        assert!(exceeds_depth(&format!("<svg>{}", "<input>".repeat(40)), 32).is_some());
        // In an integration point, start tags are HTML.
        assert!(exceeds_depth(&format!("<svg><foreignObject>{}", "<x/>".repeat(40)), 32).is_some());
        assert!(exceeds_depth(&format!("<math><mi>{}", "<x/>".repeat(40)), 32).is_some());
        // `mglyph` stays MathML directly in `mi`.
        assert_eq!(
            exceeds_depth(&format!("<math><mi>{}", "<mglyph/>".repeat(1_000)), 32),
            None
        );
    }

    #[test]
    fn breakout_tags_leave_foreign_content() {
        // These start tags pop `svg` or `math` and open HTML elements, which
        // ignore `/>`.
        for root in ["<svg>", "<math>", "<svg><g>", "<math><mrow>", "<svg><p>"] {
            for tag in ["div", "b", "span", "sub", "ul", "blockquote", "em", "pre"] {
                let input = format!("{root}{}", format!("<{tag}/>").repeat(300));
                assert!(tree_depth(&input) > 256, "{input:.40}");
                assert!(exceeds_depth(&input, 256).is_some(), "{input:.40}");
            }
        }
        // `font` breaks out only with `color`, `face` or `size`.
        assert!(exceeds_depth(&format!("<svg>{}", "<font color=red/>".repeat(40)), 32).is_some());
        assert_eq!(
            exceeds_depth(&format!("<svg>{}", "<font/>".repeat(1_000)), 32),
            None
        );
        // `</p>` and `</br>` break out too.
        assert!(exceeds_depth(&format!("<svg></p>{}", "<x/>".repeat(40)), 32).is_some());
        assert!(exceeds_depth(&format!("<svg></br>{}", "<x/>".repeat(40)), 32).is_some());
    }

    #[test]
    fn cdata_is_read_only_in_foreign_content() {
        let soup = "<div>".repeat(1_000);
        // In `svg`, CDATA is text up to `]]>`.
        assert_eq!(
            exceeds_depth(&format!("<svg><![CDATA[{soup}]]></svg>"), 32),
            None
        );
        // In HTML content, `<![CDATA[` starts a comment that ends at `>`.
        for prefix in [
            "",
            "<p>",
            "<svg><p>",
            "<svg></p>",
            "<svg><g></svg>",
            "<svg></svg>",
            "<math><mi></mi></math>",
            "<svg><div/>",
        ] {
            let input = format!("{prefix}<![CDATA[>{soup}");
            assert!(tree_depth(&input) > 256, "{prefix}");
            assert!(exceeds_depth(&input, 256).is_some(), "{prefix}");
        }
    }

    #[test]
    fn end_tags_close_foreign_elements_like_the_tree_builder() {
        let soup = "<div>".repeat(1_000);
        // `</svg>` closes everything above the `svg`, and the HTML rules
        // apply again: `<style>` is raw text.
        let input = format!("<svg><g><circle></svg><style>{soup}");
        assert_eq!(exceeds_depth(&input, 32), None);
        assert!(scan(&input, 32).fallback.is_none());
        // A stray end tag inside `svg` closes nothing.
        assert!(exceeds_depth(&format!("<svg></x></y>{}", "<g>".repeat(40)), 32).is_some());
        assert_eq!(
            exceeds_depth(&format!("<svg></x></y>{}", "<g/>".repeat(1_000)), 32),
            None
        );
    }

    #[test]
    fn well_formed_svg_and_mathml_are_followed_to_the_end() {
        for body in [
            r#"<p><svg viewBox="0 0 1 1"><title>Logo</title><desc>A dot</desc><g><path d="M0 0"/><circle r="1"></circle></g></svg> text</p>"#,
            "<p><math><mrow><mi>x</mi><mo>+</mo><mn>1</mn></mrow></math></p>",
            "<svg><![CDATA[a < b]]><text>x</text></svg><p>after</p>",
            "<math><annotation-xml><svg><g></g></svg></annotation-xml></math>",
            "<div><svg></svg></div><svg/><math/>",
        ] {
            let input = body.repeat(2_000);
            let state = scan(&input, 8);
            assert!(state.fallback.is_none(), "{body}");
            assert_eq!(exceeds_depth(&input, 8), None, "{body}");
        }
    }

    #[test]
    fn the_scan_counts_every_later_tag_where_it_cannot_follow_the_parser() {
        // The shapes where the tree builder's state depends on what the scan
        // does not model. Each refuses an input that is not deep only when
        // more tags follow than the limit allows.
        for prefix in [
            // `</div>` handed from the SVG rules to the HTML rules.
            "<div><svg></div>",
            // An end tag on an integration point that HTML content entered.
            "<svg><foreignObject><p>x</p></foreignObject>",
            // CDATA in HTML content inside an integration point.
            "<svg><foreignObject><p><![CDATA[x]]>",
            // A table cell's tags can close the integration point.
            "<table><tr><td><svg><foreignObject><col><![CDATA[x]]>",
            // `mglyph` above a MathML text integration point.
            "<math><mi><p><b></p><mglyph>",
            // ammonia's and scraper's trees differ here.
            r#"<math><annotation-xml encoding="text/html"><div>"#,
        ] {
            let state = scan(prefix, 1_000);
            let fallback = state.fallback.expect(prefix);
            assert_eq!(fallback.line, 1, "{prefix}");
            let few = format!("{prefix}{}", "<p>x</p>".repeat(10));
            assert_eq!(exceeds_depth(&few, 32), None, "{prefix}");
            let many = format!("{prefix}{}", "<p>x</p>".repeat(40));
            assert_eq!(
                exceeds_depth(&many, 32),
                Some(DepthExceeded { line: 1 }),
                "{prefix}"
            );
        }
    }

    #[test]
    fn deep_bodies_that_hide_from_a_simpler_scan_are_refused() {
        // Each of these opens thousands of nested HTML elements in
        // html5ever's tree.
        let n = 2_000;
        let divs = "<div>".repeat(n);
        for input in [
            format!("<svg>{}", "<div/>".repeat(n)),
            format!("<math>{}", "<div/>".repeat(n)),
            format!("<svg><p>{}", "<div/>".repeat(n)),
            format!("<svg><p><![CDATA[>{divs}"),
            format!("<svg></p><![CDATA[>{divs}"),
            format!("<svg><g></svg><![CDATA[>{divs}"),
            format!("<div><svg></div><![CDATA[>{divs}"),
            format!("<table><svg></table>{}", "<x/>".repeat(n)),
            format!("<table><tr><td><svg><foreignObject><col><![CDATA[>{divs}"),
            format!("<svg><foreignObject><p><b></p></b></p>x<![CDATA[>{divs}"),
            format!("<svg><foreignObject><p><b></p><![CDATA[><!--]]>{divs}"),
            format!("<math><mi><p><b></p><mglyph><style>{divs}"),
            format!(r#"<math><annotation-xml encoding="text/html"><style>{divs}"#),
            format!("<template><col><style></template>{divs}"),
            format!("<svg><foreignObject><p><b></p></b></p>x<svg></b><style><!--</style>{divs}"),
        ] {
            assert!(tree_depth(&input) >= n, "{input:.60}");
            assert!(exceeds_depth(&input, 256).is_some(), "{input:.60}");
        }
    }

    #[test]
    fn a_template_that_starts_with_col_ignores_raw_text_tags() {
        let soup = "<div>".repeat(1_000);
        // "In column group" with the `template` as the current element: the
        // tree builder ignores `<style>`, so the `div`s after `</template>`
        // are markup.
        for prefix in [
            "<template><col>",
            "<template>x<col>",
            "<template><style></style><col>",
        ] {
            let input = format!("{prefix}<style></template>{soup}");
            assert!(tree_depth(&input) >= 1_000, "{prefix}");
            assert!(exceeds_depth(&input, 32).is_some(), "{prefix}");
        }
        // Any other first tag chooses a mode in which `<style>` is raw text.
        for prefix in ["<template><div></div><col>", "<template><tr><col>", "<col>"] {
            let input = format!("{prefix}<style></template>{soup}");
            assert_eq!(exceeds_depth(&input, 32), None, "{prefix}");
        }
        // A `template` inside starts over.
        let nested = format!("<template><col><template><style>{soup}");
        assert_eq!(exceeds_depth(&nested, 32), None);
        // `</template>` closes the inner `template` with what is open in it,
        // and the outer one is "in column group" again.
        for inner in ["<div><b>", "<foreignObject>", "<svg><div>"] {
            let input =
                format!("<template><col><template>{inner}</template><style></template>{soup}");
            assert!(tree_depth(&input) >= 1_000, "{inner}");
            assert!(exceeds_depth(&input, 32).is_some(), "{inner}");
        }
    }

    #[test]
    fn the_oracle_agrees_on_the_hard_cases() {
        for input in [
            "<svg><div/><p><![CDATA[x]]><style>",
            "<svg><g></svg><![CDATA[x]]>",
            "<svg></p><![CDATA[x]]><svg><![CDATA[y]]>",
            "<table><svg></table><x/><![CDATA[x]]>",
            "<table><tr><td><svg><foreignObject><col><![CDATA[x]]>",
            "<svg><foreignObject><p><b></p></b></p>x<![CDATA[y]]>",
            "<math><mi><p><b></p><mglyph><style>",
            "<math><mi><mglyph><style></style></mi><![CDATA[x]]>",
            "<math><annotation-xml><svg><foreignObject><p>x",
            r#"<math><annotation-xml encoding="text/html"><style>"#,
            "<template><col><svg><div><style>",
            "<template><col><template><style></style></template><style>",
            "<svg><font color=red/><font/><![CDATA[x]]>",
            "<select><svg><option><![CDATA[x]]>",
            "<p><svg></p><style>x</style>",
        ] {
            assert_eq!(oracle::check(input), Ok(()), "{input}");
        }
    }

    #[test]
    fn misnested_end_tags_never_lower_the_count() {
        // The tree builder ignores `</span>` while a `div` is open inside
        // it, so the tree is 400 levels deep; the model counts all of them.
        assert!(exceeds_depth(&"<span><div></span>".repeat(200), 256).is_some());
        // Implied end tags are not modelled, so unclosed paragraphs count.
        assert!(exceeds_depth(&"<p>x".repeat(300), 256).is_some());
        assert!(exceeds_depth(&"</div>".repeat(300), 1).is_none());
    }

    #[test]
    fn start_tag_candidates_are_a_less_than_sign_and_a_letter() {
        assert_eq!(start_tag_candidates(""), 0);
        assert_eq!(start_tag_candidates("<"), 0);
        assert_eq!(start_tag_candidates("<a><B></c><1><-<é<!x<?x"), 2);
        assert_eq!(start_tag_candidates("a<b<c"), 2);
    }

    /// Start tags, with attributes where they change the parse, for the
    /// bound proptests: foreign roots and integration points, the breakout
    /// tags, raw-text, table, template and formatting tags, and tags the
    /// tree builder ignores or closes implicitly.
    #[rustfmt::skip]
    const SOUP_TAGS: &[&str] = &[
        "svg", "math", "foreignObject", "desc", "title", "mi", "mo", "ms", "mtext", "mglyph",
        "malignmark", "annotation-xml", "annotation-xml encoding=text/html",
        "annotation-xml encoding=application/xhtml+xml", "annotation-xml encoding=TEXT/HTML",
        "g", "path", "div", "p", "b", "i", "a", "nobr", "span", "font", "font color=red",
        "font size=3", "h1", "h2", "li", "dd", "ul", "sub", "pre", "listing", "ruby", "rt", "br",
        "hr", "img", "input", "input type=hidden", "button", "form", "select", "option",
        "optgroup", "table", "caption", "colgroup", "col", "thead", "tbody", "tr", "td", "th",
        "template", "template shadowrootmode=open", "style", "script", "textarea", "xmp",
        "iframe", "noscript", "noframes", "noembed", "plaintext", "object", "marquee",
        "frameset", "body", "html", "head", "x",
    ];

    /// Text for the bound proptests: CDATA and comment openers and closers,
    /// and characters that start or end a tag or an attribute value.
    #[rustfmt::skip]
    const SOUP_TEXTS: &[&str] = &[
        "x", " ", "\n", "<![CDATA[", "<![CDATA[>", "]]>", "<!--", "-->", "<!x>", ">", "<", "\"",
        "<a x=\"", "&lt;",
    ];

    /// Foreign roots, and integration points in them, that start a run.
    #[rustfmt::skip]
    const RUN_ROOTS: &[&str] = &[
        "<svg>", "<math>", "<svg><g>", "<math><mrow>", "<svg><foreignObject>", "<math><mi>",
        "<math><annotation-xml>", "<svg><p>", "<svg></p>", "<svg><g></svg>",
    ];

    /// The tags a run repeats after its root.
    #[rustfmt::skip]
    const RUN_TAGS: &[&str] = &[
        "div", "p", "b", "span", "table", "font color=red", "font", "sub", "x", "path", "td",
        "![CDATA[",
    ];

    /// One piece of the tag soup for the bound proptests, sometimes
    /// repeated, so that a scan that loses count shows it as depth.
    fn arb_part() -> impl Strategy<Value = String> {
        let tags = prop::sample::select(SOUP_TAGS);
        // A foreign root followed by a run of one tag: the shape in which a
        // scan that misreads foreign content loses whole runs.
        let foreign_run = (
            prop::sample::select(RUN_ROOTS),
            prop::sample::select(RUN_TAGS),
            any::<bool>(),
            1usize..30,
        )
            .prop_map(|(root, tag, closing, times)| {
                let tag = if tag.starts_with('!') {
                    format!("<{tag}>")
                } else {
                    format!("<{tag}{}>", if closing { "/" } else { "" })
                };
                format!("{root}{}", tag.repeat(times))
            });
        let one = prop_oneof![
            5 => (tags.clone(), any::<bool>()).prop_map(|(tag, closing)| {
                format!("<{tag}{}>", if closing { "/" } else { "" })
            }),
            3 => tags.prop_map(|tag| {
                format!("</{}>", tag.split(' ').next().unwrap_or(tag))
            }),
            2 => prop::sample::select(SOUP_TEXTS).prop_map(str::to_owned),
        ];
        prop_oneof![
            5 => (one, prop_oneof![4 => Just(1usize), 1 => 2usize..40])
                .prop_map(|(part, times)| part.repeat(times)),
            1 => foreign_run,
        ]
    }

    proptest! {
        #[test]
        fn the_scan_never_panics(input in any::<String>(), limit in 0usize..8) {
            let _ = exceeds_depth(&input, limit);
        }

        // For markup that closes every element, the scan sees exactly the
        // tree's depth.
        #[test]
        fn closed_markup_is_measured_exactly(tags in prop::collection::vec(prop::sample::select(&["div", "span", "blockquote", "em", "ul", "ol", "li", "h2"][..]), 0..12)) {
            let input = format!(
                "{}x{}",
                tags.iter().map(|t| format!("<{t}>")).collect::<Vec<_>>().concat(),
                tags.iter().rev().map(|t| format!("</{t}>")).collect::<Vec<_>>().concat(),
            );
            let depth = tags.len();
            prop_assert_eq!(exceeds_depth(&input, depth), None);
            if depth > 0 {
                prop_assert!(exceeds_depth(&input, depth - 1).is_some());
            }
        }

        // For tag soup, html5ever's tree stays within twice the scan's count,
        // plus the two elements a bare cell implies.
        #[test]
        fn the_tree_is_never_much_deeper_than_the_scan(parts in prop::collection::vec(arb_part(), 0..60)) {
            let input = parts.concat();
            let (tree, count) = (tree_depth(&input), measured(&input));
            prop_assert!(tree <= 2 * count + 2, "{:?}: tree {} count {}", input, tree, count);
        }

        // Every answer the model gives the tokenizer, while it follows the
        // parser, is the answer html5ever's tree builder gives, for both
        // ammonia's and scraper's trees.
        #[test]
        fn the_model_answers_the_tokenizer_like_the_tree_builder(parts in prop::collection::vec(arb_part(), 0..60)) {
            let soup = parts.concat();
            // Each probe shows what state the soup left the parser in:
            // `<x/>` whether a start tag gets the HTML or the foreign rules,
            // `<style>` the raw-text decision, CDATA the namespace.
            for probe in ["", "<x/>", "<style>", "<![CDATA[x]]>"] {
                let input = format!("{soup}{probe}");
                if let Err(message) = oracle::check(&input) {
                    return Err(TestCaseError::fail(format!("{input:?}: {message}")));
                }
            }
        }

        // No start tag escapes the candidate count, whatever state the
        // tokenizer is in.
        #[test]
        fn every_start_tag_is_a_candidate(parts in prop::collection::vec(arb_part(), 0..60)) {
            let input = parts.concat();
            prop_assert!(oracle::start_tags(&input) <= start_tag_candidates(&input));
        }

        #[test]
        fn start_tag_candidates_match_a_character_scan(input in any::<String>()) {
            let characters: Vec<char> = input.chars().collect();
            let expected = characters
                .windows(2)
                .filter(|pair| pair[0] == '<' && pair[1].is_ascii_alphabetic())
                .count();
            prop_assert_eq!(start_tag_candidates(&input), expected);
        }
    }

    #[test]
    fn the_soup_reaches_the_shapes_that_matter() {
        use proptest::strategy::ValueTree;
        let mut runner = proptest::test_runner::TestRunner::deterministic();
        let strategy = prop::collection::vec(arb_part(), 0..60);
        let (mut followed, mut lost) = (0, 0);
        let mut sample = String::new();
        for _ in 0..500 {
            let input = strategy
                .new_tree(&mut runner)
                .expect("the generator always produces a value")
                .current()
                .concat();
            if scan(&input, usize::MAX).fallback.is_some() {
                lost += 1;
            } else {
                followed += 1;
            }
            sample.push_str(&input);
        }
        // Both branches of the model are exercised, not just the fallback.
        assert!(
            followed > 300 && lost > 10,
            "followed {followed}, lost {lost}"
        );
        for needle in [
            "<svg><div/>",
            "<math><p",
            "<![CDATA[>",
            "<foreignObject><",
            "</svg>",
        ] {
            assert!(sample.contains(needle), "{needle}");
        }
    }
}
