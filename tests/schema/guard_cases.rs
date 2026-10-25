//! The tests of the guard (`guard.rs`): one case per kind of object or
//! setting that the snapshot does not show, each run on the migrated
//! per-test database in a transaction that is rolled back, so each case sees
//! the clean schema plus its own objects only.
//!
//! A case lists exactly the findings it expects, so a rule that reports the
//! wrong object, or reports where it should not, fails as well as one that
//! stays silent. [`every_rule_is_exercised_by_a_case`] makes sure that no rule
//! goes untested, and [`every_catalog_of_pg_catalog_is_accounted_for`] that no
//! catalog is forgotten.
//!
//! Every object a case creates is named `probe` or starts with it, and the
//! cases create their own tables rather than use the migrations', so a later
//! migration does not change them. Another role is `pg_monitor`, a role every
//! PostgreSQL cluster has. Where PostgreSQL offers no statement that a test
//! can run inside a transaction, the case writes the catalog row directly, as
//! the statement would, and says why. That, and several statements below
//! (`CREATE EVENT TRIGGER`, `ALTER TABLE ... DISABLE TRIGGER ALL`), need the
//! test role to be a superuser, as `loco` is in compose and CI.

use std::collections::BTreeSet;

use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};

use super::guard::{self, Finding, OTHER_CATALOGS};
use crate::support::with_app;

/// One kind of object or setting, and what the guard must report for it.
struct Case {
    /// What the case creates.
    name: &'static str,
    /// The statements, run as one batch in a transaction that is rolled back.
    sql: &'static str,
    /// Every finding the guard must report, in any order: the rule, the
    /// object as `pg_describe_object` names it, and a part of the problem
    /// text.
    expected: &'static [(&'static str, &'static str, &'static str)],
}

