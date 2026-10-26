//! The guard that makes the schema snapshot complete (ADR 0009: "a schema
//! snapshot test fails on any unplanned change"; research: DECISIONS R15,
//! D80).
//!
//! The reader (`catalog.rs`) shows extensions, collations, tables with their
//! columns, comments, constraints, indexes and triggers, and functions. A
//! migration can create far more: views, sequences, types, rules, policies,
//! grants, storage parameters, operators and so on. Rather than render every
//! one of those, the snapshot test runs this guard first. It fails when the
//! migrated database holds anything that the reader does not show, and names
//! each object. A migration that really needs such an object extends the
//! reader and narrows the rule here in its own PR, so the snapshot keeps
//! showing every change.
//!
//! The guard is data, in four tables:
//!
//! - [`catalogs`]: every catalog whose rows are objects that a migration can
//!   create. For each one, the rows the reader shows. Every other row that
//!   was created after initdb is reported (`<catalog>.created`). Where the
//!   catalog has the columns, so is an owner other than the migrating role
//!   (`<catalog>.owner`) and privileges other than the initial ones
//!   (`<catalog>.privileges`).
//! - [`properties`]: what can be set on an object without creating a new one,
//!   and that the reader's text leaves out (storage parameters, row-level
//!   security, column settings, identity sequence options, the state of the
//!   triggers behind constraints, comments, extension membership, ...), and
//!   the state of the database itself (settings, comment, security label,
//!   connection options).
//! - [`EXTENSIONS`]: the expected extensions and every object each one owns.
//!   The reader leaves extension objects out, so a migration could hide an
//!   object by adding it to an extension; the guard compares the members,
//!   and reports an expected extension that is missing.
//! - [`OTHER_CATALOGS`]: the remaining catalogs of `pg_catalog`, each with the
//!   reason it needs no rule. A test lists `pg_catalog` and fails on a catalog
//!   that is in none of these tables, so a new PostgreSQL major cannot add a
//!   catalog unnoticed.
//!
//! What "created by a migration" means. initdb gives every object it creates
//! an oid below [`FIRST_NORMAL_OID`], so a row with a higher oid was created
//! later. Three kinds of such rows are not reported as objects of their own:
//!
//! - parts of another object (a `pg_depend` entry of type `i`, internal):
//!   a table's row type and TOAST table, an identity column's sequence, the
//!   index behind a constraint and the triggers behind a foreign key or a
//!   deferrable unique, primary key or exclusion constraint, a view's
//!   `_RETURN` rule, a range type's constructor functions. PostgreSQL
//!   creates and drops them with their object; the rules check the settings
//!   a migration can change on them (TOAST storage parameters, identity
//!   sequence options, the state and name of internal triggers, privileges,
//!   comments, and the owner where a part can have one of its own);
//! - members of an extension, which [`EXTENSIONS`] checks;
//! - temporary schemas (`pg_temp_N`, `pg_toast_temp_N`), which PostgreSQL
//!   creates for a session that uses a temporary table and keeps afterwards,
//!   owned by the bootstrap superuser. A temporary table itself is reported
//!   while it exists.
//!
//! Limits, so that the guard claims no more than it checks:
//!
//! - Objects that initdb creates (oid below [`FIRST_NORMAL_OID`]) are checked
//!   for their privileges (against `pg_init_privs`), for the owner and the
//!   comment of schema `public`, for the members of `plpgsql`, and for text
//!   search mappings to dictionaries created later. Other changes to them
//!   are not seen, because the database holds no copy of their initial
//!   state: a built-in function redefined with `CREATE OR REPLACE` (it keeps
//!   its oid, so an index over `lower` would silently change meaning), a
//!   built-in object dropped (a function, a text search configuration, a
//!   collation), a comment on one, or a mapping dropped from a built-in text
//!   search configuration. The privileges of `information_schema` and its
//!   views are not compared either, because initdb sets them without
//!   recording them in `pg_init_privs`.
//! - The members of an expected extension are compared by identity, owner,
//!   privileges and comments like every object, but not by content: a
//!   member's changed definition or setting (`ALTER FUNCTION ... STABLE`) is
//!   not seen.
//! - Of the parts of other objects, only the settings listed above are
//!   checked. A range type's constructor functions also take `RENAME`, `SET`
//!   and `COST`; range types are reported as objects today, so a slice that
//!   makes the reader show them shows their constructors too.
//! - Owners are compared with the role the test connects as, which is the
//!   role that ran the migrations. The guard and its cases assume a
//!   superuser, as compose and CI have (the role `loco`): the cases write
//!   catalog rows, and only an extension that a superuser creates belongs to
//!   that role member by member (a trusted extension that another role
//!   creates belongs to the bootstrap superuser).
//! - The rules read the catalogs as the DDL statements leave them, and the
//!   privileges rules compare with `pg_init_privs`. A migration that writes
//!   system catalogs directly could change both; backend3's migrations use
//!   DDL only.
//! - Cluster-wide objects (roles, tablespaces, configuration parameters) are
//!   not part of a database. Only this database's own rows of the shared
//!   catalogs are read.
//! - The test of [`OTHER_CATALOGS`] notices a new catalog, not a new column
//!   of a catalog the rules read; a PostgreSQL major-version bump reviews the
//!   rules against the release notes.

use std::fmt::Write as _;

use sea_orm::{ConnectionTrait, DbBackend, Statement};

use super::catalog::{collation_is_read, function_is_read, not_in_extension, table_is_read};

/// `FirstNormalObjectId` of PostgreSQL (`src/include/access/transam.h`):
/// initdb gives every object it creates a lower oid, so an object with this
/// oid or a higher one was created later, in `template1` or in the database.
pub const FIRST_NORMAL_OID: u32 = 16384;

