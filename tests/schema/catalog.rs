//! Reads the schema of a migrated database from `pg_catalog` as the
//! [`Object`]s that `backend3::domain::schema_snapshot::render` turns into
//! the snapshot text.
//!
//! The reader covers schema `public`, where the migrations create every
//! object, and leaves out whatever an extension owns (`pg_depend` entries of
//! type `e`, such as the functions of `unaccent`): the snapshot shows that
//! the extension is installed, not its internals. Definitions come from
//! PostgreSQL's own deparsing functions (`format_type`, `pg_get_expr`,
//! `pg_get_constraintdef`, `pg_get_indexdef`, `pg_get_triggerdef`,
//! `pg_get_functiondef`, `pg_get_partkeydef`), so the text is what the server
//! holds, not what a migration wrote. Versions that change with the
//! PostgreSQL image rather than with a migration (of an extension, of a
//! collation's ICU library) are not read. Rows come back in no particular
//! order; the formatter sorts them.
//!
//! What it reads, and so all that the snapshot shows:
//!
//! - the installed extensions (`pg_extension`) and the collations
//!   (`pg_collation`: provider, locale, a libc `LC_CTYPE` that differs, ICU
//!   tailoring rules, determinism);
//! - the ordinary and partitioned tables (`pg_class`), each with UNLOGGED,
//!   its parent (PARTITION OF with the bound, or INHERITS) and its partition
//!   key;
//! - their columns (`pg_attribute`, `pg_attrdef`): type, collation, NOT NULL,
//!   identity, generation expression and default;
//! - the comments on those tables and columns (`pg_description`);
//! - their constraints (`pg_constraint`; a NOT NULL one where the column line
//!   does not show all of it: NOT VALID, NO INHERIT or a name of its own),
//!   indexes (`pg_index`) and triggers (`pg_trigger`, with a mark on a
//!   disabled one);
//! - the functions and procedures (`pg_proc`).
//!
//! Anything else is not in the snapshot: for example other schemas, views,
//! sequences and the options of an identity column's sequence, types,
//! aggregates, operators, text search objects, rules, row-level security and
//! its policies, privileges, storage parameters and column storage settings,
//! statistics objects, and comments on any other object. The guard in
//! `guard.rs` fails the snapshot test when the database holds any of these,
//! and names each one. A migration that needs one extends this reader in the
//! same PR, and stops the guard from reporting it, so that the snapshot still
//! shows every change (ADR 0009).
//!
//! What the reader reads is decided in one place for the guard as well:
//! [`table_is_read`], [`function_is_read`] and [`collation_is_read`] are the
//! conditions of the queries below and of the guard's rules, so the two cannot
//! disagree about which tables, functions and collations the snapshot shows.
//! Of each, the snapshot shows what the list above names; the guard's
//! property rules report the settings it leaves out.

use backend3::domain::schema_snapshot::{Kind, Object};
use sea_orm::{ConnectionTrait, DbBackend, QueryResult, Statement, TryGetable};

/// The SQL condition that the object `oid` of the system catalog `class`
/// belongs to no extension.
///
/// - `class`: an SQL expression for the catalog's `regclass`, such as
///   `'pg_class'::regclass` or a column like `d.classoid`.
/// - `oid`: an SQL expression for the object's oid, such as `c.oid`.
pub fn not_in_extension(class: &str, oid: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM pg_depend e WHERE e.classid = {class} \
         AND e.objid = {oid} AND e.deptype = 'e')"
    )
}

/// The SQL condition that the `pg_class` row `c` is a table the reader reads:
/// an ordinary or partitioned table of schema `public` that no extension owns.
///
/// - `c`: the alias of a `pg_class` row in the query that uses the condition.
pub fn table_is_read(c: &str) -> String {
    format!(
        "{c}.relnamespace = 'public'::regnamespace AND {c}.relkind IN ('r', 'p') AND {}",
        not_in_extension("'pg_class'::regclass", &format!("{c}.oid"))
    )
}

/// The SQL condition that the `pg_proc` row `p` is a function the reader
/// reads: a function or procedure of schema `public` that no extension owns.
/// Aggregates are left out because `pg_get_functiondef` cannot write them.
///
/// - `p`: the alias of a `pg_proc` row in the query that uses the condition.
pub fn function_is_read(p: &str) -> String {
    format!(
        "{p}.pronamespace = 'public'::regnamespace AND {p}.prokind <> 'a' AND {}",
        not_in_extension("'pg_proc'::regclass", &format!("{p}.oid"))
    )
}

/// The SQL condition that the `pg_collation` row `c` is a collation the
/// reader reads: one of schema `public` that no extension owns.
///
/// - `c`: the alias of a `pg_collation` row in the query that uses the
///   condition.
pub fn collation_is_read(c: &str) -> String {
    format!(
        "{c}.collnamespace = 'public'::regnamespace AND {}",
        not_in_extension("'pg_collation'::regclass", &format!("{c}.oid"))
    )
}