const CASES: &[Case] = &[
    Case {
        name: "the migrated schema alone",
        sql: "",
        expected: &[],
    },
    Case {
        name: "a schema of its own, owned by another role, with privileges",
        sql: "CREATE SCHEMA probe AUTHORIZATION pg_monitor; \
              GRANT USAGE ON SCHEMA probe TO PUBLIC",
        expected: &[
            ("pg_namespace.created", "schema probe", "not read"),
            ("pg_namespace.owner", "schema probe", "owned by pg_monitor"),
            ("pg_namespace.privileges", "schema probe", "privileges {"),
        ],
    },
    Case {
        name: "privileges on schema public, an initdb object",
        sql: "REVOKE USAGE ON SCHEMA public FROM PUBLIC",
        expected: &[("pg_namespace.privileges", "schema public", "privileges {")],
    },
    Case {
        name: "schema public with another owner and comment",
        // A new owner also becomes the grantor of the schema's privileges.
        sql: "ALTER SCHEMA public OWNER TO pg_monitor; COMMENT ON SCHEMA public IS 'probe'",
        expected: &[
            (
                "pg_namespace.public_owner",
                "schema public",
                "owned by pg_monitor",
            ),
            (
                "pg_namespace.public_comment",
                "schema public",
                "comment 'probe'",
            ),
            ("pg_namespace.privileges", "schema public", "pg_monitor"),
        ],
    },
    Case {
        name: "a view with a column default and an INSTEAD OF trigger",
        sql: "CREATE VIEW probe AS SELECT 1 AS one; \
              ALTER VIEW probe ALTER COLUMN one SET DEFAULT 2; \
              CREATE FUNCTION probe() RETURNS trigger LANGUAGE plpgsql \
                  AS $$ BEGIN RETURN NULL; END $$; \
              CREATE TRIGGER probe INSTEAD OF INSERT ON probe \
                  FOR EACH ROW EXECUTE FUNCTION probe()",
        expected: &[
            ("pg_class.created", "view probe", "not read"),
            (
                "pg_attrdef.created",
                "default value for column one of view probe",
                "not read",
            ),
            (
                "pg_trigger.created",
                "trigger probe on view probe",
                "not read",
            ),
        ],
    },
    Case {
        name: "a materialized view with an index",
        sql: "CREATE MATERIALIZED VIEW probe AS SELECT 1 AS one; \
              CREATE INDEX probe_one ON probe (one)",
        expected: &[
            ("pg_class.created", "materialized view probe", "not read"),
            ("pg_class.created", "index probe_one", "not read"),
        ],
    },
    Case {
        name: "a sequence of its own and a serial column's",
        sql: "CREATE SEQUENCE probe_numbers; CREATE TABLE probe (id serial)",
        expected: &[
            ("pg_class.created", "sequence probe_numbers", "not read"),
            ("pg_class.created", "sequence probe_id_seq", "not read"),
        ],
    },
    Case {
        name: "a foreign table, its server, wrapper and user mapping, with owners and privileges",
        // The owner of a foreign-data wrapper must be a superuser, and the
        // only one a test has is the migrating role itself, so the owner is
        // written into the catalog row.
        sql: "CREATE FOREIGN DATA WRAPPER probe; \
              CREATE SERVER probe FOREIGN DATA WRAPPER probe; \
              CREATE USER MAPPING FOR PUBLIC SERVER probe; \
              CREATE FOREIGN TABLE probe (one int) SERVER probe; \
              GRANT USAGE ON FOREIGN DATA WRAPPER probe TO pg_monitor; \
              GRANT USAGE ON FOREIGN SERVER probe TO pg_monitor; \
              ALTER SERVER probe OWNER TO pg_monitor; \
              UPDATE pg_foreign_data_wrapper SET fdwowner = 'pg_monitor'::regrole \
                  WHERE fdwname = 'probe'",
        expected: &[
            (
                "pg_foreign_data_wrapper.created",
                "foreign-data wrapper probe",
                "not read",
            ),
            (
                "pg_foreign_data_wrapper.owner",
                "foreign-data wrapper probe",
                "owned by pg_monitor",
            ),
            (
                "pg_foreign_data_wrapper.privileges",
                "foreign-data wrapper probe",
                "privileges {",
            ),
            ("pg_foreign_server.created", "server probe", "not read"),
            (
                "pg_foreign_server.owner",
                "server probe",
                "owned by pg_monitor",
            ),
            (
                "pg_foreign_server.privileges",
                "server probe",
                "privileges {",
            ),
            (
                "pg_user_mapping.created",
                "user mapping for public on server probe",
                "not read",
            ),
            ("pg_class.created", "foreign table probe", "not read"),
        ],
    },
    Case {
        name: "types of every kind, and a domain constraint",
        sql: "CREATE TYPE probe_enum AS ENUM ('a'); \
              CREATE DOMAIN probe_domain AS int CONSTRAINT probe_positive CHECK (VALUE > 0); \
              CREATE TYPE probe_range AS RANGE (subtype = int4); \
              CREATE TYPE probe_row AS (one int); \
              CREATE TYPE probe_shell",
        expected: &[
            ("pg_type.created", "type probe_enum", "not read"),
            ("pg_type.created", "type probe_domain", "not read"),
            ("pg_type.created", "type probe_range", "not read"),
            ("pg_type.created", "type probe_row", "not read"),
            ("pg_type.created", "type probe_shell", "not read"),
            // pg_describe_object names a domain constraint without its
            // domain.
            (
                "pg_constraint.created",
                "constraint probe_positive",
                "not read",
            ),
        ],
    },
    Case {
        name: "a type owned by another role, with privileges",
        sql: "CREATE TYPE probe AS ENUM ('a'); ALTER TYPE probe OWNER TO pg_monitor; \
              REVOKE USAGE ON TYPE probe FROM PUBLIC",
        expected: &[
            ("pg_type.created", "type probe", "not read"),
            ("pg_type.owner", "type probe", "owned by pg_monitor"),
            ("pg_type.privileges", "type probe", "privileges {"),
        ],
    },
    Case {
        name: "a typed table",
        sql: "CREATE TYPE probe_row AS (one int); CREATE TABLE probe OF probe_row",
        expected: &[
            ("pg_type.created", "type probe_row", "not read"),
            ("pg_class.typed_table", "table probe", "OF probe_row"),
        ],
    },
    Case {
        name: "a function outside schema public, and an aggregate",
        sql: "CREATE FUNCTION pg_catalog.probe() RETURNS int LANGUAGE sql AS 'SELECT 1'; \
              CREATE AGGREGATE probe_sum (int) (SFUNC = int4pl, STYPE = int)",
        expected: &[
            ("pg_proc.created", "function probe()", "not read"),
            ("pg_proc.created", "function probe_sum(integer)", "not read"),
        ],
    },
    Case {
        name: "an extension member and a range type's constructor owned by another role",
        // The constructor is an internal part of its type, yet ALTER FUNCTION
        // gives it an owner of its own (`Catalog::parts_have_owners`). The
        // range type itself is reported as not read.
        sql: "ALTER FUNCTION unaccent(text) OWNER TO pg_monitor; \
              CREATE TYPE probe_range AS RANGE (subtype = int4); \
              ALTER FUNCTION probe_range(int4, int4) OWNER TO pg_monitor",
        expected: &[
            (
                "pg_proc.owner",
                "function unaccent(text)",
                "owned by pg_monitor",
            ),
            ("pg_type.created", "type probe_range", "not read"),
            (
                "pg_proc.owner",
                "function probe_range(integer,integer)",
                "owned by pg_monitor",
            ),
        ],
    },
    Case {
        name: "a function owned by another role, and privileges on it and on a built-in one",
        sql: "CREATE FUNCTION probe() RETURNS int LANGUAGE sql AS 'SELECT 1'; \
              ALTER FUNCTION probe() OWNER TO pg_monitor; \
              REVOKE EXECUTE ON FUNCTION probe() FROM PUBLIC; \
              REVOKE EXECUTE ON FUNCTION pg_catalog.now() FROM PUBLIC",
        expected: &[
            ("pg_proc.owner", "function probe()", "owned by pg_monitor"),
            ("pg_proc.privileges", "function probe()", "privileges {"),
            ("pg_proc.privileges", "function now()", "privileges {"),
        ],
    },
    Case {
        name: "comments on objects other than tables and their columns, \
               a system column and an extension member among them",
        // The table and column comments are in the snapshot, so only the
        // other five are reported.
        sql: "CREATE TABLE probe (one int CONSTRAINT probe_one CHECK (one > 0)); \
              CREATE INDEX probe_index ON probe (one); \
              CREATE FUNCTION probe() RETURNS int LANGUAGE sql AS 'SELECT 1'; \
              COMMENT ON TABLE probe IS 'shown'; COMMENT ON COLUMN probe.one IS 'shown'; \
              COMMENT ON CONSTRAINT probe_one ON probe IS 'hidden'; \
              COMMENT ON INDEX probe_index IS 'hidden'; \
              COMMENT ON FUNCTION probe() IS 'hidden'; \
              COMMENT ON COLUMN probe.ctid IS 'hidden'; \
              COMMENT ON FUNCTION unaccent(text) IS 'hidden'",
        expected: &[
            (
                "pg_description.comment",
                "column ctid of table probe",
                "comment 'hidden'",
            ),
            (
                "pg_description.comment",
                "function unaccent(text)",
                "comment 'hidden'",
            ),
            (
                "pg_description.comment",
                "constraint probe_one on table probe",
                "comment 'hidden'",
            ),
            (
                "pg_description.comment",
                "index probe_index",
                "comment 'hidden'",
            ),
            (
                "pg_description.comment",
                "function probe()",
                "comment 'hidden'",
            ),
        ],
    },
    Case {
        name: "a collation outside schema public, and one owned by another role",
        sql: "CREATE SCHEMA probe; \
              CREATE COLLATION probe.probe (provider = icu, locale = 'de'); \
              CREATE COLLATION probe_de (provider = icu, locale = 'de'); \
              ALTER COLLATION probe_de OWNER TO pg_monitor",
        expected: &[
            ("pg_namespace.created", "schema probe", "not read"),
            ("pg_collation.created", "collation probe.probe", "not read"),
            (
                "pg_collation.owner",
                "collation probe_de",
                "owned by pg_monitor",
            ),
        ],
    },
    Case {
        name: "a rule",
        sql: "CREATE TABLE probe (one int); \
              CREATE RULE probe AS ON INSERT TO probe DO INSTEAD NOTHING",
        expected: &[(
            "pg_rewrite.created",
            "rule probe on table probe",
            "not read",
        )],
    },
    Case {
        name: "row-level security with a policy",
        sql: "CREATE TABLE probe (one int); ALTER TABLE probe ENABLE ROW LEVEL SECURITY; \
              CREATE POLICY probe ON probe USING (true)",
        expected: &[
            (
                "pg_class.row_security",
                "table probe",
                "row-level security enabled",
            ),
            (
                "pg_policy.created",
                "policy probe on table probe",
                "not read",
            ),
        ],
    },
    Case {
        name: "forced row-level security alone",
        sql: "CREATE TABLE probe (one int); ALTER TABLE probe FORCE ROW LEVEL SECURITY",
        expected: &[(
            "pg_class.row_security",
            "table probe",
            "row-level security forced",
        )],
    },
    Case {
        name: "privileges on a table, a column, an identity sequence and a built-in catalog",
        sql: "CREATE TABLE probe (id bigint GENERATED ALWAYS AS IDENTITY, one int); \
              GRANT SELECT ON probe TO PUBLIC; GRANT UPDATE (one) ON probe TO PUBLIC; \
              GRANT USAGE ON SEQUENCE probe_id_seq TO PUBLIC; \
              REVOKE SELECT ON pg_catalog.pg_class FROM PUBLIC",
        expected: &[
            ("pg_class.privileges", "table probe", "privileges {"),
            (
                "pg_class.privileges",
                "sequence probe_id_seq",
                "privileges {",
            ),
            ("pg_class.privileges", "table pg_class", "privileges {"),
            (
                "pg_attribute.privileges",
                "column one of table probe",
                "privileges {",
            ),
        ],
    },
    Case {
        name: "a table owned by another role",
        // The index follows its table to the new owner; the primary key's
        // index and the identity sequence are internal parts of the table.
        sql: "CREATE TABLE probe (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, one int); \
              CREATE INDEX probe_one ON probe (one); ALTER TABLE probe OWNER TO pg_monitor",
        expected: &[
            ("pg_class.owner", "table probe", "owned by pg_monitor"),
            ("pg_class.owner", "index probe_one", "owned by pg_monitor"),
        ],
    },
    Case {
        name: "storage parameters of a table and of its TOAST table",
        sql: "CREATE TABLE probe (one text) \
              WITH (fillfactor = 70, toast.autovacuum_enabled = false)",
        expected: &[
            (
                "pg_class.storage_parameters",
                "table probe",
                "fillfactor=70",
            ),
            (
                "pg_class.toast_storage_parameters",
                "table probe",
                "autovacuum_enabled=false",
            ),
        ],
    },
    Case {
        name: "column settings",
        sql: "CREATE TABLE probe (a text, b text, c text, d text); \
              ALTER TABLE probe ALTER COLUMN a SET (n_distinct = 5); \
              ALTER TABLE probe ALTER COLUMN b SET STATISTICS 500; \
              ALTER TABLE probe ALTER COLUMN c SET STORAGE EXTERNAL; \
              ALTER TABLE probe ALTER COLUMN d SET COMPRESSION pglz; \
              CREATE INDEX probe_lower ON probe (lower(a)); \
              ALTER INDEX probe_lower ALTER COLUMN 1 SET STATISTICS 100",
        expected: &[
            (
                "pg_attribute.options",
                "column a of table probe",
                "n_distinct=5",
            ),
            (
                "pg_attribute.statistics_target",
                "column b of table probe",
                "500",
            ),
            (
                "pg_attribute.storage",
                "column c of table probe",
                "EXTERNAL",
            ),
            (
                "pg_attribute.compression",
                "column d of table probe",
                "pglz",
            ),
            (
                "pg_attribute.statistics_target",
                "column lower of index probe_lower",
                "100",
            ),
        ],
    },
    Case {
        name: "a replica identity",
        sql: "CREATE TABLE probe (one int); ALTER TABLE probe REPLICA IDENTITY FULL",
        expected: &[("pg_class.replica_identity", "table probe", "FULL")],
    },
    Case {
        name: "a table access method",
        sql: "CREATE ACCESS METHOD probe TYPE TABLE HANDLER heap_tableam_handler; \
              CREATE TABLE probe (one int) USING probe",
        expected: &[
            ("pg_am.created", "access method probe", "not read"),
            (
                "pg_class.access_method",
                "table probe",
                "table access method probe",
            ),
        ],
    },
    Case {
        name: "a partitioned table with an access method for its partitions",
        // A partitioned table has none (0) unless one is set, even heap.
        sql: "CREATE TABLE probe (id int) PARTITION BY LIST (id) USING heap",
        expected: &[(
            "pg_class.access_method",
            "table probe",
            "table access method heap",
        )],
    },
    Case {
        name: "a table in another tablespace",
        // CREATE TABLESPACE needs a directory on the server and refuses to
        // run in a transaction, so the table is placed in pg_global (the
        // tablespace every cluster has) in its catalog row.
        sql: "CREATE TABLE probe (one int); \
              UPDATE pg_class SET reltablespace = \
                  (SELECT oid FROM pg_tablespace WHERE spcname = 'pg_global') \
                  WHERE oid = 'probe'::regclass",
        expected: &[("pg_class.tablespace", "table probe", "tablespace pg_global")],
    },
    Case {
        name: "an index that is not valid",
        // An index ON ONLY a partitioned table stays invalid until every
        // partition has one attached.
        sql: "CREATE TABLE probe (id int) PARTITION BY LIST (id); \
              CREATE TABLE probe_one PARTITION OF probe FOR VALUES IN (1); \
              CREATE INDEX probe_id ON ONLY probe (id)",
        expected: &[("pg_index.state", "index probe_id", "not valid")],
    },
    Case {
        name: "a clustered index",
        sql: "CREATE TABLE probe (id int PRIMARY KEY); ALTER TABLE probe CLUSTER ON probe_pkey",
        expected: &[("pg_index.clustered", "index probe_pkey", "CLUSTER ON")],
    },
    Case {
        name: "identity sequences with options, a name, a data type or a persistence of their own",
        // `ALTER COLUMN ... TYPE` would change the sequence's type with the
        // column's; `ALTER SEQUENCE ... AS` changes the sequence's alone.
        sql: "CREATE TABLE probe (id bigint GENERATED ALWAYS AS IDENTITY \
                  (START WITH 10 INCREMENT BY 2)); \
              CREATE TABLE probe_named (id int GENERATED BY DEFAULT AS IDENTITY \
                  (SEQUENCE NAME probe_numbers)); \
              CREATE TABLE probe_typed (id bigint GENERATED BY DEFAULT AS IDENTITY); \
              ALTER SEQUENCE probe_typed_id_seq AS integer; \
              CREATE TABLE probe_unlogged (id int GENERATED ALWAYS AS IDENTITY); \
              ALTER SEQUENCE probe_unlogged_id_seq SET UNLOGGED",
        expected: &[
            (
                "pg_sequence.identity",
                "column id of table probe",
                "AS bigint, START 10, INCREMENT 2",
            ),
            (
                "pg_sequence.identity",
                "column id of table probe_named",
                "probe_numbers",
            ),
            (
                "pg_sequence.identity",
                "column id of table probe_typed",
                "AS integer, START 1,",
            ),
            (
                "pg_sequence.identity",
                "column id of table probe_unlogged",
                "CACHE 1, UNLOGGED",
            ),
        ],
    },
    Case {
        name: "internal triggers of a foreign key and a deferrable unique constraint \
               that are disabled, enabled otherwise or renamed",
        // The triggers' names carry their oids, so a DO block finds them by
        // their functions. A self-referencing foreign key has all four of
        // its triggers on its own table.
        sql: "CREATE TABLE probe (id int PRIMARY KEY, parent int REFERENCES probe, \
                  code int UNIQUE DEFERRABLE); \
              ALTER TABLE probe DISABLE TRIGGER ALL; \
              CREATE TABLE probe_other (id int PRIMARY KEY, parent int REFERENCES probe_other); \
              DO $$ DECLARE \
                  name_of CONSTANT text := 'SELECT tgname FROM pg_trigger \
                      WHERE tgrelid = ''probe_other''::regclass AND tgfoid = $1::regproc'; \
                  found text; \
              BEGIN \
                  EXECUTE name_of INTO found USING '\"RI_FKey_check_ins\"'; \
                  EXECUTE format('ALTER TABLE probe_other ENABLE REPLICA TRIGGER %I', found); \
                  EXECUTE name_of INTO found USING '\"RI_FKey_check_upd\"'; \
                  EXECUTE format('ALTER TABLE probe_other ENABLE ALWAYS TRIGGER %I', found); \
                  EXECUTE name_of INTO found USING '\"RI_FKey_noaction_del\"'; \
                  EXECUTE format('ALTER TRIGGER %I ON probe_other RENAME TO probe_renamed', \
                      found); \
              END $$",
        expected: &[
            (
                "pg_trigger.internal",
                "constraint probe_parent_fkey on table probe",
                "\"RI_FKey_check_ins\" on probe: disabled",
            ),
            (
                "pg_trigger.internal",
                "constraint probe_parent_fkey on table probe",
                "\"RI_FKey_check_upd\" on probe: disabled",
            ),
            (
                "pg_trigger.internal",
                "constraint probe_parent_fkey on table probe",
                "\"RI_FKey_noaction_del\" on probe: disabled",
            ),
            (
                "pg_trigger.internal",
                "constraint probe_parent_fkey on table probe",
                "\"RI_FKey_noaction_upd\" on probe: disabled",
            ),
            (
                "pg_trigger.internal",
                "constraint probe_code_key on table probe",
                "unique_key_recheck on probe: disabled",
            ),
            (
                "pg_trigger.internal",
                "constraint probe_other_parent_fkey on table probe_other",
                "\"RI_FKey_check_ins\" on probe_other: enabled for replicas only",
            ),
            (
                "pg_trigger.internal",
                "constraint probe_other_parent_fkey on table probe_other",
                "\"RI_FKey_check_upd\" on probe_other: enabled always",
            ),
            (
                "pg_trigger.internal",
                "constraint probe_other_parent_fkey on table probe_other",
                "\"RI_FKey_noaction_del\" on probe_other: renamed to probe_renamed",
            ),
        ],
    },
    Case {
        name: "an extended statistics object owned by another role",
        sql: "CREATE TABLE probe (a int, b int); CREATE STATISTICS probe ON a, b FROM probe; \
              ALTER STATISTICS probe OWNER TO pg_monitor",
        expected: &[
            (
                "pg_statistic_ext.created",
                "statistics object probe",
                "not read",
            ),
            (
                "pg_statistic_ext.owner",
                "statistics object probe",
                "owned by pg_monitor",
            ),
        ],
    },
    Case {
        name: "an operator, also added to a built-in operator family with a support function",
        sql: "CREATE FUNCTION probe_lt(text, bytea) RETURNS bool LANGUAGE sql AS 'SELECT true'; \
              CREATE FUNCTION probe_cmp(text, bytea) RETURNS int LANGUAGE sql AS 'SELECT 0'; \
              CREATE OPERATOR <<< (FUNCTION = probe_lt, LEFTARG = text, RIGHTARG = bytea); \
              ALTER OPERATOR <<< (text, bytea) OWNER TO pg_monitor; \
              ALTER OPERATOR FAMILY text_ops USING btree ADD OPERATOR 1 <<< (text, bytea), \
                  FUNCTION 1 (text, bytea) probe_cmp(text, bytea)",
        expected: &[
            (
                "pg_operator.created",
                "operator <<<(text,bytea)",
                "not read",
            ),
            (
                "pg_operator.owner",
                "operator <<<(text,bytea)",
                "owned by pg_monitor",
            ),
            (
                "pg_amop.created",
                "operator 1 (text, bytea) of operator family text_ops for access method btree: \
                 <<<(text,bytea)",
                "not read",
            ),
            (
                "pg_amproc.created",
                "function 1 (text, bytea) of operator family text_ops for access method btree: \
                 probe_cmp(text,bytea)",
                "not read",
            ),
        ],
    },
    Case {
        name: "an operator class and its family, owned by another role",
        // The class's operator and function are internal parts of it.
        sql: "CREATE OPERATOR CLASS probe FOR TYPE text USING btree \
                  AS OPERATOR 1 <, FUNCTION 1 bttextcmp(text, text); \
              ALTER OPERATOR CLASS probe USING btree OWNER TO pg_monitor; \
              ALTER OPERATOR FAMILY probe USING btree OWNER TO pg_monitor",
        expected: &[
            (
                "pg_opclass.created",
                "operator class probe for access method btree",
                "not read",
            ),
            (
                "pg_opclass.owner",
                "operator class probe for access method btree",
                "owned by pg_monitor",
            ),
            (
                "pg_opfamily.created",
                "operator family probe for access method btree",
                "not read",
            ),
            (
                "pg_opfamily.owner",
                "operator family probe for access method btree",
                "owned by pg_monitor",
            ),
        ],
    },
    Case {
        name: "a cast",
        sql: "CREATE CAST (bytea AS money) WITH INOUT",
        expected: &[("pg_cast.created", "cast from bytea to money", "not read")],
    },
    Case {
        name: "a conversion owned by another role",
        sql: "CREATE CONVERSION probe FOR 'LATIN1' TO 'UTF8' FROM iso8859_1_to_utf8; \
              ALTER CONVERSION probe OWNER TO pg_monitor",
        expected: &[
            ("pg_conversion.created", "conversion probe", "not read"),
            (
                "pg_conversion.owner",
                "conversion probe",
                "owned by pg_monitor",
            ),
        ],
    },
    Case {
        name: "a language owned by another role, and privileges on it and on plpgsql",
        sql: "CREATE TRUSTED LANGUAGE probe HANDLER plpgsql_call_handler; \
              ALTER LANGUAGE probe OWNER TO pg_monitor; \
              GRANT USAGE ON LANGUAGE probe TO pg_monitor; \
              REVOKE USAGE ON LANGUAGE plpgsql FROM PUBLIC",
        expected: &[
            ("pg_language.created", "language probe", "not read"),
            ("pg_language.owner", "language probe", "owned by pg_monitor"),
            ("pg_language.privileges", "language probe", "privileges {"),
            ("pg_language.privileges", "language plpgsql", "privileges {"),
        ],
    },
    Case {
        name: "a transform",
        // Any function from internal to internal passes CREATE TRANSFORM's
        // checks; the transform is never used.
        sql: "CREATE TRANSFORM FOR int4 LANGUAGE plpgsql \
                  (FROM SQL WITH FUNCTION brin_minmax_opcinfo(internal))",
        expected: &[(
            "pg_transform.created",
            "transform for integer language plpgsql",
            "not read",
        )],
    },
    Case {
        name: "text search objects, owned by another role where they have an owner",
        sql: "CREATE TEXT SEARCH PARSER probe (START = prsd_start, GETTOKEN = prsd_nexttoken, \
                  END = prsd_end, LEXTYPES = prsd_lextype); \
              CREATE TEXT SEARCH TEMPLATE probe (INIT = dsimple_init, LEXIZE = dsimple_lexize); \
              CREATE TEXT SEARCH DICTIONARY probe (TEMPLATE = simple); \
              CREATE TEXT SEARCH CONFIGURATION probe (COPY = simple); \
              ALTER TEXT SEARCH DICTIONARY probe OWNER TO pg_monitor; \
              ALTER TEXT SEARCH CONFIGURATION probe OWNER TO pg_monitor",
        expected: &[
            (
                "pg_ts_parser.created",
                "text search parser probe",
                "not read",
            ),
            (
                "pg_ts_template.created",
                "text search template probe",
                "not read",
            ),
            (
                "pg_ts_dict.created",
                "text search dictionary probe",
                "not read",
            ),
            (
                "pg_ts_dict.owner",
                "text search dictionary probe",
                "owned by pg_monitor",
            ),
            (
                "pg_ts_config.created",
                "text search configuration probe",
                "not read",
            ),
            (
                "pg_ts_config.owner",
                "text search configuration probe",
                "owned by pg_monitor",
            ),
        ],
    },
    Case {
        name: "a built-in text search configuration mapped to a new dictionary",
        sql: "CREATE TEXT SEARCH DICTIONARY probe (TEMPLATE = simple); \
              ALTER TEXT SEARCH CONFIGURATION simple ALTER MAPPING FOR asciiword WITH probe",
        expected: &[
            (
                "pg_ts_dict.created",
                "text search dictionary probe",
                "not read",
            ),
            (
                "pg_ts_config_map.mapping",
                "text search configuration simple",
                "maps token type asciiword to dictionary probe",
            ),
        ],
    },
    Case {
        name: "an extension that is not expected",
        sql: "CREATE EXTENSION tsm_system_rows",
        expected: &[(
            "pg_extension.expected",
            "extension tsm_system_rows",
            "not one of the expected extensions",
        )],
    },
    Case {
        name: "an object added to an expected extension, and one dropped from it",
        // The dropped function stays in schema public, where the reader
        // shows it like any other function.
        sql: "CREATE FUNCTION probe() RETURNS int LANGUAGE sql AS 'SELECT 1'; \
              ALTER EXTENSION unaccent ADD FUNCTION probe(); \
              ALTER EXTENSION unaccent DROP FUNCTION unaccent(text)",
        expected: &[
            (
                "pg_extension.members",
                "function probe()",
                "does not expect",
            ),
            (
                "pg_extension.members",
                "extension unaccent",
                "lacks its expected member function public.unaccent(pg_catalog.text)",
            ),
        ],
    },
    Case {
        name: "an expected extension that is missing",
        // plpgsql, because the formatter leaves it out of the snapshot.
        // CASCADE also drops the migrations' plpgsql functions, if any, and
        // their triggers; dropping what the reader shows is no finding.
        sql: "DROP EXTENSION plpgsql CASCADE",
        expected: &[(
            "pg_extension.missing",
            "extension plpgsql",
            "expected by tests/schema/guard.rs, but not installed",
        )],
    },
    Case {
        name: "an extension with another comment",
        sql: "COMMENT ON EXTENSION unaccent IS 'probe'",
        expected: &[(
            "pg_extension.comment",
            "extension unaccent",
            "comment 'probe'",
        )],
    },
    Case {
        name: "a function that depends on an extension",
        sql: "CREATE FUNCTION probe() RETURNS int LANGUAGE sql AS 'SELECT 1'; \
              ALTER FUNCTION probe() DEPENDS ON EXTENSION unaccent",
        expected: &[(
            "pg_depend.extension_dependency",
            "function probe()",
            "depends on extension unaccent",
        )],
    },
    Case {
        name: "an event trigger owned by another role",
        // An event trigger's owner must be a superuser too, so the owner is
        // written into the catalog row, as for the foreign-data wrapper.
        sql: "CREATE FUNCTION probe() RETURNS event_trigger LANGUAGE plpgsql \
                  AS $$ BEGIN END $$; \
              CREATE EVENT TRIGGER probe ON ddl_command_start EXECUTE FUNCTION probe(); \
              UPDATE pg_event_trigger SET evtowner = 'pg_monitor'::regrole \
                  WHERE evtname = 'probe'",
        expected: &[
            (
                "pg_event_trigger.created",
                "event trigger probe",
                "not read",
            ),
            (
                "pg_event_trigger.owner",
                "event trigger probe",
                "owned by pg_monitor",
            ),
        ],
    },
    Case {
        name: "publications of a table and of a schema, one owned by another role",
        sql: "CREATE TABLE probe (one int); CREATE PUBLICATION probe FOR TABLE probe; \
              CREATE PUBLICATION probe_schema FOR TABLES IN SCHEMA public; \
              ALTER PUBLICATION probe OWNER TO pg_monitor",
        expected: &[
            ("pg_publication.created", "publication probe", "not read"),
            (
                "pg_publication.owner",
                "publication probe",
                "owned by pg_monitor",
            ),
            (
                "pg_publication.created",
                "publication probe_schema",
                "not read",
            ),
            (
                "pg_publication_rel.created",
                "publication of table probe in publication probe",
                "not read",
            ),
            (
                "pg_publication_namespace.created",
                "publication of schema public in publication probe_schema",
                "not read",
            ),
        ],
    },
    Case {
        name: "a subscription",
        // `connect = false` creates no replication slot, so it may run in a
        // transaction and needs no publisher.
        sql: "CREATE SUBSCRIPTION probe CONNECTION 'dbname=probe' PUBLICATION probe \
                  WITH (connect = false)",
        expected: &[("pg_subscription.created", "subscription probe", "not read")],
    },
    Case {
        name: "default privileges",
        sql: "ALTER DEFAULT PRIVILEGES FOR ROLE pg_monitor GRANT SELECT ON TABLES TO PUBLIC",
        expected: &[(
            "pg_default_acl.created",
            "default privileges on new relations belonging to role pg_monitor",
            "not read",
        )],
    },
    Case {
        name: "a large object with an oid below 16384, owned by another role, with privileges",
        sql: "SELECT lo_create(4242); ALTER LARGE OBJECT 4242 OWNER TO pg_monitor; \
              GRANT SELECT ON LARGE OBJECT 4242 TO PUBLIC",
        expected: &[
            (
                "pg_largeobject_metadata.created",
                "large object 4242",
                "not read",
            ),
            (
                "pg_largeobject_metadata.owner",
                "large object 4242",
                "owned by pg_monitor",
            ),
            (
                "pg_largeobject_metadata.privileges",
                "large object 4242",
                "privileges {",
            ),
        ],
    },
    Case {
        name: "security labels on a table and on the database",
        // SECURITY LABEL needs a label provider (a server module such as
        // sepgsql) that the test server does not load, so the labels are
        // written into the catalogs.
        sql: "CREATE TABLE probe (one int); \
              INSERT INTO pg_seclabel \
                  VALUES ('probe'::regclass, 'pg_class'::regclass, 0, 'probe', 'probe'); \
              INSERT INTO pg_shseclabel SELECT oid, 'pg_database'::regclass, 'probe', 'probe' \
                  FROM pg_database WHERE datname = current_database()",
        expected: &[
            ("pg_seclabel.label", "table probe", "provider probe"),
            ("pg_shseclabel.label", "the database", "provider probe"),
        ],
    },
    Case {
        name: "settings, a comment and a connection limit on the database",
        sql: "DO $$ BEGIN \
              EXECUTE format('ALTER DATABASE %I SET work_mem = %L', current_database(), '8MB'); \
              EXECUTE format('COMMENT ON DATABASE %I IS %L', current_database(), 'probe'); \
              EXECUTE format('ALTER DATABASE %I CONNECTION LIMIT 5', current_database()); \
              END $$",
        expected: &[
            (
                "pg_db_role_setting.settings",
                "the database",
                "work_mem=8MB",
            ),
            (
                "pg_shdescription.comment",
                "the database",
                "comment 'probe'",
            ),
            ("pg_database.options", "the database", "connection limit 5"),
        ],
    },
    Case {
        name: "privileges on the database",
        sql: "DO $$ BEGIN \
              EXECUTE format('REVOKE CONNECT ON DATABASE %I FROM PUBLIC', current_database()); \
              END $$",
        expected: &[("pg_database.options", "the database", "privileges {")],
    },
    Case {
        name: "the database as a template",
        sql: "DO $$ BEGIN \
              EXECUTE format('ALTER DATABASE %I IS_TEMPLATE true', current_database()); \
              END $$",
        expected: &[("pg_database.options", "the database", "a template")],
    },
    Case {
        name: "the database owned by another role",
        sql: "DO $$ BEGIN \
              EXECUTE format('ALTER DATABASE %I OWNER TO pg_monitor', current_database()); \
              END $$",
        expected: &[("pg_database.options", "the database", "owned by pg_monitor")],
    },
    Case {
        name: "a temporary table, in a temporary schema that is not reported, whoever owns it",
        // PostgreSQL makes the bootstrap superuser the owner of every
        // temporary schema, which is the test role in compose and CI. The
        // catalog write gives this session's schema another owner and
        // privileges, as a test role that is not the bootstrap superuser,
        // or default privileges for schemas, would leave it.
        sql: "CREATE TEMP TABLE probe (one int); \
              UPDATE pg_namespace SET nspowner = 'pg_monitor'::regrole, \
                  nspacl = '{pg_monitor=UC/pg_monitor}' WHERE oid = pg_my_temp_schema()",
        expected: &[("pg_class.created", "table probe", "not read")],
    },
];

