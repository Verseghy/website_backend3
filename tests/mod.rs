//! Integration and request tests of backend3, in one test binary as Loco's
//! template lays them out (ADR 0008, "Testing per layer").
//!
//! Every test that boots the app uses Loco's per-test databases through
//! `support::with_app` and `support::with_server`: each test creates
//! its own `_loco_test_*` database in the local PostgreSQL of compose.yaml and
//! migrates it, so tests run in parallel without sharing state. Tests of
//! time-dependent rules use `support::with_app_at` and `support::with_server_at`,
//! which start the app clock at a chosen instant (ADR 0010 point 7). Tests that
//! must observe the process environment run the built `backend3-cli` binary
//! as a child process with an explicit environment instead of changing their
//! own.
//!
//! `media_contract` boots no app: it checks a `/storage` server over HTTP
//! (ADR 0015 §1). Its nginx test starts a container, so it is ignored here and
//! run by its own step of the Test job.

mod boot;
mod cli;
mod domain;
mod media_contract;
mod migration;
mod models;
mod requests;
mod schema;
mod support;
mod tasks;
mod workers;