/// The extensions that the migrated database is expected to hold, with the
/// identity of every object each one owns, as `pg_identify_object` writes
/// them (type, then the schema-qualified identity).
///
/// `plpgsql` comes from initdb (`template1`); `unaccent` from the
/// search-support migration (ADR 0020). A migration that adds an extension
/// adds it here with its members, which is the review of what it brings. A
/// PostgreSQL image whose extension version adds or drops a member fails the
/// guard until the list follows it. Neither script comments its members, so
/// `pg_description.comment` reports any comment on one; an extension whose
/// script does would also need those comments exempted there.
pub const EXTENSIONS: &[(&str, &[&str])] = &[
    (
        "plpgsql",
        &[
            "function pg_catalog.plpgsql_call_handler()",
            "function pg_catalog.plpgsql_inline_handler(pg_catalog.internal)",
            "function pg_catalog.plpgsql_validator(pg_catalog.oid)",
            "language plpgsql",
        ],
    ),
    (
        "unaccent",
        &[
            "function public.unaccent(pg_catalog.regdictionary,pg_catalog.text)",
            "function public.unaccent(pg_catalog.text)",
            "function public.unaccent_init(pg_catalog.internal)",
            "function public.unaccent_lexize(pg_catalog.internal,pg_catalog.internal,\
             pg_catalog.internal,pg_catalog.internal)",
            "text search dictionary public.unaccent",
            "text search template public.unaccent",
        ],
    ),
];

/// The catalogs of `pg_catalog` that no rule reads, and why none is needed.
pub const OTHER_CATALOGS: &[(&str, &str)] = &[
    (
        "pg_aggregate",
        "the aggregate part of a pg_proc function; pg_proc.created reports every aggregate",
    ),
    (
        "pg_enum",
        "the labels of an enum type, which pg_type.created reports",
    ),
    (
        "pg_range",
        "the range part of a type, which pg_type.created reports",
    ),
    (
        "pg_inherits",
        "read: the table line's INHERITS and PARTITION OF",
    ),
    (
        "pg_partitioned_table",
        "read: the table line's PARTITION BY",
    ),
    (
        "pg_foreign_table",
        "the foreign part of a relation, which pg_class.created reports",
    ),
    (
        "pg_largeobject",
        "the data of large objects, which pg_largeobject_metadata.created reports",
    ),
    (
        "pg_statistic",
        "planner statistics that ANALYZE gathers: data, not schema",
    ),
    (
        "pg_statistic_ext_data",
        "the statistics that ANALYZE gathers for a pg_statistic_ext object: data, not schema",
    ),
    (
        "pg_subscription_rel",
        "the table states of a subscription, which pg_subscription.created reports",
    ),
    (
        "pg_init_privs",
        "the initial privileges that the privileges rules compare with",
    ),
    (
        "pg_shdepend",
        "references from this database's objects to roles and tablespaces, which the \
         owner, privileges and tablespace rules read where the objects store them",
    ),
    ("pg_authid", "roles: cluster-wide, not part of a database"),
    (
        "pg_auth_members",
        "role memberships: cluster-wide, not part of a database",
    ),
    (
        "pg_tablespace",
        "tablespaces: cluster-wide; pg_class.tablespace reports a relation stored in one",
    ),
    (
        "pg_parameter_acl",
        "privileges on configuration parameters: cluster-wide, not part of a database",
    ),
    (
        "pg_replication_origin",
        "replication origins: cluster-wide, not part of a database",
    ),
];

/// A system catalog whose rows are objects that a migration can create.
pub struct Catalog {
    /// The catalog, for example `pg_class`.
    pub name: &'static str,
    /// The class that `pg_describe_object`, `pg_depend` and `pg_init_privs`
    /// name these objects by: the catalog itself, except for large objects,
    /// which they name by `pg_largeobject`.
    pub class: &'static str,
    /// The SQL condition on the catalog row `o` for the rows that the reader
    /// shows, or `None` when it shows none.
    pub read: Option<String>,
    /// The SQL condition on the catalog row `o` for rows that are not objects
    /// of their own and that no rule of this catalog reports, or `None`: the
    /// temporary schemas (see the module documentation).
    pub skip: Option<&'static str>,
    /// The owner column, when a migration can give the objects an owner.
    pub owner: Option<&'static str>,
    /// Whether a part of another object in this catalog can have an owner of
    /// its own. Only functions can: `ALTER FUNCTION` gives a range type's
    /// constructor another owner, and `ALTER TYPE ... OWNER` leaves the
    /// constructors alone. For the parts in `pg_class` and `pg_type` (a TOAST
    /// table, an identity sequence, a constraint's index, a row, array or
    /// multirange type) PostgreSQL refuses it, or ignores it with a warning,
    /// and moves them with their object, so only the object is reported.
    pub parts_have_owners: bool,
    /// The privileges column (`aclitem[]`), when the objects take GRANT.
    pub privileges: Option<&'static str>,
    /// The column that holds the object's schema, when the objects live in
    /// one; the privileges rule needs it to recognise `information_schema`.
    pub schema: Option<&'static str>,
    /// Whether every row counts as created by a migration, whatever its oid.
    /// Only for large objects: initdb creates none, and `lo_create` accepts
    /// any free oid, also one below [`FIRST_NORMAL_OID`].
    pub every_row: bool,
}

impl Catalog {
    /// A catalog that `pg_describe_object` names by itself, with no row the
    /// reader shows, no owner, no privileges and no schema; the fields are
    /// then set as the catalog needs.
    fn plain(name: &'static str) -> Self {
        Self {
            name,
            class: name,
            read: None,
            skip: None,
            owner: None,
            parts_have_owners: false,
            privileges: None,
            schema: None,
            every_row: false,
        }
    }

    /// The SQL condition that the catalog row `o` is one of the rows that
    /// [`Catalog::skip`] leaves out; `false` when the catalog skips none.
    fn skipped(&self) -> String {
        self.skip
            .map_or_else(|| "false".to_owned(), |skip| format!("({skip})"))
    }

