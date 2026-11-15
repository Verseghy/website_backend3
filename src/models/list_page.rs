//! One page of an admin list and its total, read in one snapshot: the
//! database half of the shared admin list contract (parity SHR-02, SHR-03;
//! ADR 0020 §4). A model module without an entity (ADR 0008 rule 5): every
//! entity's list finder shares it.
//!
//! The pure half, `crate::domain::list_query`, has checked the request
//! against the resource's allow-list and says what to read: the filters,
//! the `ORDER BY` terms and the window. [`fetch`] applies them to the
//! entity's select, which the caller has already narrowed to the rows the
//! list shows (for every list so far, the live rows: the entity's live-row
//! helper, ADR 0011 §3).
//!
//! Three choices that a reader might not expect:
//!
//! - **One snapshot.** The total and the page are two statements, and a
//!   save between them could make the total disagree with the rows (a page
//!   that shows a row the total does not count). They run in one read-only
//!   `REPEATABLE READ` transaction, so both see the same committed state.
//!   That is why [`fetch`] takes the pool rather than any connection: it
//!   opens the transaction itself, and a transaction cannot be opened with
//!   an isolation level inside another one.
//! - **Collation in the query, by name.** A text sort is
//!   `<column> COLLATE hu_icu` (ADR 0020 §4). sea-query has no expression
//!   for a collation, so [`order_expr`] writes it with `Expr::cust_with_expr`
//!   around the typed column. The collation name is a constant of the
//!   domain module (`Collation::sql_name`) and the column comes from the
//!   spec, never from the request.
//! - **Columns by name.** The spec names its columns as text, so they are
//!   resolved through the entity's `Column::from_str`. A name the entity
//!   does not have is a defect in the spec, answered as an error (a `500`),
//!   which each entity's list tests would catch first.

use std::str::FromStr;

use loco_rs::prelude::*;
use sea_orm::{
    AccessMode, DatabaseConnection, IntoSimpleExpr, IsolationLevel, Order, PaginatorTrait,
    QueryOrder, QuerySelect, Select, TransactionTrait, sea_query::Expr,
};

use crate::domain::list_query::{Direction, FilterValue, ListQuery, OrderTerm};

/// One page of a list: its rows, and how many rows the whole list has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<M> {
    /// The rows of the page, in the list's order; fewer than the page size
    /// on the last page, none past it.
    pub rows: Vec<M>,
    /// The number of rows the list has with its filters, on every page.
    pub total: u64,
}

/// Reads one page of `select` and its total, as `query` asks, in one
/// read-only snapshot.
///
/// - `db`: the pool. The read opens its own transaction (the module
///   documentation says why it cannot join the caller's).
/// - `select`: the entity's select, already narrowed to the rows the list
///   shows, such as the live rows. It must have no order, offset or limit
///   of its own.
/// - `query`: the checked request: filters, order and window.
///
/// # Errors
///
/// A database error, or a [`ModelError::Message`] when the spec of `query`
/// names a column that the entity does not have.
pub async fn fetch<E>(
    db: &DatabaseConnection,
    select: Select<E>,
    query: &ListQuery,
) -> ModelResult<Page<E::Model>>
where
    E: EntityTrait,
    E::Model: Sync,
{
    let filtered = filter(select, query)?;
    let mut ordered = filtered.clone();
    for term in query.order() {
        ordered = ordered.order_by(order_expr::<E>(&term)?, order(term.direction));
    }
    let window = ordered.offset(query.offset()).limit(query.limit());

    let txn = db
        .begin_with_config(
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadOnly),
        )
        .await?;
    let total = filtered.count(&txn).await?;
    let rows = window.all(&txn).await?;
    // A read-only transaction has nothing to keep; committing ends it
    // without the warning a dropped transaction logs.
    txn.commit().await?;
    Ok(Page { rows, total })
}

/// `select` with every filter of `query`: the rows must match all of them.
///
/// - `select`: the entity's narrowed select.
/// - `query`: the checked request.
fn filter<E: EntityTrait>(select: Select<E>, query: &ListQuery) -> ModelResult<Select<E>> {
    let mut select = select;
    for filter in &query.filters {
        let column = column::<E>(filter.key.column)?;
        select = match &filter.value {
            FilterValue::Ids(ids) => select.filter(column.is_in(ids.iter().copied())),
        };
    }
    Ok(select)
}

