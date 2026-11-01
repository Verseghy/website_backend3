//! The `revisions` table: one JSON snapshot of a record after each of its
//! saves, for every admin-editable entity (ADR 0011 §4).
//!
//! The columns and the index are ADR 0011 §4's, the append-only rules its
//! §5 table. As for `activity_log`, Loco's `create_table` cannot write this
//! table: it always adds `updated_at`, which a revision never has, because a
//! revision never changes. So every statement here is plain SQL, with the
//! names PostgreSQL itself would choose for keys and indexes and descriptive
//! `snake_case` names for the CHECKs (ADR 0009).
//!
//! - **One table for every entity.** `entity_type` is the record's table
//!   name (`posts`, `labels`, `users`, ...) and `entity_id` its id, so no
//!   foreign key can point at the record: permanent delete removes the
//!   record's revisions in the same transaction as the record itself
//!   (ADR 0011 §6). `backend3::models::revisions` locks the record's row
//!   before it writes, and refuses a revision of a row that does not exist.
//!   Its `delete_for_record` takes the same lock before a permanent delete
//!   removes the record's revisions, so that a save still running cannot
//!   leave one behind.
//! - **The revision number** counts 1, 2, 3, ... per record. The unique
//!   index on `(entity_type, entity_id, revision_no DESC)` is both the rule
//!   "unique on `(entity_type, entity_id, revision_no)`" and the index ADR
//!   0011 §4 asks for: the history panel and the next number read a record's
//!   revisions newest first. A `UNIQUE` constraint cannot have a descending
//!   column (PostgreSQL's `ADD CONSTRAINT ... USING INDEX` takes only an
//!   index of the default sort order), and a second index on the same three
//!   columns would only cost writes, so it is a unique index. The model
//!   takes the next number under a row lock on the record, so two saves of
//!   one record never race for it; the index is what makes a mistake fail
//!   instead of writing two revisions with one number.
//! - **The instant comes from PostgreSQL's clock.** `created_at` defaults to
//!   `CURRENT_TIMESTAMP`, the start of the writing transaction, so a revision
//!   carries the same instant as the audit columns of the save it records
//!   and as its activity entry (research: DECISIONS R15, D90).
//! - **Who saved it.** `created_by` references `users (id)` with `ON DELETE
//!   RESTRICT`, as every audit column does: accounts are anonymised, never
//!   deleted, and keep every reference (ADR 0011 §6). It is NULL for the
//!   system and the importer (ADR 0011 §2); the table has no actor kind, so
//!   `revisions_import_without_user` holds the one rule the column can show:
//!   an `import` or `import_original` revision was written by the importer,
//!   never by a user. Its own index serves the key (ADR 0009,
//!   "Constraints") and the anonymisation, which counts the revisions a user
//!   saved (ADR 0011 "What must be true").
//! - **String codes** (ADR 0009, "Column types"): `reason` is `text` with a
//!   CHECK of the five reasons of ADR 0011 §4, mapped to `Reason` in
//!   `src/domain/revision.rs`; `tests/models/revisions.rs` fails while they
//!   differ. A new reason is a new migration that replaces
//!   `revisions_reason_known`, together with the new variant.
//! - **`snapshot` is a JSON object**, the entity's revision snapshot type
//!   serialised (ADR 0011 §4), and `schema_version` that type's version,
//!   from 1. `revision_no` starts at 1 too.
//!
//! **Append-only** (ADR 0011 §5). Three triggers make the table
//! tamper-resistant against application bugs:
//!
//! - `UPDATE` and `TRUNCATE` are always refused, also in a permanent-delete
//!   transaction, by statement-level triggers, so even a statement that
//!   would touch no row fails. That covers every statement PostgreSQL counts
//!   as an `UPDATE`: an upsert (`INSERT ... ON CONFLICT DO UPDATE`, even
//!   when nothing conflicts) and a `MERGE` with an `UPDATE` action. The
//!   `TRUNCATE` trigger also fires for a `TRUNCATE users CASCADE`, which is
//!   why `App::truncate` never covers `users` or this table.
//! - `DELETE` is refused unless the transaction has set the permanent-delete
//!   flag, the setting `backend3.permanent_delete` that
//!   `backend3::models::activity_log::with_permanent_delete_flag` sets to
//!   `on` with `SET LOCAL`, the one `activity_log`'s `UPDATE` trigger reads.
//!   Only a permanent delete issues it: a record's purge deletes the
//!   record's revisions, and the anonymisation of a user deletes the
//!   revisions of the account record (ADR 0011 §6). The check is at
//!   statement level, before any row is read, so a `DELETE` or a `MERGE ...
//!   DELETE` that matches no row is refused like one that matches some.
//!   Unlike `activity_log`, which compares each changed row, a `DELETE`
//!   needs no row-level check: the flag's value when the trigger fires
//!   decides. That is when the `DELETE` starts to run, which is not always
//!   when its statement starts. A `set_config` in the `DELETE`'s own `WHERE`
//!   clause, or in a `WITH` query attached to that `DELETE`, runs after the
//!   trigger and comes too late. A `DELETE` written as a data-modifying
//!   `WITH` query of another statement runs only when that statement reads
//!   its output, or after the rest of it, so a `set_config(..., true)`
//!   earlier in that statement has already set the flag for the
//!   transaction. `set_config(..., true)` does what `SET LOCAL` does, so the
//!   trigger cannot tell the two apart: the flag marks the transaction, not
//!   one statement. `tests/migration/revisions.rs` pins both behaviours.
//!
//! Removing a trigger takes a migration, which is a reviewed PR. The trigger
//! functions are `plpgsql`, name only `pg_catalog` functions and need no
//! `search_path` of their own; they raise SQLSTATE 23001
//! (`restrict_violation`) with a message that names the rule, as
//! `activity_log`'s do.