    /// The SQL condition that the catalog row `o` was created after initdb.
    fn created(&self) -> String {
        if self.every_row {
            "true".to_owned()
        } else {
            format!("o.oid >= {FIRST_NORMAL_OID}")
        }
    }

    /// The SQL condition that the catalog row `o` is `information_schema`
    /// itself or one of its objects.
    fn in_information_schema(&self) -> String {
        match (self.name, self.schema) {
            ("pg_namespace", _) => "o.nspname = 'information_schema'".to_owned(),
            (_, Some(schema)) => format!("o.{schema} = 'information_schema'::regnamespace"),
            (_, None) => "false".to_owned(),
        }
    }
}

/// The catalogs of objects, with what the reader shows of each.
pub fn catalogs() -> Vec<Catalog> {
    let named = |name, owner, privileges| Catalog {
        owner: Some(owner),
        privileges,
        ..Catalog::plain(name)
    };
    let read = |catalog: Catalog, condition: String| Catalog {
        read: Some(condition),
        ..catalog
    };
    let in_schema = |catalog: Catalog, schema| Catalog {
        schema: Some(schema),
        ..catalog
    };
    vec![
        // Schema `public` (an initdb object) is read. The temporary schemas
        // are skipped by every rule, the owner rule included: the bootstrap
        // superuser owns them, which is the test role in compose and CI but
        // need not be (the module documentation explains them).
        Catalog {
            skip: Some("o.nspname ~ '^pg_(toast_)?temp_[0-9]+$'"),
            ..read(
                named("pg_namespace", "nspowner", Some("nspacl")),
                "o.nspname = 'public'".to_owned(),
            )
        },
        // The tables, their indexes, and the indexes of TOAST tables: the
        // TOAST table is an internal part of its table, while its index
        // depends on the TOAST table's columns only automatically (`a`).
        read(
            in_schema(
                named("pg_class", "relowner", Some("relacl")),
                "relnamespace",
            ),
            format!(
                "EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = o.oid AND {}) \
                 OR EXISTS (SELECT 1 FROM pg_index x JOIN pg_class c ON c.oid = x.indrelid \
                 WHERE x.indexrelid = o.oid AND (c.relkind = 't' OR {}))",
                table_is_read("c"),
                table_is_read("c")
            ),
        ),
        in_schema(named("pg_type", "typowner", Some("typacl")), "typnamespace"),
        Catalog {
            parts_have_owners: true,
            ..read(
                in_schema(named("pg_proc", "proowner", Some("proacl")), "pronamespace"),
                function_is_read("o"),
            )
        },
        read(
            named("pg_collation", "collowner", None),
            collation_is_read("o"),
        ),
        // A table's constraints (also its NOT NULL constraints, which the
        // column line shows, with a line of their own where it does not show
        // all), defaults and triggers are read; those of anything else (a
        // domain, a view) are reported. The internal triggers behind
        // constraints are parts of them, whose state `pg_trigger.internal`
        // checks.
        read(
            Catalog::plain("pg_constraint"),
            format!(
                "EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = o.conrelid AND {})",
                table_is_read("c")
            ),
        ),
        read(
            Catalog::plain("pg_attrdef"),
            format!(
                "EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = o.adrelid AND {})",
                table_is_read("c")
            ),
        ),
        read(
            Catalog::plain("pg_trigger"),
            format!(
                "NOT o.tgisinternal AND EXISTS (SELECT 1 FROM pg_class c \
                 WHERE c.oid = o.tgrelid AND {})",
                table_is_read("c")
            ),
        ),
        Catalog::plain("pg_rewrite"),
        Catalog::plain("pg_policy"),
        named("pg_statistic_ext", "stxowner", None),
        named("pg_operator", "oprowner", None),
        named("pg_opclass", "opcowner", None),
        named("pg_opfamily", "opfowner", None),
        Catalog::plain("pg_amop"),
        Catalog::plain("pg_amproc"),
        Catalog::plain("pg_am"),
        Catalog::plain("pg_cast"),
        named("pg_conversion", "conowner", None),
        named("pg_language", "lanowner", Some("lanacl")),
        Catalog::plain("pg_transform"),
        named("pg_ts_config", "cfgowner", None),
        named("pg_ts_dict", "dictowner", None),
        Catalog::plain("pg_ts_parser"),
        Catalog::plain("pg_ts_template"),
        named("pg_event_trigger", "evtowner", None),
        named("pg_foreign_data_wrapper", "fdwowner", Some("fdwacl")),
        named("pg_foreign_server", "srvowner", Some("srvacl")),
        Catalog::plain("pg_user_mapping"),
        named("pg_publication", "pubowner", None),
        Catalog::plain("pg_publication_rel"),
        Catalog::plain("pg_publication_namespace"),
        // `defaclrole` is the role whose new objects get the defaults, not an
        // owner, and `defaclacl` the defaults themselves: the row is
        // reported whatever they hold.
        Catalog::plain("pg_default_acl"),
        Catalog {
            class: "pg_largeobject",
            every_row: true,
            ..named("pg_largeobject_metadata", "lomowner", Some("lomacl"))
        },
    ]
}

/// One query of the guard.
pub struct Rule {
    /// `<catalog>.<check>`, for example `pg_class.created` or
    /// `pg_class.row_security`; the tests name the rules they expect by it.
    pub id: String,
    /// The catalog the rule reads, for the test that every catalog of
    /// `pg_catalog` is accounted for.
    pub catalog: &'static str,
    /// A query that returns two text columns for each finding: the object
    /// (as `pg_describe_object` names it) and what the snapshot does not show
    /// of it.
    pub sql: String,
}

/// A rule over `catalog` named `<catalog>.<check>`.
fn rule(catalog: &'static str, check: &str, sql: String) -> Rule {
    Rule {
        id: format!("{catalog}.{check}"),
        catalog,
        sql,
    }
}