/// What `CREATE TABLE` says about table `c` besides its columns and
/// constraints: UNLOGGED, the parent (PARTITION OF with the bound, or the
/// INHERITS list) and PARTITION BY. Each part is `NULL` when it does not
/// apply, so `concat_ws` leaves it out, and a plain table gets empty text.
/// It reads the parents `p` of [`TABLE_JOINS`].
const TABLE_CLAUSE: &str = "concat_ws(' ', \
     CASE WHEN c.relpersistence = 'u' THEN 'UNLOGGED' END, \
     CASE WHEN c.relispartition \
     THEN 'PARTITION OF ' || p.parents || ' ' || pg_get_expr(c.relpartbound, c.oid) \
     ELSE 'INHERITS (' || p.parents || ')' END, \
     'PARTITION BY ' || pg_get_partkeydef(c.oid))";

/// The `pg_class` row `c` of each table `t`, and its parents `p`, in the
/// order `INHERITS` lists them, for [`TABLE_CLAUSE`].
const TABLE_JOINS: &str = "JOIN pg_class c ON c.oid = t.oid \
     LEFT JOIN LATERAL (SELECT string_agg(i.inhparent::regclass::text, ', ' \
     ORDER BY i.inhseqno) AS parents FROM pg_inherits i WHERE i.inhrelid = c.oid) p ON true";

/// The column clause of `CREATE TABLE`: the type, a collation other than the
/// type's own, NOT NULL, then the identity (`attidentity` `a` or `d`), the
/// generation expression (`attgenerated` `s` or `v`) or the default. Both
/// codes are empty for none. It reads the joins of [`COLUMN_JOINS`].
const COLUMN_CLAUSE: &str = "format_type(a.atttypid, a.atttypmod) \
     || CASE WHEN a.attcollation <> ty.typcollation THEN ' COLLATE ' || co.collname::text ELSE '' END \
     || CASE WHEN a.attnotnull THEN ' NOT NULL' ELSE '' END \
     || CASE WHEN a.attidentity = 'a' THEN ' GENERATED ALWAYS AS IDENTITY' \
     WHEN a.attidentity = 'd' THEN ' GENERATED BY DEFAULT AS IDENTITY' \
     WHEN a.attgenerated = 's' THEN ' GENERATED ALWAYS AS (' || x.expr || ') STORED' \
     WHEN a.attgenerated = 'v' THEN ' GENERATED ALWAYS AS (' || x.expr || ') VIRTUAL' \
     ELSE COALESCE(' DEFAULT ' || x.expr, '') END";

/// The type, collation and default of each column `a`, for [`COLUMN_CLAUSE`].
const COLUMN_JOINS: &str = "JOIN pg_type ty ON ty.oid = a.atttypid \
     LEFT JOIN pg_collation co ON co.oid = a.attcollation \
     LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
     LEFT JOIN LATERAL (SELECT pg_get_expr(d.adbin, d.adrelid) AS expr) x ON true";