use sea_orm_migration::prelude::*;

/// Creates the `revisions` table, its constraints and index, and the
/// append-only triggers.
#[derive(DeriveMigrationName)]
pub struct Migration;

/// The reasons of ADR 0011 §4, in the order it lists them. `Reason::ALL` in
/// `src/domain/revision.rs` lists the same, and `tests/models/revisions.rs`
/// compares the stored CHECK with it.
const REASONS: &[&str] = &["create", "update", "revert", "import", "import_original"];

/// The two reasons only the importer writes (ADR 0011 §4, "Import").
const IMPORT_REASONS: &[&str] = &["import", "import_original"];

/// The table, in the column order of ADR 0011 §4. `id` is written as Loco's
/// `ColType::PkAuto` renders it (ADR 0009, "Primary keys and ids").
const CREATE_TABLE: &str = "CREATE TABLE revisions (
    id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    entity_type text NOT NULL,
    entity_id bigint NOT NULL,
    revision_no integer NOT NULL,
    reason text NOT NULL,
    schema_version integer NOT NULL,
    snapshot jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_by bigint,
    request_id text
)";

/// The constraints and indexes after the table, in order. The CHECKs that
/// name reasons are built from the lists above by [`reason_checks`].
const CONSTRAINTS: &[&str] = &[
    "ALTER TABLE revisions ADD CONSTRAINT revisions_created_by_fkey \
     FOREIGN KEY (created_by) REFERENCES users (id) ON DELETE RESTRICT",
    "ALTER TABLE revisions ADD CONSTRAINT revisions_revision_no_positive \
     CHECK (revision_no >= 1)",
    "ALTER TABLE revisions ADD CONSTRAINT revisions_schema_version_positive \
     CHECK (schema_version >= 1)",
    "ALTER TABLE revisions ADD CONSTRAINT revisions_snapshot_object \
     CHECK (jsonb_typeof(snapshot) = 'object')",
    "CREATE UNIQUE INDEX revisions_entity_type_entity_id_revision_no_idx \
     ON revisions (entity_type, entity_id, revision_no DESC)",
    "CREATE INDEX revisions_created_by_idx ON revisions (created_by)",
];