/// The SQL condition that the row `o` of a catalog whose objects are named by
/// `class` is an internal part of another object (see the module
/// documentation).
fn is_internal_part(class: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = '{class}'::regclass \
         AND i.objid = o.oid AND i.deptype = 'i')"
    )
}

/// The oid of the database that the guard runs in, as an SQL expression.
const THIS_DATABASE: &str = "(SELECT oid FROM pg_database WHERE datname = current_database())";

/// The oid of the role the test connects as, which ran the migrations, as an
/// SQL expression. It looks the name up rather than casting it to `regrole`,
/// which would parse it again as an identifier: fold a mixed-case name to
/// another role's, and fail on a name such as `test-role`.
pub const MIGRATING_ROLE: &str = "(SELECT oid FROM pg_roles WHERE rolname = current_user)";

/// The rules that [`catalogs`] implies: for each catalog, `created`, and
/// `owner` and `privileges` where it has those columns.
fn catalog_rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    for catalog in catalogs() {
        let Catalog { name, class, .. } = catalog;
        let created = catalog.created();
        let skipped = catalog.skipped();
        let outside_extension = not_in_extension(&format!("'{class}'::regclass"), "o.oid");
        let internal = is_internal_part(class);
        let describe = format!("pg_describe_object('{class}'::regclass, o.oid, 0)");
        let unread = catalog
            .read
            .as_ref()
            .map_or_else(|| "true".to_owned(), |read| format!("NOT ({read})"));
        rules.push(rule(
            name,
            "created",
            format!(
                "SELECT {describe}, 'not read' FROM {name} o \
                 WHERE {created} AND {unread} AND NOT {skipped} AND {outside_extension} \
                 AND NOT {internal}"
            ),
        ));
        // Extension members are compared like every object: a migration can
        // give one another owner (`ALTER FUNCTION unaccent(text) OWNER TO`),
        // and an extension that the migrating role creates as a superuser
        // gives it all its members. A part of another object is left to its
        // object where it cannot have an owner of its own
        // (`parts_have_owners`).
        if let Some(owner) = catalog.owner {
            let part = if catalog.parts_have_owners {
                String::new()
            } else {
                format!(" AND NOT {internal}")
            };
            rules.push(rule(
                name,
                "owner",
                format!(
                    "SELECT {describe}, format('owned by %s, not by the migrating role %s', \
                     o.{owner}::regrole, current_user) FROM {name} o \
                     WHERE {created} AND o.{owner} <> {MIGRATING_ROLE} AND NOT {skipped}{part}"
                ),
            ));
        }
        // Every object, also an initdb one, is compared with its initial
        // privileges, which `pg_init_privs` records for initdb and extension
        // objects; any other object starts with none (NULL). initdb grants
        // on information_schema and its views after it records the others,
        // so for those, without a record, there is nothing to compare with.
        if let Some(acl) = catalog.privileges {
            let information_schema = catalog.in_information_schema();
            rules.push(rule(
                name,
                "privileges",
                format!(
                    "SELECT {describe}, 'privileges ' || COALESCE(o.{acl}::text, 'reset') \
                     FROM {name} o LEFT JOIN pg_init_privs p ON p.classoid = '{class}'::regclass \
                     AND p.objoid = o.oid AND p.objsubid = 0 \
                     WHERE o.{acl} IS DISTINCT FROM p.initprivs AND NOT {skipped} \
                     AND NOT (o.oid < {FIRST_NORMAL_OID} AND p.objoid IS NULL \
                     AND {information_schema})"
                ),
            ));
        }
    }
    rules
}

/// The rules for what can be set on an object, or on the database, without
/// creating a new object (see the module documentation), in groups by what
/// they read. Each rule says in a comment what it reports.
pub fn properties() -> Vec<Rule> {
    let mut rules = relation_rules();
    rules.extend(column_rules());
    rules.extend(index_identity_and_trigger_rules());
    rules.extend(comment_label_and_extension_rules());
    rules.extend(initdb_object_rules());
    rules.extend(database_rules());
    rules
}

/// The settings of a relation (`pg_class`) that the table line leaves out.
fn relation_rules() -> Vec<Rule> {
    let created = format!("o.oid >= {FIRST_NORMAL_OID}");
    let table = "pg_describe_object('pg_class'::regclass, o.oid, 0)";
    vec![
        // Storage parameters (`WITH (fillfactor = 70)`) of a relation other
        // than an index, whose parameters `pg_get_indexdef` shows, or a
        // TOAST table, which `toast_storage_parameters` reports by its table.
        rule(
            "pg_class",
            "storage_parameters",
            format!(
                "SELECT {table}, 'storage parameters ' || array_to_string(o.reloptions, ', ') \
                 FROM pg_class o WHERE {created} AND o.relkind NOT IN ('i', 'I', 't') \
                 AND o.reloptions IS NOT NULL"
            ),
        ),
        // The `toast.` storage parameters, reported by the table they belong
        // to, because a TOAST table's name carries an oid.
        rule(
            "pg_class",
            "toast_storage_parameters",
            format!(
                "SELECT {table}, 'TOAST storage parameters ' || array_to_string(t.reloptions, ', ') \
                 FROM pg_class o JOIN pg_class t ON t.oid = o.reltoastrelid \
                 WHERE {created} AND t.reloptions IS NOT NULL"
            ),
        ),
        // ENABLE and FORCE ROW LEVEL SECURITY, each on its own.
        rule(
            "pg_class",
            "row_security",
            format!(
                "SELECT {table}, concat_ws(' and ', \
                 CASE WHEN o.relrowsecurity THEN 'row-level security enabled' END, \
                 CASE WHEN o.relforcerowsecurity THEN 'row-level security forced' END) \
                 FROM pg_class o WHERE o.relrowsecurity OR o.relforcerowsecurity"
            ),
        ),
        // REPLICA IDENTITY other than DEFAULT.
        rule(
            "pg_class",
            "replica_identity",
            format!(
                "SELECT {table}, 'replica identity ' || CASE o.relreplident \
                 WHEN 'n' THEN 'NOTHING' WHEN 'f' THEN 'FULL' WHEN 'i' THEN 'USING INDEX' \
                 ELSE o.relreplident::text END \
                 FROM pg_class o WHERE {created} AND o.relkind IN ('r', 'p') \
                 AND o.relreplident <> 'd'"
            ),
        ),
        // A table access method other than heap, or any on a partitioned
        // table, which has none (0) unless one is set for its partitions.
        rule(
            "pg_class",
            "access_method",
            format!(
                "SELECT {table}, 'table access method ' || m.amname \
                 FROM pg_class o JOIN pg_am m ON m.oid = o.relam \
                 WHERE {created} AND o.relkind IN ('r', 'p') \
                 AND (o.relkind = 'p' OR m.amname <> 'heap')"
            ),
        ),
        // A relation stored outside the database's default tablespace (0).
        rule(
            "pg_class",
            "tablespace",
            format!(
                "SELECT {table}, 'tablespace ' || s.spcname \
                 FROM pg_class o JOIN pg_tablespace s ON s.oid = o.reltablespace WHERE {created}"
            ),
        ),
        // CREATE TABLE ... OF a composite type.
        rule(
            "pg_class",
            "typed_table",
            format!(
                "SELECT {table}, 'typed table OF ' || o.reloftype::regtype \
                 FROM pg_class o WHERE o.reloftype <> 0"
            ),
        ),
    ]
}

