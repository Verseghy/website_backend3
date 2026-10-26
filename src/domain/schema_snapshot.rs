//! The text form of the database schema that the schema snapshot test pins
//! (ADR 0009).
//!
//! ADR 0009 requires that the migrations create exactly the planned tables,
//! constraints and indexes, and that "a schema snapshot test fails on any
//! unplanned change". That test (`tests/schema/`) migrates a fresh database,
//! reads the schema objects from `pg_catalog` as [`Object`]s with their SQL
//! definitions (its reader lists the kinds it covers), and compares
//! [`render`]'s text with a committed insta snapshot, so every migration
//! shows its effect in its own PR. A guard in the same test fails on any
//! object or setting of the database that the reader does not cover, within
//! the limits that the guard documents, so what a migration creates does not
//! escape the snapshot unseen. This module is the pure
//! half: it decides the order and the layout of the text and is tested
//! without a database. Only that test calls it; it lives here because
//! ADR 0008 puts every pure function in `src/domain/`.
//!
//! The text is built for review in a diff:
//!
//! - Every object is one line at column 0 that starts with its kind and its
//!   qualified name, for example `column users.email: text`. A changed object
//!   is therefore one changed line that names it, and its table if it has
//!   one, even where the diff shows no context. Nothing is aligned in
//!   columns, because padding would
//!   make one longer name change every line around it.
//! - A definition with several lines (a function body, a comment) continues
//!   on lines that start with [`CONTINUATION`], so no definition can produce
//!   a line that reads as another object.
//! - The order is decided here, not by the catalog queries: extensions,
//!   collations, then each table (its columns by position, each followed by
//!   its comment, then everything else by name), then functions. Names
//!   compare byte by byte, so the text depends neither on the order the rows
//!   arrived in nor on the database's collation.
//! - Trailing whitespace is dropped, so an editor that strips it cannot break
//!   the committed snapshot.
//!
//! The objects of [`FRAMEWORK_TABLES`] and the [`DEFAULT_EXTENSIONS`] are not
//! part of backend3's schema and are left out even when the catalog has them.

/// Tables that the frameworks own (ADR 0009, "Tables"): Loco creates
/// `pg_loco_queue` at run time with its own shape, and Sea-ORM records the
/// applied migrations in `seaql_migrations`. The snapshot leaves both out.
pub const FRAMEWORK_TABLES: [&str; 2] = ["pg_loco_queue", "seaql_migrations"];

/// Extensions that PostgreSQL installs in every new database, so that no
/// migration creates them: `plpgsql` comes from `template1`.
pub const DEFAULT_EXTENSIONS: [&str; 1] = ["plpgsql"];

/// The start of every line that continues the definition of the line above.
pub const CONTINUATION: &str = "    |";

/// The kinds of schema object, in the order they appear within a table (a
/// column comment directly follows its column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// An installed extension, by name.
    Extension,
    /// A collation; the definition gives its provider, locale, `LC_CTYPE` or
    /// ICU tailoring rules where it has them, and determinism.
    Collation,
    /// A table, by name; the definition is what `CREATE TABLE` says besides
    /// the columns and constraints (UNLOGGED, PARTITION OF, INHERITS,
    /// PARTITION BY), empty for a plain table.
    Table,
    /// The comment on a table (`COMMENT ON TABLE`); the name is the table's.
    TableComment,
    /// A column; the definition is its type and options as in
    /// `CREATE TABLE`.
    Column,
    /// The comment on a column (`COMMENT ON COLUMN`); the name is the
    /// column's.
    ColumnComment,
    /// A primary key, unique key, foreign key, CHECK or exclusion constraint,
    /// or a NOT NULL constraint that the column line does not fully show.
    Constraint,
    /// An index, with its `CREATE INDEX` statement.
    Index,
    /// A trigger, with its `CREATE TRIGGER` statement.
    Trigger,
    /// A function or procedure; the name is its signature, the definition
    /// its whole `CREATE OR REPLACE FUNCTION` statement.
    Function,
}