/// The trigger function of `UPDATE` and `TRUNCATE`: it refuses the
/// statement and names the operation (`TG_OP`).
const REFUSE_FUNCTION: &str = "CREATE FUNCTION revisions_refuse() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'revisions is append-only: % is never allowed (ADR 0011 §5)', TG_OP
        USING ERRCODE = 'restrict_violation';
END
$$";

/// The trigger function of `DELETE`. The flag is read with `missing_ok`, so
/// an unset flag is NULL; after a transaction that set it ends, the session
/// still knows the setting with the empty value, so only `on` counts. The
/// return value of a statement-level trigger is ignored (NULL by
/// convention).
const CHECK_DELETE_FUNCTION: &str = "CREATE FUNCTION revisions_check_delete() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('backend3.permanent_delete', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'revisions is append-only: DELETE is allowed only in a \
permanent-delete transaction, which sets backend3.permanent_delete with SET LOCAL (ADR 0011 §5)'
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NULL;
END
$$";

/// The three triggers, all `BEFORE` and statement-level, so a refused
/// statement changes nothing, fires no other trigger, and is refused even
/// when no row matches: PostgreSQL fires a statement-level `BEFORE` trigger
/// before it reads any row.
const TRIGGERS: &[&str] = &[
    "CREATE TRIGGER revisions_refuse_update BEFORE UPDATE ON revisions \
     FOR EACH STATEMENT EXECUTE FUNCTION revisions_refuse()",
    "CREATE TRIGGER revisions_refuse_truncate BEFORE TRUNCATE ON revisions \
     FOR EACH STATEMENT EXECUTE FUNCTION revisions_refuse()",
    "CREATE TRIGGER revisions_check_delete BEFORE DELETE ON revisions \
     FOR EACH STATEMENT EXECUTE FUNCTION revisions_check_delete()",
];

/// `values` as a parenthesised SQL list of string literals, for `IN`. Every
/// value is a constant of this file, all `snake_case`, so none needs quoting
/// beyond the single quotes.
fn sql_list(values: &[&str]) -> String {
    let quoted = values
        .iter()
        .map(|value| format!("'{value}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("({quoted})")
}

/// The two CHECKs built from the lists above, in order:
///
/// - `revisions_reason_known`: exactly the reasons of ADR 0011 §4.
/// - `revisions_import_without_user`: an `import` or `import_original`
///   revision has no `created_by`, because only the importer writes one
///   (ADR 0011 §2, §4).
fn reason_checks() -> [String; 2] {
    [
        format!(
            "ALTER TABLE revisions ADD CONSTRAINT revisions_reason_known CHECK (reason IN {})",
            sql_list(REASONS)
        ),
        format!(
            "ALTER TABLE revisions ADD CONSTRAINT revisions_import_without_user \
             CHECK (created_by IS NULL OR reason NOT IN {})",
            sql_list(IMPORT_REASONS)
        ),
    ]
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        db.execute_unprepared(CREATE_TABLE).await?;
        for statement in CONSTRAINTS {
            db.execute_unprepared(statement).await?;
        }
        for statement in reason_checks() {
            db.execute_unprepared(&statement).await?;
        }
        db.execute_unprepared(REFUSE_FUNCTION).await?;
        db.execute_unprepared(CHECK_DELETE_FUNCTION).await?;
        for statement in TRIGGERS {
            db.execute_unprepared(statement).await?;
        }
        Ok(())
    }

    /// Drops the table, which takes its constraints, index and triggers with
    /// it (`DROP TABLE` fires no `DELETE` or `TRUNCATE` trigger), then the
    /// two trigger functions.
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        db.execute_unprepared("DROP TABLE revisions").await?;
        db.execute_unprepared("DROP FUNCTION revisions_refuse(), revisions_check_delete()")
            .await?;
        Ok(())
    }
}
