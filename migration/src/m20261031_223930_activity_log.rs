//! The `activity_log` table: the append-only record of who did what and when
//! (ADR 0011 §5).
//!
//! The columns, the three indexes and the append-only rules are ADR 0011 §5's.
//! Loco's `create_table` cannot write this table: it pluralises the name it
//! is given (`activity_log` would become `activity_logs`), always adds
//! `created_at` and `updated_at`, which an entry does not have, and has no
//! `inet` column type. So every statement here is plain SQL, with the names
//! PostgreSQL itself would choose for keys and indexes, and descriptive
//! `snake_case` names for the CHECKs (ADR 0009), as the `users` migration
//! does.
//!
//! - **The instant comes from PostgreSQL's clock.** `occurred_at` defaults
//!   to `CURRENT_TIMESTAMP`, the start of the writing transaction, and
//!   writers leave it unset, so an entry carries the same instant as the
//!   audit columns of the change it records (research: DECISIONS R15, D90).
//! - **Actors.** `actor_id` references `users (id)` with `ON DELETE
//!   RESTRICT`, as the audit columns of `users` do: accounts are anonymised,
//!   never deleted, and keep every reference (ADR 0011 §6). The CHECKs tie
//!   it to `actor_kind`: an id for `user`, none for `system` and `import`.
//!   The index on `(actor_id, occurred_at DESC)` starts with the foreign key
//!   column, so it also serves the key (ADR 0009, "Constraints").
//! - **Records.** An `entity_id` needs its `entity_type`, and a
//!   `revision_id` the record it belongs to (ADR 0011 §4), both by CHECK. A
//!   table without a record id is allowed: the importer writes one entry per
//!   table (ADR 0011 §5).
//! - **No foreign key on `revision_id`.** Permanent delete removes a
//!   record's revisions but keeps its activity entries (ADR 0011 §6), and the
//!   append-only triggers below would refuse the `SET NULL` that a key would
//!   need; the column keeps the id of a revision that may be gone.
//! - **String codes** (ADR 0009, "Column types"): `actor_kind` and `action`
//!   are `text` with a CHECK of the allowed values, mapped to the Rust enums
//!   `Actor` and `Action` of `src/domain/activity.rs`. A new action is a new
//!   migration that replaces `activity_log_action_known` (and, for an
//!   account action, the two CHECKs below that list the account actions),
//!   together with the new enum variant; `tests/models/activity_log.rs`
//!   fails while they differ.
//! - **`details` is an object or NULL**, like `posts.images`' array CHECK
//!   (ADR 0009): every shape ADR 0011 §5 names (changed field names, media
//!   paths and sizes, batch counts) is a JSON object.
//! - **Personal data stays out by CHECK too.** An entry can never be
//!   corrected, and the anonymisation may clear only `ip` (ADR 0011 §6), so
//!   the two rules of ADR 0011 §5 that keep personal data out of the log
//!   hold for every INSERT, also one that does not go through
//!   `backend3::models::activity_log::record`: an `ip` only on an account
//!   action (`activity_log_ip_account_only`), and a `label` only on an entry
//!   about a content record (`activity_log_label_content_only`: never on an
//!   account action, never without an `entity_type`, never for `users`,
//!   `authors` or `colleagues`, which are people). `record` checks the same
//!   rules first (`backend3::domain::activity::check_entry`), so its refusal
//!   names the rule rather than a constraint. The rules that keep only a
//!   badly formed value out (a blank label, an `entity_type` that is not
//!   `snake_case`, a request id outside 1 to 255 characters) stay in that
//!   check alone: none of them lets personal data in.
//!
//! **Append-only** (ADR 0011 §5). Four triggers make the table
//! tamper-resistant against application bugs:
//!
//! - `DELETE` and `TRUNCATE` are always refused, by statement-level triggers,
//!   so even a statement that would touch no row fails. The `TRUNCATE`
//!   trigger also fires for a `TRUNCATE users CASCADE`, which is why
//!   `App::truncate` never covers `users` or this table.
//! - `UPDATE` is refused unless the transaction has set the permanent-delete
//!   flag, and even then the only change allowed is `ip` set to NULL, which
//!   only the anonymisation of a user issues (ADR 0011 §6). Two triggers
//!   share one function. The statement-level one checks the flag before any
//!   row is read, so an `UPDATE` that matches no row is refused like a
//!   `DELETE` that matches none, and so is every statement that PostgreSQL
//!   counts as an `UPDATE`: an upsert (`INSERT ... ON CONFLICT DO UPDATE`,
//!   even when nothing conflicts) and a `MERGE` with an `UPDATE` action,
//!   even when nothing matches. A plain `INSERT`, `ON CONFLICT DO NOTHING`
//!   and an insert-only `MERGE` fire no `UPDATE` trigger. The row-level
//!   one checks the flag again (an expression of the statement could call
//!   `set_config`) and then compares the new row with the old one with `ip`
//!   cleared, using the binary row comparison `*<>`, so any other changed
//!   byte is refused (also one that `=` would call equal, such as `1` and
//!   `1.0` in `details`), and a column added later is covered without
//!   changing the trigger.
//! - The flag is the setting `backend3.permanent_delete`, set to `on` with
//!   `SET LOCAL` by
//!   `backend3::models::activity_log::with_permanent_delete_flag`, so it ends
//!   with the transaction; set inside a savepoint, it lasts until the
//!   outermost transaction ends, unless the savepoint is rolled back. A
//!   custom setting needs a dotted name, and any role may set one; the
//!   `revisions` table will check the same flag. The triggers read only the
//!   value, so they cannot tell `SET LOCAL` from a session-wide `SET`: the
//!   model never issues the latter, and even then the flag lets nothing
//!   through but clearing an address.
//!
//! Removing a trigger takes a migration, which is a reviewed PR. The trigger
//! functions are `plpgsql`, which initdb installs in every database. They
//! name only `pg_catalog` functions and the trigger's own rows, so they need
//! no `search_path` setting of their own. They raise SQLSTATE 23001
//! (`restrict_violation`), with a message that names the rule.

