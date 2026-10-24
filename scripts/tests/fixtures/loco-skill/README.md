# Fixture crate for test-sync-loco-skill.sh

`crate/` stands in for an unpacked `loco-rs` crate: `crate/skills/loco/` plays
the upstream skill tree and `crate/LICENSE` the crate's licence file. The files
are tiny invented placeholders, not copies of the real skill, so the tests stay
fast and need no network. The tests copy this directory into a scratch
location before changing anything, so the fixture itself is never modified.

There is deliberately no `Cargo.toml` here: `scripts/sync-loco-skill.sh` only
needs the crate directory, and a stray manifest inside the repository could be
picked up by cargo tooling.