/// The settings of a column (`pg_attribute`) that the column line leaves
/// out.
fn column_rules() -> Vec<Rule> {
    let created = format!("o.oid >= {FIRST_NORMAL_OID}");
    let column = "pg_describe_object('pg_class'::regclass, a.attrelid, a.attnum)";
    // The live columns of every relation created after initdb.
    let columns = format!(
        "FROM pg_attribute a JOIN pg_class o ON o.oid = a.attrelid \
         WHERE {created} AND a.attnum > 0 AND NOT a.attisdropped"
    );
    vec![
        // Column privileges, compared as in the `privileges` rules. No column
        // of an initdb relation has privileges without a record, so none is
        // left out.
        rule(
            "pg_attribute",
            "privileges",
            format!(
                "SELECT {column}, 'privileges ' || COALESCE(a.attacl::text, 'reset') \
                 FROM pg_attribute a LEFT JOIN pg_init_privs p \
                 ON p.classoid = 'pg_class'::regclass AND p.objoid = a.attrelid \
                 AND p.objsubid = a.attnum \
                 WHERE a.attnum > 0 AND NOT a.attisdropped \
                 AND a.attacl IS DISTINCT FROM p.initprivs"
            ),
        ),
        // ALTER COLUMN ... SET (n_distinct = ...) and the like.
        rule(
            "pg_attribute",
            "options",
            format!(
                "SELECT {column}, 'column options ' || array_to_string(a.attoptions, ', ') \
                 {columns} AND a.attoptions IS NOT NULL"
            ),
        ),
        // ALTER COLUMN ... SET STATISTICS, also on an index's expression
        // column. Since PostgreSQL 17 the default is NULL.
        rule(
            "pg_attribute",
            "statistics_target",
            format!(
                "SELECT {column}, 'statistics target ' || a.attstattarget \
                 {columns} AND a.attstattarget IS NOT NULL"
            ),
        ),
        // ALTER COLUMN ... SET STORAGE on a table's column. A column starts
        // with its type's storage; index and TOAST columns differ by design.
        rule(
            "pg_attribute",
            "storage",
            format!(
                "SELECT {column}, 'storage ' || CASE a.attstorage WHEN 'p' THEN 'PLAIN' \
                 WHEN 'e' THEN 'EXTERNAL' WHEN 'm' THEN 'MAIN' WHEN 'x' THEN 'EXTENDED' \
                 ELSE a.attstorage::text END \
                 {columns} AND o.relkind IN ('r', 'p') \
                 AND a.attstorage <> (SELECT t.typstorage FROM pg_type t WHERE t.oid = a.atttypid)"
            ),
        ),
        // A column's COMPRESSION method; empty is the server default.
        rule(
            "pg_attribute",
            "compression",
            format!(
                "SELECT {column}, 'compression ' || CASE a.attcompression WHEN 'p' THEN 'pglz' \
                 WHEN 'l' THEN 'lz4' ELSE a.attcompression::text END \
                 {columns} AND a.attcompression <> ''"
            ),
        ),
    ]
}