use sea_orm_migration::prelude::*;

/// Creates the `activity_log` table, its constraints, indexes and the
/// append-only triggers.
#[derive(DeriveMigrationName)]
pub struct Migration;

/// The content actions of ADR 0011 §5, in the order of its table. Together
/// with [`ACCOUNT_ACTIONS`] and [`MEDIA_ACTIONS`], in that order, they are
/// every action the `action` CHECK allows; `Action::ALL` in
/// `src/domain/activity.rs` lists the same, and `tests/models/activity_log.rs`
/// compares the stored CHECKs with it.
const CONTENT_ACTIONS: &[&str] = &[
    "create",
    "update",
    "revert",
    "delete",
    "restore",
    "purge",
    "publish",
    "unpublish",
    "reorder",
    "preview_link_regenerated",
    "import",
];

/// The sign-in and account actions of ADR 0011 §5 (ADR 0006), in the order
/// of its table: the only entries that may hold an `ip`, and never a
/// `label`, because they are about a person.
const ACCOUNT_ACTIONS: &[&str] = &[
    "login",
    "login_failed",
    "logout",
    "password_changed",
    "reset_link_issued",
    "temporary_password_issued",
    "totp_enabled",
    "totp_disabled",
    "totp_reset",
    "recovery_code_used",
    "recovery_codes_regenerated",
    "account_unlocked",
    "user_activated",
    "user_deactivated",
    "roles_changed",
    "grants_changed",
];

/// The media-library actions of ADR 0011 §5 (ADR 0015), in the order of its
/// table.
const MEDIA_ACTIONS: &[&str] = &[
    "media_upload",
    "media_rename",
    "media_move",
    "media_delete",
    "media_restore",
    "media_purge",
];

/// The table, in the column order of ADR 0011 §5. `id` is written as Loco's
/// `ColType::PkAuto` renders it (ADR 0009, "Primary keys and ids").
const CREATE_TABLE: &str = "CREATE TABLE activity_log (
    id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
    occurred_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
    actor_id bigint,
    actor_kind text NOT NULL,
    action text NOT NULL,
    entity_type text,
    entity_id bigint,
    revision_id bigint,
    label text,
    details jsonb,
    request_id text,
    ip inet
)";

/// The entity types whose records are people (ADR 0011 §5, `label`): their
/// entries keep only the id. `PEOPLE_ENTITY_TYPES` in `src/domain/activity.rs`
/// lists the same.
const PEOPLE_ENTITY_TYPES: &[&str] = &["users", "authors", "colleagues"];

/// The constraints and indexes after the table, in order. The CHECKs that
/// name actions or people are built from the lists above by
/// [`list_checks`].
const CONSTRAINTS: &[&str] = &[
    "ALTER TABLE activity_log ADD CONSTRAINT activity_log_actor_id_fkey \
     FOREIGN KEY (actor_id) REFERENCES users (id) ON DELETE RESTRICT",
    "ALTER TABLE activity_log ADD CONSTRAINT activity_log_actor_kind_known \
     CHECK (actor_kind IN ('user', 'system', 'import'))",
    "ALTER TABLE activity_log ADD CONSTRAINT activity_log_actor_shape \
     CHECK ((actor_kind = 'user') = (actor_id IS NOT NULL))",
    "ALTER TABLE activity_log ADD CONSTRAINT activity_log_entity_shape \
     CHECK (entity_id IS NULL OR entity_type IS NOT NULL)",
    "ALTER TABLE activity_log ADD CONSTRAINT activity_log_revision_shape \
     CHECK (revision_id IS NULL OR entity_id IS NOT NULL)",
    "ALTER TABLE activity_log ADD CONSTRAINT activity_log_details_object \
     CHECK (jsonb_typeof(details) = 'object')",
    "CREATE INDEX activity_log_occurred_at_idx ON activity_log (occurred_at DESC)",
    "CREATE INDEX activity_log_actor_id_occurred_at_idx \
     ON activity_log (actor_id, occurred_at DESC)",
    "CREATE INDEX activity_log_entity_type_entity_id_occurred_at_idx \
     ON activity_log (entity_type, entity_id, occurred_at DESC)",
];