/// One schema object as the catalog describes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Object {
    /// What kind of object this is.
    pub kind: Kind,
    /// The table the object belongs to, for [`Kind::Table`] through
    /// [`Kind::Trigger`]; `None` for the other kinds.
    pub table: Option<String>,
    /// The column's position in its table (`attnum`) for [`Kind::Column`]
    /// and [`Kind::ColumnComment`], which orders the columns as `SELECT *`
    /// returns them; 0 for every other kind.
    pub position: i16,
    /// The object's name.
    pub name: String,
    /// The object's SQL definition as the catalog writes it, or the comment
    /// text; empty where the kind has none.
    pub definition: String,
}

/// Renders `objects` as the snapshot text described in the module
/// documentation.
///
/// - `objects`: the schema as read from the catalog, in any order.
///
/// Sections (the extensions, the collations, each table, the functions) are
/// separated by one empty line. The text ends with a newline unless it is
/// empty.
#[must_use]
pub fn render(objects: &[Object]) -> String {
    let mut shown: Vec<&Object> = objects.iter().filter(|object| is_shown(object)).collect();
    // The whole object is the key, so equal input renders equally even when
    // it holds duplicates.
    shown.sort_by_key(|object| {
        (
            section(object),
            // A column comment follows its own column, not the last one.
            if object.kind == Kind::ColumnComment {
                Kind::Column
            } else {
                object.kind
            },
            object.position,
            object.kind,
            &object.name,
            &object.table,
            &object.definition,
        )
    });

    let mut text = String::new();
    let mut previous = None;
    for object in shown {
        let current = section(object);
        if previous.is_some_and(|previous| previous != current) {
            text.push('\n');
        }
        previous = Some(current);

        let target = match (&object.table, object.kind) {
            (Some(table), kind) if is_in_table(kind) => format!("{table}.{}", object.name),
            _ => object.name.clone(),
        };
        let mut item = format!("{} {target}", label(object.kind));
        if !object.definition.is_empty() {
            item.push_str(": ");
            item.push_str(&object.definition);
        }
        // `pg_get_functiondef` ends its text with a newline; trimming the
        // whole item first keeps that from becoming an empty last line.
        for (index, part) in item.trim_end().split('\n').enumerate() {
            if index > 0 {
                text.push_str(CONTINUATION);
                text.push(' ');
            }
            text.push_str(part);
            text.truncate(text.trim_end().len());
            text.push('\n');
        }
    }
    text
}

/// Whether the snapshot shows `object`: not when it belongs to a
/// [`FRAMEWORK_TABLES`] table or is one of the [`DEFAULT_EXTENSIONS`].
fn is_shown(object: &Object) -> bool {
    let framework = object
        .table
        .as_deref()
        .is_some_and(|table| FRAMEWORK_TABLES.contains(&table));
    let default =
        object.kind == Kind::Extension && DEFAULT_EXTENSIONS.contains(&object.name.as_str());
    !framework && !default
}

/// Whether objects of `kind` are named after their table (`users.email`).
fn is_in_table(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Column | Kind::ColumnComment | Kind::Constraint | Kind::Index | Kind::Trigger
    )
}

/// The section `object` is rendered in: its rank in the text, and for the
/// kinds of a table, the table.
fn section(object: &Object) -> (u8, Option<&str>) {
    match object.kind {
        Kind::Extension => (0, None),
        Kind::Collation => (1, None),
        Kind::Function => (3, None),
        _ => (2, object.table.as_deref()),
    }
}