/// Runs `case` on `db` in a transaction that is rolled back, and returns
/// what is wrong with the guard's findings: an empty list when they are
/// exactly the expected ones.
async fn check(db: &sea_orm::DatabaseConnection, case: &Case) -> Vec<String> {
    let transaction = db.begin().await.expect("begin the case's transaction");
    if !case.sql.is_empty() {
        transaction
            .execute_unprepared(case.sql)
            .await
            .unwrap_or_else(|error| panic!("case {:?}: {error}", case.name));
    }
    let found = guard::unread(&transaction).await;
    transaction.rollback().await.expect("roll the case back");

    let mut problems = Vec::new();
    let mut unmatched: Vec<&Finding> = found.iter().collect();
    for &(rule, object, part) in case.expected {
        let position = unmatched.iter().position(|finding| {
            finding.rule == rule && finding.object == object && finding.problem.contains(part)
        });
        match position {
            Some(index) => {
                unmatched.remove(index);
            }
            None => problems.push(format!("missing: {object}: …{part}… [{rule}]")),
        }
    }
    for finding in unmatched {
        problems.push(format!(
            "unexpected: {}: {} [{}]",
            finding.object, finding.problem, finding.rule
        ));
    }
    problems
}

#[tokio::test]
async fn the_guard_reports_exactly_what_each_case_creates() {
    with_app(|_router, ctx| async move {
        let mut failures = Vec::new();
        for case in CASES {
            let problems = check(&ctx.db, case).await;
            if !problems.is_empty() {
                failures.push(format!("{}:\n    {}", case.name, problems.join("\n    ")));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    })
    .await;
}

#[test]
fn every_rule_is_exercised_by_a_case() {
    let exercised: BTreeSet<&str> = CASES
        .iter()
        .flat_map(|case| case.expected.iter().map(|&(rule, _, _)| rule))
        .collect();
    let rules: BTreeSet<String> = guard::rules().into_iter().map(|rule| rule.id).collect();
    let untested: Vec<&String> = rules
        .iter()
        .filter(|id| !exercised.contains(id.as_str()))
        .collect();
    let unknown: Vec<&&str> = exercised
        .iter()
        .filter(|id| !rules.contains(**id))
        .collect();
    assert!(
        untested.is_empty() && unknown.is_empty(),
        "rules without a case: {untested:?}; cases naming no rule: {unknown:?}"
    );
}

#[test]
fn rule_ids_are_unique() {
    let rules = guard::rules();
    let ids: BTreeSet<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
    assert_eq!(ids.len(), rules.len(), "a rule id is used twice");
}

#[tokio::test]
async fn every_catalog_of_pg_catalog_is_accounted_for() {
    with_app(|_router, ctx| async move {
        let sql = "SELECT relname::text AS name FROM pg_class \
                   WHERE relnamespace = 'pg_catalog'::regnamespace AND relkind = 'r'";
        let present: BTreeSet<String> = ctx
            .db
            .query_all_raw(Statement::from_string(DbBackend::Postgres, sql))
            .await
            .expect("list the catalogs")
            .iter()
            .map(|row| row.try_get("", "name").expect("a catalog name"))
            .collect();
        let ruled: BTreeSet<String> = guard::rules()
            .iter()
            .map(|rule| rule.catalog.to_owned())
            .collect();
        let other: BTreeSet<String> = OTHER_CATALOGS
            .iter()
            .map(|&(name, _)| name.to_owned())
            .collect();
        let accounted: BTreeSet<String> = ruled.union(&other).cloned().collect();
        assert_eq!(
            ruled.intersection(&other).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "a catalog with rules is also listed in OTHER_CATALOGS"
        );
        assert_eq!(
            present.difference(&accounted).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "catalogs of pg_catalog that tests/schema/guard.rs neither reads nor explains: \
             give each one rules or an OTHER_CATALOGS entry"
        );
        assert_eq!(
            accounted.difference(&present).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "catalogs that tests/schema/guard.rs names but PostgreSQL does not have"
        );
    })
    .await;
}

/// The owner rules name the migrating role by [`guard::MIGRATING_ROLE`]. A
/// role whose name is not a plain lower-case identifier must resolve to
/// itself: casting the name to `regrole` would fold `Probe-Role` or fail on
/// it. The role exists only inside the transaction, which is rolled back.
#[tokio::test]
async fn the_migrating_role_is_found_by_its_exact_name() {
    with_app(|_router, ctx| async move {
        let transaction = ctx.db.begin().await.expect("begin the transaction");
        transaction
            .execute_unprepared("CREATE ROLE \"Probe-Role\"; SET LOCAL ROLE \"Probe-Role\"")
            .await
            .expect("create the role and switch to it");
        let sql = format!(
            "SELECT ({}) = (SELECT oid FROM pg_roles WHERE rolname = 'Probe-Role') AS same",
            guard::MIGRATING_ROLE
        );
        let row = transaction
            .query_one_raw(Statement::from_string(DbBackend::Postgres, &sql))
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
            .expect("one row");
        let same: bool = row.try_get("", "same").expect("the comparison");
        transaction.rollback().await.expect("roll back");
        assert!(same, "{sql} does not find Probe-Role");
    })
    .await;
}

#[test]
fn sql_text_quotes_and_doubles_single_quotes() {
    assert_eq!(guard::sql_text(""), "''");
    assert_eq!(guard::sql_text("plpgsql"), "'plpgsql'");
    assert_eq!(guard::sql_text("initdb's"), "'initdb''s'");
    assert_eq!(guard::sql_text("''"), "''''''");
}

#[test]
fn the_report_names_every_finding_and_what_to_do() {
    assert_eq!(guard::report(&[]), "");
    let findings = [
        Finding {
            rule: "pg_class.created".to_owned(),
            object: "view probe".to_owned(),
            problem: "not read".to_owned(),
        },
        Finding {
            rule: "pg_class.row_security".to_owned(),
            object: "table probe".to_owned(),
            problem: "row-level security enabled".to_owned(),
        },
    ];
    let report = guard::report(&findings);
    assert!(
        report.starts_with("The migrated database holds 2 object(s)"),
        "{report}"
    );
    assert!(report.contains("tests/schema/catalog.rs"), "{report}");
    assert!(report.ends_with(
        "  - view probe: not read [pg_class.created]\n  \
         - table probe: row-level security enabled [pg_class.row_security]\n"
    ));
}
