//! Tests of `src/domain/` that need fixture files.
//!
//! The pure modules carry their unit tests and proptests in-module, but they
//! may not read files (ADR 0008, rule 1). Tests that run them over a corpus
//! of real files, such as sample images, live here instead.

mod image_optimise;
