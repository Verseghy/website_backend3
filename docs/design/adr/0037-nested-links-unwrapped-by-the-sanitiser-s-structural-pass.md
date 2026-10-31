# 0037. Nested links unwrapped by the sanitiser's structural pass

- Status: Accepted (2026-10-31)
- Decision questions: none: records the maintainer's answer of 2026-10-31 to the third decision register's item D112 (research: DECISIONS R21), which settles a rule of the structural pass that the `sanitizer-structural-pass` slice built where [ADR 0013](0013-rich-text-editor-and-html-sanitiser.md) §5 is silent. It amends the structural-pass bullet of ADR 0013 §5.

## Context

[ADR 0013](0013-rich-text-editor-and-html-sanitiser.md) §5 runs a structural pass on a fresh parse of ammonia's output, inside a fixed-point loop. These are the parts this ADR touches:

| Rule | What ADR 0013 §5 says |
|---|---|
| The structural pass | "It strips the child nodes of every `iframe`, removes an `img` that has no `src`, drops `font-size` from a span inside a sized span, unwraps a `sub` or `sup` nested in another, and writes every element's attributes in the canonical order." |
| The fixed-point loop | "The sanitiser repeats until the output is a fixed point, at most three passes. An input that does not converge is refused." |
| Properties | "`S(S(x)) == S(x)`. `S(e) == e` for every body the editor emits." |

The list of the structural pass names no rule for a link inside another link. The `sanitizer-structural-pass` slice found that the output does not settle without one (research: third decision register D112; [editor.md](../editor.md) §7.5; `src/domain/rich_text/structure.rs`, "How the output settles"):

1. **The defect.** A property test that the structural pass settles failed in about 2.5 % of runs: about 1 input in 10,000 needed a third structural rewrite.
2. **The cause: html5ever's repair steps.** When the HTML the sanitiser writes holds a link inside a link, every new parse moves content again: at the inner `<a>` start tag the parser runs the adoption agency algorithm, which closes the open link and moves the content between them, except across a `td` or `th`, where the parser's list of active formatting elements starts again. A parse builds the nesting in two ways:
   - **Foster parenting.** ammonia's parse fosters a link out of a table into an enclosing link: behind the table the outer link is out of scope, so the adoption agency gives up, and the new link lands inside the old one (fixture `link-fostered-into-link` in `tests/fixtures/regressions/`).
   - **The adoption agency's eight-block limit.** The algorithm stops after eight rounds of its outer loop and moves one block per round (WHATWG HTML, "adoption agency algorithm"; html5ever 0.40.1). With eight or more blocks between a link start tag and the open link, a copy of the outer link stays open around the new one. This needs no table, and markup that closes every element builds it too (fixture `nested-link-deeper-than-eight-blocks`).
3. **What it did to the output.** The structural pass alone needed three rewrites for some tag soup. A link around eight or more nested blocks changed again in every parse, eight levels at a time, so the number of passes grew with the depth, and a link around 24 quotes was refused with `content_not_normalisable`.
4. **The rule as built.** The structural pass unwraps a link nested inside another link unless a table cell lies between them. That removes the cause: the structural pass settles in two applications, and the whole sanitiser in at most three passes, with one parse to spare. The settle properties passed 500,000 cases each in the slice's runs.
5. **What changes for content.** Only tag soup and imported markup can hold a link inside a link; links are marks on text, so the editor never writes one. In `<a href="/x"><table><a href="/y">b</a></table></a>` the text b now links to /x. A browser would follow /y there, because the nesting stays in its DOM. A link inside a table cell inside a link is kept, because the cell separates them.
6. **Versioning.** The rule changes the sanitiser's output for that class of input in place, with no content-v1 or `POLICY_VERSION` bump, on the same basis as D87: nothing is stored yet. D87 confirmed R11's in-place amendments of content-v1 on that basis on 2026-10-25: "Confirm all four as built" (research: DECISIONS R15).