/// One query per kind. Each returns `table_name` (`NULL` outside tables),
/// `position`, `name` and `definition`, with every name cast to `text`
/// because the catalogs store them as `name`.
fn queries() -> Vec<(Kind, String)> {
    // The tables that the reader reads (`table_is_read`).
    let t = format!(
        "WITH t AS (SELECT c.oid, c.relname::text AS name FROM pg_class c WHERE {})",
        table_is_read("c")
    );
    // The live columns of those tables: the select list's start, the join
    // and the filter, with room for more joins in between.
    let columns =
        format!("{t} SELECT t.name AS table_name, a.attnum AS position, a.attname::text AS name");
    let from = "FROM t JOIN pg_attribute a ON a.attrelid = t.oid";
    let live = "WHERE a.attnum > 0 AND NOT a.attisdropped";
    let none = "NULL::text AS table_name, 0::int2 AS position";
    vec![
        (
            Kind::Extension,
            format!("SELECT {none}, extname::text AS name, '' AS definition FROM pg_extension"),
        ),
        // ICU and builtin collations keep their locale in `colllocale`
        // (PostgreSQL 17 and later), libc ones in `collcollate`, with an
        // LC_CTYPE (`collctype`) that may differ from it. An ICU collation
        // may carry tailoring rules (`collicurules`), which change the order
        // as much as the locale does (ADR 0020's `hu_icu` has none). Both
        // are NULL where they do not apply, so their parts are left out.
        // `collencoding` follows from the provider and the database, so a
        // migration cannot choose it.
        (
            Kind::Collation,
            format!(
                "SELECT {none}, c.collname::text AS name, \
                 'provider ' || CASE c.collprovider WHEN 'i' THEN 'icu' WHEN 'c' THEN 'libc' \
                 WHEN 'b' THEN 'builtin' ELSE c.collprovider::text END \
                 || COALESCE(', locale ' || COALESCE(c.colllocale, c.collcollate), '') \
                 || CASE WHEN c.collctype IS DISTINCT FROM c.collcollate \
                 THEN ', ctype ' || c.collctype ELSE '' END \
                 || COALESCE(', rules ' || quote_literal(c.collicurules), '') \
                 || CASE WHEN c.collisdeterministic THEN ', deterministic' \
                 ELSE ', nondeterministic' END AS definition \
                 FROM pg_collation c WHERE {}",
                collation_is_read("c")
            ),
        ),
        (
            Kind::Table,
            format!(
                "{t} SELECT t.name AS table_name, 0::int2 AS position, t.name, \
                 {TABLE_CLAUSE} AS definition FROM t {TABLE_JOINS}"
            ),
        ),
        (
            Kind::TableComment,
            format!(
                "{t} SELECT name AS table_name, 0::int2 AS position, name, definition \
                 FROM t, obj_description(t.oid, 'pg_class') AS definition \
                 WHERE definition IS NOT NULL"
            ),
        ),
        (
            Kind::Column,
            format!("{columns}, {COLUMN_CLAUSE} AS definition {from} {COLUMN_JOINS} {live}"),
        ),
        (
            Kind::ColumnComment,
            format!(
                "{columns}, col_description(t.oid, a.attnum) AS definition {from} {live} \
                 AND col_description(t.oid, a.attnum) IS NOT NULL"
            ),
        ),
        // PostgreSQL 18 records NOT NULL as constraints too
        // (`contype = 'n'`). The column line shows a plain one. One that is
        // NOT VALID or NO INHERIT is listed as well, because `attnotnull` is
        // true for it too and the column line alone would hide the
        // difference. So is one whose name is not the default
        // `<table>_<column>_not_null` (`CONSTRAINT x NOT NULL`, `RENAME
        // CONSTRAINT`), because the column line does not show the name. An
        // inherited one (not `conislocal`) carries its parent's name and
        // cannot be renamed on its own, so it is not listed. A default that
        // PostgreSQL had to shorten or number is listed too, which shows a
        // little more than needed and hides nothing.
        (
            Kind::Constraint,
            format!(
                "{t} SELECT t.name AS table_name, 0::int2 AS position, c.conname::text AS name, \
                 pg_get_constraintdef(c.oid) AS definition \
                 FROM t JOIN pg_constraint c ON c.conrelid = t.oid \
                 WHERE c.contype <> 'n' OR NOT c.convalidated OR c.connoinherit \
                 OR (c.conislocal AND c.conname::text IS DISTINCT FROM \
                 (SELECT t.name || '_' || a.attname || '_not_null' FROM pg_attribute a \
                 WHERE a.attrelid = c.conrelid AND a.attnum = c.conkey[1]))"
            ),
        ),
        (
            Kind::Index,
            format!(
                "{t} SELECT t.name AS table_name, 0::int2 AS position, i.relname::text AS name, \
                 pg_get_indexdef(i.oid) AS definition FROM t \
                 JOIN pg_index x ON x.indrelid = t.oid JOIN pg_class i ON i.oid = x.indexrelid"
            ),
        ),
        // Internal triggers are the ones behind foreign keys and deferrable
        // unique, primary key and exclusion constraints, which the
        // constraint lines show; the guard's `pg_trigger.internal` checks
        // their state and names. `pg_get_triggerdef` does not say whether a
        // trigger is disabled, so a state other than `O` (enabled) is added.
        (
            Kind::Trigger,
            format!(
                "{t} SELECT t.name AS table_name, 0::int2 AS position, g.tgname::text AS name, \
                 pg_get_triggerdef(g.oid) || CASE WHEN g.tgenabled = 'O' THEN '' \
                 ELSE ' -- tgenabled = ' || g.tgenabled::text END AS definition \
                 FROM t JOIN pg_trigger g ON g.tgrelid = t.oid WHERE NOT g.tgisinternal"
            ),
        ),
        (
            Kind::Function,
            format!(
                "SELECT {none}, \
                 p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS name, \
                 pg_get_functiondef(p.oid) AS definition FROM pg_proc p WHERE {}",
                function_is_read("p")
            ),
        ),
    ]
}

/// Runs `sql` and returns its rows; any error fails the test with the query.
async fn rows(db: &impl ConnectionTrait, sql: &str) -> Vec<QueryResult> {
    db.query_all_raw(Statement::from_string(DbBackend::Postgres, sql))
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
}

/// The value of column `name` of `row`.
fn get<T: TryGetable>(row: &QueryResult, name: &str) -> T {
    row.try_get("", name)
        .unwrap_or_else(|error| panic!("catalog column {name}: {error}"))
}

/// Reads every object that the snapshot shows from `db`.
///
/// - `db`: a connection to the migrated database, normally the per-test
///   database of `support::with_app`, or a transaction on it.
pub async fn read(db: &impl ConnectionTrait) -> Vec<Object> {
    let mut objects = Vec::new();
    for (kind, sql) in queries() {
        for row in rows(db, &sql).await {
            objects.push(Object {
                kind,
                table: get(&row, "table_name"),
                position: get(&row, "position"),
                name: get(&row, "name"),
                definition: get(&row, "definition"),
            });
        }
    }
    objects
}