/// The state of an index (`pg_index`) that `pg_get_indexdef` leaves out, the
/// options of an identity column's sequence (`pg_sequence`), and the state of
/// the internal triggers behind constraints (`pg_trigger`), which the lines
/// of their tables leave out.
fn index_identity_and_trigger_rules() -> Vec<Rule> {
    let index = "pg_describe_object('pg_class'::regclass, x.indexrelid, 0)";
    vec![
        // An index that is not valid (CREATE INDEX ON ONLY a partitioned
        // table, a failed CREATE INDEX CONCURRENTLY), not ready or being
        // dropped: `pg_get_indexdef` writes it like a working one.
        rule(
            "pg_index",
            "state",
            format!(
                "SELECT {index}, 'index ' || concat_ws(', ', \
                 CASE WHEN NOT x.indisvalid THEN 'not valid' END, \
                 CASE WHEN NOT x.indisready THEN 'not ready' END, \
                 CASE WHEN NOT x.indislive THEN 'being dropped' END) \
                 FROM pg_index x WHERE NOT (x.indisvalid AND x.indisready AND x.indislive)"
            ),
        ),
        // ALTER TABLE ... CLUSTER ON.
        rule(
            "pg_index",
            "clustered",
            format!(
                "SELECT {index}, 'CLUSTER ON this index' FROM pg_index x WHERE x.indisclustered"
            ),
        ),
        // The options of an identity column's sequence: the column line says
        // only GENERATED ... AS IDENTITY, so anything other than the
        // defaults of CREATE TABLE is reported, by the column. Its data type
        // starts as the column's, and `ALTER COLUMN ... TYPE` changes both;
        // `ALTER SEQUENCE ... AS` changes only the sequence's. The default
        // name is `<table>_<column>_seq`; a name PostgreSQL has to shorten or
        // number is reported as well. The schema is not compared: PostgreSQL
        // keeps an identity sequence in its table's schema (CREATE TABLE
        // finds none elsewhere, and ALTER SEQUENCE ... SET SCHEMA refuses to
        // move an owned sequence).
        rule(
            "pg_sequence",
            "identity",
            "SELECT pg_describe_object('pg_class'::regclass, d.refobjid, d.refobjsubid), \
             format('identity sequence %s: AS %s, START %s, INCREMENT %s, MINVALUE %s, \
             MAXVALUE %s, CACHE %s%s%s', s.relname, q.seqtypid::regtype, q.seqstart, \
             q.seqincrement, q.seqmin, \
             q.seqmax, q.seqcache, CASE WHEN q.seqcycle THEN ', CYCLE' ELSE '' END, \
             CASE WHEN s.relpersistence <> t.relpersistence THEN CASE s.relpersistence \
             WHEN 'u' THEN ', UNLOGGED' ELSE ', LOGGED' END ELSE '' END) \
             FROM pg_sequence q JOIN pg_class s ON s.oid = q.seqrelid \
             JOIN pg_depend d ON d.classid = 'pg_class'::regclass AND d.objid = s.oid \
             AND d.refclassid = 'pg_class'::regclass AND d.deptype = 'i' \
             JOIN pg_class t ON t.oid = d.refobjid \
             JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = d.refobjsubid \
             WHERE q.seqtypid <> a.atttypid \
             OR q.seqstart <> 1 OR q.seqincrement <> 1 OR q.seqmin <> 1 \
             OR q.seqmax <> CASE q.seqtypid WHEN 'int2'::regtype THEN 32767 \
             WHEN 'int4'::regtype THEN 2147483647 ELSE 9223372036854775807 END \
             OR q.seqcache <> 1 OR q.seqcycle OR s.relpersistence <> t.relpersistence \
             OR s.relname <> t.relname || '_' || a.attname || '_seq'"
                .to_owned(),
        ),
        // The internal triggers that enforce a foreign key, or recheck a
        // deferrable unique, primary key or exclusion constraint. The
        // constraint line shows the constraint, not whether its triggers
        // fire: `ALTER TABLE ... DISABLE TRIGGER ALL` (or `ENABLE REPLICA`
        // or `ENABLE ALWAYS` on one of them) changes what the constraint
        // enforces. PostgreSQL names each `<kind>_<the trigger's own oid>`;
        // triggers fire in name order, so a renamed one is reported too.
        // Every internal trigger of PostgreSQL 18 belongs to a constraint,
        // which names the finding (a NULL name would fail the rule loudly).
        // PostgreSQL 18 has the four states written out; the ELSE writes the
        // code of any later one rather than leave it out of the text.
        rule(
            "pg_trigger",
            "internal",
            "SELECT pg_describe_object('pg_constraint'::regclass, t.tgconstraint, 0), \
             format('internal trigger %s on %s: %s', t.tgfoid::regproc, t.tgrelid::regclass, \
             concat_ws(', ', CASE t.tgenabled WHEN 'O' THEN NULL WHEN 'D' THEN 'disabled' \
             WHEN 'R' THEN 'enabled for replicas only' WHEN 'A' THEN 'enabled always' \
             ELSE 'tgenabled = ' || t.tgenabled::text END, \
             CASE WHEN NOT g.generated THEN 'renamed to ' || quote_ident(t.tgname) END)) \
             FROM pg_trigger t CROSS JOIN LATERAL (SELECT t.tgname ~ \
             ('^(RI_ConstraintTrigger_[ac]|PK_ConstraintTrigger|Unique_ConstraintTrigger)_' \
             || t.oid::text || '$') AS generated) g \
             WHERE t.tgisinternal AND (t.tgenabled <> 'O' OR NOT g.generated)"
                .to_owned(),
        ),
    ]
}