/// The expression one `ORDER BY` term sorts on: the column, table-qualified,
/// with `COLLATE hu_icu` for a text field (ADR 0020 §4).
///
/// - `term`: the term, from [`ListQuery::order`].
///
/// # Errors
///
/// [`ModelError::Message`] when the entity has no column of the term's name.
pub fn order_expr<E: EntityTrait>(term: &OrderTerm) -> ModelResult<Expr> {
    let column = column::<E>(term.column)?.into_simple_expr();
    Ok(match term.collation.sql_name() {
        None => column,
        // `$1` is sea-query's placeholder for the expression; the collation
        // name is the domain's constant, an identifier that needs no
        // quoting.
        Some(collation) => Expr::cust_with_expr(format!("$1 COLLATE {collation}"), column),
    })
}

/// sea-orm's order for a direction of the list contract.
const fn order(direction: Direction) -> Order {
    match direction {
        Direction::Asc => Order::Asc,
        Direction::Desc => Order::Desc,
    }
}

/// The entity's column called `name`.
///
/// # Errors
///
/// [`ModelError::Message`], naming the column and the table, when the entity
/// has none of that name: the list spec and the schema disagree.
fn column<E: EntityTrait>(name: &str) -> ModelResult<E::Column> {
    E::Column::from_str(name).map_err(|_| {
        ModelError::Message(format!(
            "the list spec names the column {name:?}, which the table {:?} does not have; \
                 make the spec name one of its columns",
            E::default().table_name()
        ))
    })
}

#[cfg(test)]
mod tests {
    use sea_orm::{DbBackend, QueryTrait};

    use super::*;
    use crate::{
        domain::{
            labels::LIST,
            list_query::{TIE_BREAK, parse},
        },
        models::_entities::labels,
    };

    /// The SQL of `query`'s window over the labels, as [`fetch`] builds it,
    /// for PostgreSQL.
    fn sql(raw: &str) -> String {
        let query = parse(&LIST, Some(raw)).expect("a valid query");
        let mut select = filter(labels::Entity::find(), &query).expect("filters");
        for term in query.order() {
            select = select.order_by(
                order_expr::<labels::Entity>(&term).expect("a column"),
                order(term.direction),
            );
        }
        select
            .offset(query.offset())
            .limit(query.limit())
            .build(DbBackend::Postgres)
            .to_string()
    }

    #[test]
    fn a_text_sort_names_the_hungarian_collation_and_ends_on_the_id() {
        let sql = sql("sort=name&order=DESC&page=3&perPage=10");
        assert!(
            sql.ends_with(
                r#"ORDER BY "labels"."name" COLLATE hu_icu DESC, "labels"."id" DESC LIMIT 10 OFFSET 20"#
            ),
            "{sql}"
        );
    }

    #[test]
    fn an_id_sort_has_no_tie_break_and_no_collation() {
        let sql = sql("sort=id&order=ASC");
        assert!(
            sql.ends_with(r#"ORDER BY "labels"."id" ASC LIMIT 25 OFFSET 0"#),
            "{sql}"
        );
    }

    #[test]
    fn an_id_filter_is_an_in_list_of_bound_values() {
        let sql = sql("id=7&id=3");
        assert!(sql.contains(r#"WHERE "labels"."id" IN (3, 7)"#), "{sql}");
    }

    #[test]
    fn a_column_the_entity_lacks_is_an_error_that_names_it() {
        let error = column::<labels::Entity>("colour").expect_err("no such column");
        let message = error.to_string();
        assert!(message.contains("\"colour\""), "{message}");
        assert!(message.contains("\"labels\""), "{message}");
        assert!(order_expr::<labels::Entity>(&TIE_BREAK).is_ok());
    }

    #[test]
    fn the_directions_map_to_sea_orm_s() {
        assert_eq!(order(Direction::Asc), Order::Asc);
        assert_eq!(order(Direction::Desc), Order::Desc);
    }
}