/// The words that start the line of an object of `kind`.
fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Extension => "extension",
        Kind::Collation => "collation",
        Kind::Table => "table",
        Kind::TableComment => "comment on table",
        Kind::Column => "column",
        Kind::ColumnComment => "comment on column",
        Kind::Constraint => "constraint",
        Kind::Index => "index",
        Kind::Trigger => "trigger",
        Kind::Function => "function",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind as K;
    use proptest::{collection::vec, option, prelude::*};

    const KINDS: [Kind; 10] = [
        Kind::Extension,
        Kind::Collation,
        Kind::Table,
        Kind::TableComment,
        Kind::Column,
        Kind::ColumnComment,
        Kind::Constraint,
        Kind::Index,
        Kind::Trigger,
        Kind::Function,
    ];

    fn object(kind: Kind, table: Option<&str>, position: i16, name: &str, def: &str) -> Object {
        Object {
            kind,
            table: table.map(str::to_owned),
            position,
            name: name.to_owned(),
            definition: def.to_owned(),
        }
    }

    const SEAQL: &str = "seaql_migrations";
    const FUNCTION: &str = concat!(
        "CREATE OR REPLACE FUNCTION public.reject()\n",
        " RETURNS trigger\n",
        "AS $f$\n",
        "\n",
        "BEGIN RAISE 'no'; END\n",
        "$f$\n",
    );

    /// One object of every kind, in no particular order, with a framework
    /// table and a default extension that the text leaves out.
    fn example() -> Vec<Object> {
        let at = |kind, position, name: &str, definition: &str| {
            object(kind, Some("labels"), position, name, definition)
        };
        vec![
            at(
                K::Index,
                0,
                "labels_pkey",
                "CREATE UNIQUE INDEX labels_pkey ON labels (id)",
            ),
            at(K::Column, 2, "name", "text COLLATE hu_icu NOT NULL"),
            object(K::Function, None, 0, "reject()", FUNCTION),
            at(
                K::ColumnComment,
                2,
                "name",
                "Shown in the admin.\nNever empty.",
            ),
            at(
                K::Column,
                3,
                "created_at",
                "timestamptz NOT NULL DEFAULT now()",
            ),
            at(K::Constraint, 0, "labels_pkey", "PRIMARY KEY (id)"),
            at(
                K::Column,
                1,
                "id",
                "bigint NOT NULL GENERATED BY DEFAULT AS IDENTITY",
            ),
            object(K::Table, Some(SEAQL), 0, SEAQL, ""),
            object(K::Column, Some(SEAQL), 1, "version", "text NOT NULL"),
            at(
                K::Trigger,
                0,
                "labels_guard",
                "CREATE TRIGGER labels_guard ON labels",
            ),
            at(
                K::Constraint,
                0,
                "labels_name_check",
                "CHECK ((name <> ''::text))",
            ),
            at(K::TableComment, 0, "labels", "Post labels"),
            at(K::Table, 0, "labels", ""),
            object(K::Table, Some("authors"), 0, "authors", ""),
            object(
                K::Collation,
                None,
                0,
                "hu_icu",
                "provider icu, locale hu, deterministic",
            ),
            object(K::Extension, None, 0, "unaccent", ""),
            object(K::Extension, None, 0, "plpgsql", ""),
        ]
    }

    const EXAMPLE: &str = "\
extension unaccent

collation hu_icu: provider icu, locale hu, deterministic

table authors

table labels
comment on table labels: Post labels
column labels.id: bigint NOT NULL GENERATED BY DEFAULT AS IDENTITY
column labels.name: text COLLATE hu_icu NOT NULL
comment on column labels.name: Shown in the admin.
    | Never empty.
column labels.created_at: timestamptz NOT NULL DEFAULT now()
constraint labels.labels_name_check: CHECK ((name <> ''::text))
constraint labels.labels_pkey: PRIMARY KEY (id)
index labels.labels_pkey: CREATE UNIQUE INDEX labels_pkey ON labels (id)
trigger labels.labels_guard: CREATE TRIGGER labels_guard ON labels

function reject(): CREATE OR REPLACE FUNCTION public.reject()
    |  RETURNS trigger
    | AS $f$
    |
    | BEGIN RAISE 'no'; END
    | $f$