ADR 0013 §5 lists the structural pass's rules, so adding one changes an Accepted decision, which under §6 of [ADR 0001](0001-record-architecture-decisions.md) only a new ADR that quotes the maintainer's answer does. The maintainer answered on 2026-10-31 (research: DECISIONS R21), with the recommended option. The register's question, the answer, and the consequence the register gave for that option (research: third decision register D112):

- **D112**, "To make the sanitiser settle reliably, sanitizer-structural-pass now unwraps a link that sits inside another link in the same table cell: the inner link's text stays, its tag is removed, so the text links to the outer link's address. ADR 0013 §5 lists the structural pass's rules and does not name this one. Do you accept the rule as built, recorded in an amending ADR?": "Accept the rule as built, in an amending ADR". Its consequence: "An amending ADR in the structural-pass PR adds the rule to ADR 0013 §5 and quotes this answer; editor.md §7.5 already lists it. No code change; the sanitizer pair then lands. The inner link's text links to the outer address."

## Decision

### 1. What this ADR amends

| Amended text | Change | Section |
|---|---|---|
| ADR 0013 §5, the structural pass: "It strips the child nodes of every `iframe`, removes an `img` that has no `src`, drops `font-size` from a span inside a sized span, unwraps a `sub` or `sup` nested in another, and writes every element's attributes in the canonical order." | The list gains one rule: the pass also unwraps a link nested inside another link, unless a table cell lies between them. | 2 |
| ADR 0013 §5, the fixed-point loop: "at most three passes" | The limit holds as written; section 3 records why three passes are enough with the rule. | 3 |

Everything else in ADR 0013, §5's properties, its invariants and its validation codes included, holds as written. The rule adds no error code: a nested link is repaired, never refused.

### 2. The rule