/// Comments and security labels on objects, dependencies on extensions, and
/// the extensions themselves against [`EXTENSIONS`].
fn comment_label_and_extension_rules() -> Vec<Rule> {
    let extensions = EXTENSIONS
        .iter()
        .map(|(name, _)| format!("({})", sql_text(name)))
        .collect::<Vec<_>>()
        .join(", ");
    let members = EXTENSIONS
        .iter()
        .flat_map(|(name, members)| {
            members
                .iter()
                .map(move |member| format!("({}, {})", sql_text(name), sql_text(member)))
        })
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        // ALTER ... DEPENDS ON EXTENSION: the object is dropped with the
        // extension.
        rule(
            "pg_depend",
            "extension_dependency",
            "SELECT pg_describe_object(d.classid, d.objid, d.objsubid), \
             'depends on extension ' || x.extname \
             FROM pg_depend d JOIN pg_extension x ON x.oid = d.refobjid WHERE d.deptype = 'x'"
                .to_owned(),
        ),
        // A comment on an object created after initdb, other than the table
        // and column comments the reader shows and an extension's own
        // (`pg_extension.comment`). A comment on a system column of a read
        // table (`COMMENT ON COLUMN users.ctid`, `objsubid` below 0) is not
        // shown, so it is reported. So is a comment on an extension member:
        // the scripts of the EXTENSIONS set none, so any is a migration's.
        rule(
            "pg_description",
            "comment",
            format!(
                "SELECT pg_describe_object(d.classoid, d.objoid, d.objsubid), \
                 'comment ' || quote_literal(d.description) FROM pg_description d \
                 WHERE d.objoid >= {FIRST_NORMAL_OID} AND d.classoid <> 'pg_extension'::regclass \
                 AND NOT (d.classoid = 'pg_class'::regclass AND d.objsubid >= 0 \
                 AND EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = d.objoid AND {}))",
                table_is_read("c")
            ),
        ),
        // An extension that EXTENSIONS does not list. The reader shows its
        // name but none of its objects.
        rule(
            "pg_extension",
            "expected",
            format!(
                "SELECT pg_describe_object('pg_extension'::regclass, x.oid, 0), \
                 'not one of the expected extensions of tests/schema/guard.rs' \
                 FROM pg_extension x WHERE x.extname NOT IN (SELECT name FROM (VALUES {extensions}) \
                 AS e(name))"
            ),
        ),
        // An extension that EXTENSIONS lists but the database lacks. The
        // snapshot shows a missing `unaccent`, but not a missing `plpgsql`,
        // which the formatter leaves out; the members rule compares only
        // the extensions that are there.
        rule(
            "pg_extension",
            "missing",
            format!(
                "SELECT 'extension ' || e.name, \
                 'expected by tests/schema/guard.rs, but not installed' \
                 FROM (VALUES {extensions}) AS e(name) \
                 WHERE e.name NOT IN (SELECT extname::text FROM pg_extension)"
            ),
        ),
        // The members of an expected extension against EXTENSIONS, both
        // ways: an object added to it (ALTER EXTENSION ... ADD) would be
        // hidden from the reader, and one missing was dropped from it.
        rule(
            "pg_extension",
            "members",
            format!(
                "WITH expected(extension, member) AS (VALUES {members}), \
                 actual AS (SELECT x.extname AS extension, i.type || ' ' || i.identity AS member, \
                 pg_describe_object(d.classid, d.objid, 0) AS object FROM pg_depend d \
                 JOIN pg_extension x ON d.refclassid = 'pg_extension'::regclass \
                 AND x.oid = d.refobjid \
                 CROSS JOIN LATERAL pg_identify_object(d.classid, d.objid, 0) AS i \
                 WHERE d.deptype = 'e' AND x.extname IN (SELECT extension FROM expected)) \
                 SELECT a.object, 'a member of extension ' || a.extension \
                 || ' that tests/schema/guard.rs does not expect' FROM actual a \
                 WHERE (a.extension, a.member) NOT IN (SELECT extension, member FROM expected) \
                 UNION ALL \
                 SELECT pg_describe_object('pg_extension'::regclass, x.oid, 0), \
                 'lacks its expected member ' || e.member FROM expected e \
                 JOIN pg_extension x ON x.extname = e.extension \
                 WHERE (e.extension, e.member) NOT IN (SELECT extension, member FROM actual)"
            ),
        ),
        // An extension's comment other than the one CREATE EXTENSION copies
        // from its control file.
        rule(
            "pg_extension",
            "comment",
            "SELECT pg_describe_object('pg_extension'::regclass, x.oid, 0), \
             'comment ' || COALESCE(quote_literal(obj_description(x.oid, 'pg_extension')), \
             'removed') || ', not the one of its control file' \
             FROM pg_extension x JOIN pg_available_extensions a ON a.name = x.extname \
             WHERE obj_description(x.oid, 'pg_extension') IS DISTINCT FROM a.comment"
                .to_owned(),
        ),
        // SECURITY LABEL on any object of the database.
        rule(
            "pg_seclabel",
            "label",
            "SELECT pg_describe_object(s.classoid, s.objoid, s.objsubid), \
             format('security label %s of provider %s', quote_literal(s.label), s.provider) \
             FROM pg_seclabel s"
                .to_owned(),
        ),
    ]
}

/// The initdb objects that a migration is most likely to change: schema
/// `public` and the built-in text search configurations.
fn initdb_object_rules() -> Vec<Rule> {
    vec![
        // Schema public is an initdb object: owned by pg_database_owner
        // since PostgreSQL 15, with initdb's comment.
        rule(
            "pg_namespace",
            "public_owner",
            "SELECT pg_describe_object('pg_namespace'::regclass, n.oid, 0), \
             'owned by ' || n.nspowner::regrole || ', not by pg_database_owner' \
             FROM pg_namespace n WHERE n.nspname = 'public' \
             AND n.nspowner <> 'pg_database_owner'::regrole"
                .to_owned(),
        ),
        rule(
            "pg_namespace",
            "public_comment",
            "SELECT pg_describe_object('pg_namespace'::regclass, n.oid, 0), \
             'comment ' || COALESCE(quote_literal(obj_description(n.oid, 'pg_namespace')), \
             'removed') || ', not initdb''s' FROM pg_namespace n WHERE n.nspname = 'public' \
             AND obj_description(n.oid, 'pg_namespace') IS DISTINCT FROM 'standard public schema'"
                .to_owned(),
        ),
        // A built-in text search configuration that maps a token type to a
        // dictionary created after initdb (ALTER TEXT SEARCH CONFIGURATION
        // ... ALTER MAPPING). A configuration of a migration's own is
        // reported by pg_ts_config.created.
        rule(
            "pg_ts_config_map",
            "mapping",
            format!(
                "SELECT pg_describe_object('pg_ts_config'::regclass, m.mapcfg, 0), \
                 format('maps token type %s to dictionary %s', t.alias, m.mapdict::regdictionary) \
                 FROM pg_ts_config_map m JOIN pg_ts_config c ON c.oid = m.mapcfg \
                 JOIN LATERAL ts_token_type(c.cfgparser) t ON t.tokid = m.maptokentype \
                 WHERE m.mapcfg < {FIRST_NORMAL_OID} AND m.mapdict >= {FIRST_NORMAL_OID}"
            ),
        ),
    ]
}