";

    #[test]
    fn renders_every_kind_sorted_and_without_framework_objects() {
        assert_eq!(render(&example()), EXAMPLE);
        let mut reversed = example();
        reversed.reverse();
        assert_eq!(render(&reversed), EXAMPLE);
    }

    #[test]
    fn a_changed_column_is_one_readable_changed_line() {
        let mut changed = example();
        changed[1].definition = "character varying(64) COLLATE hu_icu".to_owned();
        let (before, after) = (render(&example()), render(&changed));
        let differences: Vec<(&str, &str)> = before
            .lines()
            .zip(after.lines())
            .filter(|(old, new)| old != new)
            .collect();
        assert_eq!(before.lines().count(), after.lines().count());
        assert_eq!(
            differences,
            [(
                "column labels.name: text COLLATE hu_icu NOT NULL",
                "column labels.name: character varying(64) COLLATE hu_icu",
            )]
        );
    }

    #[test]
    fn nothing_to_show_renders_as_empty_text() {
        assert_eq!(render(&[]), "");
        let framework_only = [
            object(K::Extension, None, 0, "plpgsql", ""),
            object(K::Table, Some("pg_loco_queue"), 0, "pg_loco_queue", ""),
            object(K::Index, Some(SEAQL), 0, "seaql_migrations_pkey", "x"),
        ];
        assert_eq!(render(&framework_only), "");
    }

    /// Text of three shapes: from a two-letter alphabet, so that equal names,
    /// tables and definitions are common and every part of the sort key is
    /// needed; with newlines, spaces and `|`, which look like the start of a
    /// continuation line; and any characters. `any::<String>()` would not do
    /// for the last: its default pattern `\PC*` excludes every control
    /// character, while `any::<char>()` favours NUL, `\t`, `\r`, `\n` and
    /// other special characters.
    fn text() -> impl Strategy<Value = String> {
        prop_oneof![
            "[ab]{0,2}",
            "[a |\n]{0,8}",
            vec(any::<char>(), 0..12).prop_map(String::from_iter),
        ]
    }

    /// Any object. Positions come from a small range for the same reason as
    /// the two-letter names of [`text`].
    fn arbitrary_object() -> impl Strategy<Value = Object> {
        let kind = proptest::sample::select(KINDS.to_vec());
        (kind, option::of(text()), 0i16..3, text(), text()).prop_map(
            |(kind, table, position, name, definition)| Object {
                kind,
                table,
                position,
                name,
                definition,
            },
        )
    }

    /// A list of objects and the same list in another order. `prop_shuffle`
    /// shrinks a failing case towards the original order.
    fn shuffled_objects() -> impl Strategy<Value = (Vec<Object>, Vec<Object>)> {
        vec(arbitrary_object(), 0..12)
            .prop_flat_map(|objects| (Just(objects.clone()), Just(objects).prop_shuffle()))
    }

    proptest! {
        #[test]
        fn rendering_ignores_the_input_order((objects, shuffled) in shuffled_objects()) {
            prop_assert_eq!(render(&shuffled), render(&objects));
        }

        #[test]
        fn every_object_is_one_line_that_starts_with_its_kind(
            objects in vec(arbitrary_object(), 0..12),
        ) {
            let text = render(&objects);
            let mut items = 0;
            for line in text.split_terminator('\n') {
                prop_assert_eq!(line, line.trim_end());
                if line.is_empty() || line.starts_with(CONTINUATION) {
                    continue;
                }
                items += 1;
                prop_assert!(
                    KINDS.iter().any(|&kind| line.starts_with(label(kind))),
                    "not an object line: {:?}", line
                );
            }
            prop_assert!(!text.starts_with('\n') && !text.contains("\n\n\n"));
            prop_assert_eq!(items, objects.iter().filter(|object| is_shown(object)).count());
        }
    }
}
