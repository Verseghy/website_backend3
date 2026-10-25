//! The schema snapshot test (ADR 0009, "What must be true in code and
//! tests"), and the test that keeps the catalog reader honest.
//!
//! A migration that changes the schema fails
//! [`the_migrated_schema_matches_the_snapshot`] until its PR updates
//! `snapshots/schema.snap`: run the tests, review the `.snap.new` that insta
//! writes beside it (`cargo insta review`), and commit it with the
//! migration. CI sets `INSTA_UPDATE=no`, so there a mismatch only fails.
//!
//! Before it compares the text, the test runs the guard (`guard.rs`), which
//! fails on anything in the database that the snapshot would not show
//! (within the limits that `guard.rs` lists), and names it.

use backend3::domain::schema_snapshot::{Object, render};
use sea_orm::ConnectionTrait;

use super::{catalog, guard};
use crate::support::with_app;

/// The header line of the committed snapshot, for whoever meets it in a diff.
const DESCRIPTION: &str = "The schema that the migrations create (ADR 0009), read from \
     pg_catalog by tests/schema. A migration updates this file in its own PR.";

#[tokio::test]
async fn the_migrated_schema_matches_the_snapshot() {
    with_app(|_router, ctx| async move {
        // First, so that a migration whose objects the snapshot cannot show
        // fails with their names rather than with a snapshot that looks
        // unchanged.
        let unread = guard::unread(&ctx.db).await;
        assert!(unread.is_empty(), "{}", guard::report(&unread));
        let text = render(&catalog::read(&ctx.db).await);
        // The plain file name: insta would otherwise prefix it with the
        // module path of the `mod` test crate (`mod__schema__snapshot__`).
        insta::with_settings!({
            prepend_module_to_snapshot => false,
            description => DESCRIPTION,
            omit_expression => true,
        }, {
            insta::assert_snapshot!("schema", text);
        });
    })
    .await;
}

/// One object of every kind that the snapshot shows but the migrations do
/// not create yet: an ICU collation with tailoring rules and a libc one
/// whose `LC_CTYPE` differs, comments, a collated column, an identity column,
/// stored and virtual generated columns, a CHECK, a foreign key, NOT NULL
/// constraints that are NOT VALID, NO INHERIT or named (beside plain ones,
/// which only their column lines show), an index, an enabled and a disabled
/// trigger and their function, a partitioned table with a partition, and an
/// UNLOGGED table that inherits a NOT NULL column from another. Every name
/// starts with `probe`. One batch of statements, which `execute_unprepared` sends as a
/// simple query.
const PROBE: &str = "
    CREATE COLLATION probe_rules (provider = icu, locale = 'hu', rules = '&a < z');
    CREATE COLLATION probe_ctype (provider = libc, lc_collate = 'C', lc_ctype = 'POSIX');
    CREATE FUNCTION probe_reject() RETURNS trigger LANGUAGE plpgsql
        AS $$ BEGIN RAISE EXCEPTION 'probe'; END $$;
    CREATE TABLE probe (
        id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
        name text COLLATE hu_icu NOT NULL DEFAULT '',
        folded text GENERATED ALWAYS AS (lower(name)) STORED,
        doubled bigint GENERATED ALWAYS AS (id * 2) VIRTUAL,
        parent_id bigint REFERENCES probe (id) ON DELETE RESTRICT,
        note text CONSTRAINT probe_note_nn NOT NULL NO INHERIT,
        label text CONSTRAINT probe_label_nn NOT NULL,
        CONSTRAINT probe_name_check CHECK (name <> 'x'));
    ALTER TABLE probe ADD CONSTRAINT probe_parent_id_nn NOT NULL parent_id NOT VALID;
    CREATE INDEX probe_parent_id_idx ON probe (parent_id);
    CREATE TRIGGER probe_guard BEFORE DELETE ON probe
        FOR EACH ROW EXECUTE FUNCTION probe_reject();
    CREATE TRIGGER probe_off BEFORE UPDATE ON probe
        FOR EACH ROW EXECUTE FUNCTION probe_reject();
    ALTER TABLE probe DISABLE TRIGGER probe_off;
    COMMENT ON TABLE probe IS 'Próba';
    COMMENT ON COLUMN probe.name IS E'Two\\nlines';
    CREATE TABLE probe_part (id bigint) PARTITION BY LIST (id);
    CREATE TABLE probe_part_one PARTITION OF probe_part FOR VALUES IN (1);
    CREATE TABLE probe_root (rooted int NOT NULL);
    CREATE UNLOGGED TABLE probe_heir () INHERITS (probe_root);