/// The state of the database itself, from its rows of the shared catalogs.
fn database_rules() -> Vec<Rule> {
    vec![
        // SECURITY LABEL ON DATABASE.
        rule(
            "pg_shseclabel",
            "label",
            format!(
                "SELECT 'the database', \
                 format('security label %s of provider %s', quote_literal(s.label), s.provider) \
                 FROM pg_shseclabel s WHERE s.classoid = 'pg_database'::regclass \
                 AND s.objoid = {THIS_DATABASE}"
            ),
        ),
        // COMMENT ON DATABASE.
        rule(
            "pg_shdescription",
            "comment",
            format!(
                "SELECT 'the database', 'comment ' || quote_literal(s.description) \
                 FROM pg_shdescription s WHERE s.classoid = 'pg_database'::regclass \
                 AND s.objoid = {THIS_DATABASE}"
            ),
        ),
        // ALTER DATABASE ... SET, and ALTER ROLE ... IN DATABASE ... SET.
        rule(
            "pg_db_role_setting",
            "settings",
            format!(
                "SELECT 'the database', 'settings ' || array_to_string(s.setconfig, ', ') \
                 || CASE WHEN s.setrole <> 0 THEN ' for role ' || s.setrole::regrole ELSE '' END \
                 FROM pg_db_role_setting s WHERE s.setdatabase = {THIS_DATABASE}"
            ),
        ),
        // The database's own privileges, connection options and owner, as
        // the test harness creates it: no privileges recorded, no limit, not
        // a template, owned by the migrating role. `datallowconn` is not
        // read: the guard runs connected to the database, and PostgreSQL
        // refuses ALLOW_CONNECTIONS false for the current database.
        rule(
            "pg_database",
            "options",
            format!(
                "SELECT 'the database', concat_ws(', ', \
                 CASE WHEN d.datacl IS NOT NULL THEN 'privileges ' || d.datacl::text END, \
                 CASE WHEN d.datconnlimit <> -1 THEN 'connection limit ' || d.datconnlimit END, \
                 CASE WHEN d.datistemplate THEN 'a template' END, \
                 CASE WHEN d.datdba <> {MIGRATING_ROLE} THEN 'owned by ' || d.datdba::regrole END) \
                 FROM pg_database d WHERE d.datname = current_database() \
                 AND (d.datacl IS NOT NULL OR d.datconnlimit <> -1 OR d.datistemplate \
                 OR d.datdba <> {MIGRATING_ROLE})"
            ),
        ),
        // CREATE SUBSCRIPTION in this database. Only the columns that every
        // role may read are used (`subconninfo` is not).
        rule(
            "pg_subscription",
            "created",
            format!(
                "SELECT pg_describe_object('pg_subscription'::regclass, s.oid, 0), 'not read' \
                 FROM pg_subscription s WHERE s.subdbid = {THIS_DATABASE}"
            ),
        ),
    ]
}

/// Every rule of the guard: those of [`catalogs`], then [`properties`].
pub fn rules() -> Vec<Rule> {
    let mut rules = catalog_rules();
    rules.extend(properties());
    rules
}

/// `value` as an SQL string literal: in single quotes, each one doubled.
///
/// - `value`: any text; the result is safe to splice into a query.
pub fn sql_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// One object or setting that the snapshot does not show.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    /// The [`Rule::id`] that reported it.
    pub rule: String,
    /// The object, as `pg_describe_object` names it (`view probe`), or
    /// `the database` for the database's own state.
    pub object: String,
    /// What the snapshot does not show of it.
    pub problem: String,
}

/// Runs every rule against `db` and returns the findings, sorted.
///
/// - `db`: a connection to the migrated database, or a transaction on it
///   (the tests create objects in one and roll it back).
///
/// A rule whose query fails panics with the rule's id and the error, so a
/// broken rule cannot pass as "nothing found".
pub async fn unread(db: &impl ConnectionTrait) -> Vec<Finding> {
    let mut findings = Vec::new();
    for rule in rules() {
        // The column list names the rule's two columns, so a rule need not
        // alias them; the casts make a literal such as 'the database' text.
        let sql = format!(
            "SELECT f.object::text, f.problem::text FROM ({}) AS f(object, problem)",
            rule.sql
        );
        let rows = db
            .query_all_raw(Statement::from_string(DbBackend::Postgres, &sql))
            .await
            .unwrap_or_else(|error| panic!("guard rule {}: {error}\n{sql}", rule.id));
        for row in rows {
            let get = |column: &str| -> String {
                row.try_get("", column)
                    .unwrap_or_else(|error| panic!("guard rule {}, {column}: {error}", rule.id))
            };
            findings.push(Finding {
                rule: rule.id.clone(),
                object: get("object"),
                problem: get("problem"),
            });
        }
    }
    findings.sort();
    findings
}

/// The failure message for `findings`: what is wrong, one line per finding,
/// and what to do about it.
///
/// - `findings`: what [`unread`] returned; empty gives an empty message.
pub fn report(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "The migrated database holds {} object(s) or setting(s) that the schema snapshot \
         does not show, so the snapshot could not fail on a change to them (ADR 0009). \
         Remove each one from the migration, or extend tests/schema/catalog.rs to show it, \
         narrow the rule of tests/schema/guard.rs that reports it, and update the \
         snapshot, in this PR (recipe 1 of .claude/skills/backend3/SKILL.md):\n",
        findings.len()
    );
    for finding in findings {
        // Writing to a String cannot fail, so the Result carries nothing.
        let _ = writeln!(
            text,
            "  - {}: {} [{}]",
            finding.object, finding.problem, finding.rule
        );
    }
    text
}