- **What.** The structural pass unwraps an `a` that has another `a` as an ancestor, unless a table cell (`td` or `th`) lies between them. The inner link's tag is removed and its content stays where it was, inside the outer link, so the inner text links to the outer link's address. For example, `<a href="/x">a<table><a href="/y">b</a></table>c</a>` becomes `<a href="/x">ab<table></table></a>c`.
- **The cell exception.** A link inside a cell of a table that a link holds parses back as written, because the parser starts a new list of active formatting elements in each cell, so it is kept. A link nested inside that cell's own link is unwrapped like any other.
- **The report.** Each unwrapped link is counted once in the removal report as `nested_link_unwrapped` (`RemovalRule::NestedLinkUnwrapped`). The drift canary, the import, revert and `content_normalize` read that report ([editor.md](../editor.md) §7.2), so the repair shows wherever it happens.
- **Editor output.** The editor never nests links, so `S(e) == e` holds as before.
- **Implemented by:** `sanitizer-structural-pass` (this ADR's PR), in `src/domain/rich_text/structure.rs`, specified in [editor.md](../editor.md) §7.5.

### 3. Why: the output settles

- **The structural pass** reaches a fixed point after two applications: `restructure(restructure(restructure(x)))` equals `restructure(restructure(x))`, and the third application removes nothing. The second application parses markup that a tree wrote, with no link left inside a link: nothing is fostered, the adoption agency runs only for the end tag of a formatting element and moves no node, and the remaining repairs only close elements, which builds a tree that parses back into itself.
- **The sanitiser** settles within ADR 0013 §5's three passes with one parse to spare: the second pass's structural parse never changes anything. The number of passes no longer grows with the nesting depth.
- **The argument is a sketch** over the parser rules that content-v1's elements reach, written out in `structure.rs` and [editor.md](../editor.md) §7.5, not a proof over every rule of the tree builder. Proptests check it over tag soup and over nested links up to 240 levels deep, and the fixed-point loop still refuses an input that keeps changing (`content_not_normalisable`).

### 4. Output changed in place, with no version bump

- The rule changes what the sanitiser returns for a link nested in a link with no cell between them. It is made in place: `POLICY_VERSION` stays `"content-v1"`, the content-v1 tables of [editor.md](../editor.md) §3 do not change, and no `content_normalize` run is needed.
- **The basis** is D87's: nothing is stored yet. No body was written under the earlier output, so no stored body is served verbatim with bytes that the sanitiser would now write differently ([editor.md](../editor.md) §8).
- This answer does not change how later changes are versioned ([editor.md](../editor.md) §3, §8.3).

## Consequences

**Positive**

- The convergence defect is fixed with the smallest change: the structural pass settles in two applications and the sanitiser in at most three passes, at every nesting depth, and a link around many nested blocks, such as 24 quotes, is stored instead of refused.
- The sanitiser keeps repairing rather than refusing: no new error code and no new Hungarian message.
- Editor output is unchanged.

**Negative**

- For such input the inner link's address is lost: its text links to the outer address, while a browser that rendered the original markup follows the inner link. Only tag soup, hand-made API requests and imported markup can hold the nesting.
- The convergence argument stays a sketch, backed by the proptests and by the loop's refusal of an input that keeps changing.

**Follow-up work**

- In this ADR's PR (`sanitizer-structural-pass`): ADR 0013's Status line, both ADR indexes and the overview's ADR list; [editor.md](../editor.md) §7.5 and the doc comments of `structure.rs`, `report.rs` and `sanitize.rs` that describe the rule cite this ADR.
- `sanitizer-css-values`, stacked on this PR, rebases onto it.

**What must be true in code and tests**

- Unit tests of `structure.rs`: a link nested in a link is unwrapped and counted once as `nested_link_unwrapped`, including a link fostered out of a table into another link and a link with eight or more blocks between it and the open link; a link in a `td` or `th` of a table inside a link is kept; a link nested in that cell's own link is unwrapped; sibling links stay; every output is a fixed point of the pass.
- Proptests: the structural pass settles after two applications over tag soup and over nested links up to 240 levels deep; the sanitiser's idempotence (P1) and determinism (P5).
- Fixtures in `tests/fixtures/regressions/`: `link-fostered-into-link`, `nested-link-deeper-than-eight-blocks` and `link-around-table`.

## Alternatives considered

| Option | Source | Why not |
|---|---|---|
| Accept the rule as built, in an amending ADR | third decision register D112 | Chosen (research: DECISIONS R21). |
| Keep the inner link instead: unwrap the outer link where the two nest, so the text keeps its own address, as a browser follows it | third decision register D112 | Not chosen: following the browser's inner link needs the outer link split around it, which is more code for the same rare input. The code, its tests, editor.md §7.5 and the fixtures would change before the sanitiser pair lands, and the convergence argument would have to be checked again. The rest of the outer link's text would still link to the outer address. |
| Refuse content with nested links: reject an admin write that holds a link inside a link with a new error code and Hungarian message, and report such legacy rows in the import | third decision register D112 | Not chosen: the sanitiser otherwise repairs rather than refuses. The sanitiser and its docs, the error envelope, `hu.ts` and the importer's report would change, and some legacy pages might need fixing by hand. |
| No rule: leave nested links in place and rely on the fixed-point loop | the slice's first build (research: third decision register D112) | Not chosen: this is the defect. Each parse moves content again, eight levels at a time, so no fixed number of passes covers the depths that `max_nesting_depth` admits, and a link around 24 quotes was already refused with `content_not_normalisable`. |

## References

- research: DECISIONS R21 (D112), the answer of 2026-10-31; research: DECISIONS R15 (D87), the basis for changing output in place
- research: third decision register D112 (the question, its context and the options)
- Amended: [ADR 0013](0013-rich-text-editor-and-html-sanitiser.md) §5, the structural pass
- Related: [editor.md](../editor.md) §7.2, §7.5, §8, §13.6; [ADR 0001](0001-record-architecture-decisions.md) §6 (changing a decision)
- WHATWG HTML Living Standard, "adoption agency algorithm" and "foster parenting"; html5ever@0.40.1:src/tree_builder/mod.rs (`adoption_agency`, foster parenting) and src/tree_builder/rules.rs