/// The trigger function of `DELETE` and `TRUNCATE`: it refuses the statement
/// and names the operation (`TG_OP`).
const REFUSE_FUNCTION: &str = "CREATE FUNCTION activity_log_refuse() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'activity_log is append-only: % is never allowed (ADR 0011 §5)', TG_OP
        USING ERRCODE = 'restrict_violation';
END
$$";

/// The trigger function of both `UPDATE` triggers. The flag is read with
/// `missing_ok`, so an unset flag is NULL; after a transaction that set it
/// ends, the session still knows the setting with the empty value, so only
/// `on` counts. At statement level that is the whole check, and the return
/// value is ignored (NULL by convention). At row level, `expected` is a
/// `record` rather than an `activity_log` row variable, so the function
/// names no table and needs no `search_path`.
const CHECK_UPDATE_FUNCTION: &str = "CREATE FUNCTION activity_log_check_update() RETURNS trigger
    LANGUAGE plpgsql AS $$
DECLARE
    expected record;
BEGIN
    IF current_setting('backend3.permanent_delete', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'activity_log is append-only: UPDATE is allowed only in a \
permanent-delete transaction, which sets backend3.permanent_delete with SET LOCAL (ADR 0011 §5)'
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF TG_LEVEL = 'STATEMENT' THEN
        RETURN NULL;
    END IF;
    expected := OLD;
    expected.ip := NULL;
    IF NEW *<> expected THEN
        RAISE EXCEPTION 'activity_log is append-only: a permanent-delete transaction may only \
set ip to NULL, and this UPDATE changes entry % otherwise (ADR 0011 §5)', OLD.id
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END
$$";

/// The four triggers. `BEFORE`, so a refused statement changes nothing and
/// fires no other trigger. Statement-level for `DELETE`, `TRUNCATE` and the
/// flag check of `UPDATE`, so they refuse even when no row matches;
/// PostgreSQL fires the statement-level `BEFORE` trigger before it reads any
/// row, and so before the row-level one.
const TRIGGERS: &[&str] = &[
    "CREATE TRIGGER activity_log_refuse_delete BEFORE DELETE ON activity_log \
     FOR EACH STATEMENT EXECUTE FUNCTION activity_log_refuse()",
    "CREATE TRIGGER activity_log_refuse_truncate BEFORE TRUNCATE ON activity_log \
     FOR EACH STATEMENT EXECUTE FUNCTION activity_log_refuse()",
    "CREATE TRIGGER activity_log_check_update_statement BEFORE UPDATE ON activity_log \
     FOR EACH STATEMENT EXECUTE FUNCTION activity_log_check_update()",
    "CREATE TRIGGER activity_log_check_update_row BEFORE UPDATE ON activity_log \
     FOR EACH ROW EXECUTE FUNCTION activity_log_check_update()",
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

/// The three CHECKs built from the lists above, in order:
///
/// - `activity_log_action_known`: exactly the actions of ADR 0011 §5.
/// - `activity_log_ip_account_only`: an `ip` only on an account action.
/// - `activity_log_label_content_only`: a `label` only on an entry that
///   names an `entity_type` other than a people one, and is not an account
///   action. `entity_type IS NOT NULL` comes first, so that `NOT IN` never
///   meets a NULL (`action` is `NOT NULL`).
///
/// A new action replaces the first, and for an account action the other two
/// as well (recipe 8 of `.claude/skills/backend3/SKILL.md`).
fn list_checks() -> [String; 3] {
    let every_action = [CONTENT_ACTIONS, ACCOUNT_ACTIONS, MEDIA_ACTIONS].concat();
    let account = sql_list(ACCOUNT_ACTIONS);
    let people = sql_list(PEOPLE_ENTITY_TYPES);
    [
        format!(
            "ALTER TABLE activity_log ADD CONSTRAINT activity_log_action_known \
             CHECK (action IN {})",
            sql_list(&every_action)
        ),
        format!(
            "ALTER TABLE activity_log ADD CONSTRAINT activity_log_ip_account_only \
             CHECK (ip IS NULL OR action IN {account})"
        ),
        format!(
            "ALTER TABLE activity_log ADD CONSTRAINT activity_log_label_content_only \
             CHECK (label IS NULL OR (entity_type IS NOT NULL AND entity_type NOT IN {people} \
             AND action NOT IN {account}))"
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
        for statement in list_checks() {
            db.execute_unprepared(&statement).await?;
        }
        db.execute_unprepared(REFUSE_FUNCTION).await?;
        db.execute_unprepared(CHECK_UPDATE_FUNCTION).await?;
        for statement in TRIGGERS {
            db.execute_unprepared(statement).await?;
        }
        Ok(())
    }

    /// Drops the table, which takes its constraints, indexes and triggers
    /// with it (`DROP TABLE` fires no `DELETE` or `TRUNCATE` trigger), then
    /// the two trigger functions.
    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        db.execute_unprepared("DROP TABLE activity_log").await?;
        db.execute_unprepared("DROP FUNCTION activity_log_refuse(), activity_log_check_update()")
            .await?;
        Ok(())
    }
}