";

/// What [`PROBE`] renders as: its collation, table and function sections.
/// The NOT VALID constraint leaves `attnotnull` set, so the column line of
/// `parent_id` says NOT NULL and the constraint line says NOT VALID. The
/// plain NOT NULL of `name` has the default name, `probe_name_not_null`, so
/// it has no line of its own, and neither has the one `probe_heir` inherits,
/// which keeps its parent's name, `probe_root_rooted_not_null`.
const PROBE_TEXT: &str = "\
collation probe_ctype: provider libc, locale C, ctype POSIX, deterministic
collation probe_rules: provider icu, locale hu, rules '&a < z', deterministic

table probe
comment on table probe: Próba
column probe.id: bigint NOT NULL GENERATED ALWAYS AS IDENTITY
column probe.name: text COLLATE hu_icu NOT NULL DEFAULT ''::text
comment on column probe.name: Two
    | lines
column probe.folded: text GENERATED ALWAYS AS (lower(name)) STORED
column probe.doubled: bigint GENERATED ALWAYS AS ((id * 2)) VIRTUAL
column probe.parent_id: bigint NOT NULL
column probe.note: text NOT NULL
column probe.label: text NOT NULL
constraint probe.probe_label_nn: NOT NULL label
constraint probe.probe_name_check: CHECK ((name <> 'x'::text))
constraint probe.probe_note_nn: NOT NULL note NO INHERIT
constraint probe.probe_parent_id_fkey: FOREIGN KEY (parent_id) REFERENCES probe(id) ON DELETE RESTRICT
constraint probe.probe_parent_id_nn: NOT NULL parent_id NOT VALID
constraint probe.probe_pkey: PRIMARY KEY (id)
index probe.probe_parent_id_idx: CREATE INDEX probe_parent_id_idx ON public.probe USING btree (parent_id)
index probe.probe_pkey: CREATE UNIQUE INDEX probe_pkey ON public.probe USING btree (id)
trigger probe.probe_guard: CREATE TRIGGER probe_guard BEFORE DELETE ON public.probe FOR EACH ROW EXECUTE FUNCTION probe_reject()
trigger probe.probe_off: CREATE TRIGGER probe_off BEFORE UPDATE ON public.probe FOR EACH ROW EXECUTE FUNCTION probe_reject() -- tgenabled = D

table probe_heir: UNLOGGED INHERITS (probe_root)
column probe_heir.rooted: integer NOT NULL

table probe_part: PARTITION BY LIST (id)
column probe_part.id: bigint

table probe_part_one: PARTITION OF probe_part FOR VALUES IN ('1')
column probe_part_one.id: bigint

table probe_root
column probe_root.rooted: integer NOT NULL

function probe_reject(): CREATE OR REPLACE FUNCTION public.probe_reject()
    |  RETURNS trigger
    |  LANGUAGE plpgsql
    | AS $function$ BEGIN RAISE EXCEPTION 'probe'; END $function$
";

/// Whether `object` is one of [`PROBE`]'s: its table's name, or for an
/// object outside tables its own name, starts with `probe`. Filtering the
/// objects rather than the rendered text keeps whatever the migrations
/// create (their tables and functions) out of the comparison.
fn is_probe(object: &Object) -> bool {
    object
        .table
        .as_ref()
        .unwrap_or(&object.name)
        .starts_with("probe")
}

#[tokio::test]
async fn the_reader_shows_every_kind_of_object_that_the_formatter_renders() {
    with_app(|_router, ctx| async move {
        ctx.db
            .execute_unprepared(PROBE)
            .await
            .expect("the probe objects");
        let mut objects = catalog::read(&ctx.db).await;
        objects.retain(is_probe);
        assert_eq!(render(&objects), PROBE_TEXT);
        // The other side of the guard's cases: everything the reader shows
        // passes the guard, so the two agree on what is read.
        let unread = guard::unread(&ctx.db).await;
        assert!(unread.is_empty(), "{}", guard::report(&unread));
    })
    .await;
}
