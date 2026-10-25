//! Fat Active Record models on Sea-ORM (ADR 0008, rule 4).
//!
//! Each vertical slice adds its migration, runs `cargo loco db entities` to
//! regenerate `_entities/` (never edited by hand), and adds its model module
//! here. Finders live on `impl Model`, state transitions on
//! `impl ActiveModel`, and both take the connection explicitly. Database code
//! that several callers share and no entity owns, such as `task_lock`, is a
//! model module without an entity (rule 5).

pub mod task_lock;
