/**
 * The list page of every backend3 resource: the kit's `List` with the paging
 * of the shared admin list contract (parity SHR-02) and backend3's page
 * actions.
 *
 * Why it exists: the kit's `List` keeps ra-core's and the kit's own
 * defaults, which the admin API does not serve. ra-core asks for 10 rows to
 * a page, the kit's pagination offers 5, 10, 25 and 50, and the kit's
 * default actions hold an export button that reads the whole list as one
 * page of 1,000 rows, which the list contract refuses (`422
 * invalid_per_page`: there is no "all", SHR-02). So every resource's list
 * screen is a `ResourceList`, never the kit's `List` (a test in
 * `ResourceList.test.tsx` checks `src/resources/`), and gets, unless it
 * passes its own:
 *
 * - `perPage`: {@link DEFAULT_PER_PAGE} rows;
 * - `pagination`: the kit's `ListPagination` offering exactly
 *   {@link PAGE_SIZES};
 * - `actions`: {@link ResourceListActions}, the kit's filter button when the
 *   list has filters and the access-gated „Új” when the resource has a
 *   create page, and no export.
 *
 * Each resource still passes its own `sort`, the server's default order of
 * that resource (its `ListSpec`), because ra-core sorts on `id` when a list
 * names none. The server adds the `id` tie-break to every order (SHR-03).
 */
import { useFilterContext, useResourceDefinition, type RaRecord } from "ra-core"

import { CanAccessCreateButton } from "@/auth/access-buttons"
import {
  FilterButton,
  List,
  ListPagination,
  type ListProps,
} from "@/components/admin"
import { DEFAULT_PER_PAGE, PAGE_SIZES } from "@/data-provider/list-contract"

/** The rows-per-page choices: the contract's page sizes, in order. */
const ROWS_PER_PAGE_OPTIONS: number[] = [...PAGE_SIZES]

/**
 * The kit's pagination with the list contract's page sizes.
 *
 * @returns the pagination bar of a resource list.
 */
export function ResourceListPagination() {
  return <ListPagination rowsPerPageOptions={ROWS_PER_PAGE_OPTIONS} />
}

/**
 * The actions above a resource list: the kit's filter button when the list
 * has filters, and „Új” when the resource has a create page and the account
 * may create (`CanAccessCreateButton`, ADR 0007 §7). No export: the list
 * contract serves no page larger than 100 rows.
 *
 * @returns the actions, or nothing when the list has none.
 */
export function ResourceListActions() {
  const filters = useFilterContext()
  const { hasCreate } = useResourceDefinition()
  const hasFilters = filters !== undefined && filters.length > 0
  if (!hasFilters && hasCreate !== true) {
    return null
  }
  return (
    <div className="flex items-center gap-2">
      {hasFilters ? <FilterButton /> : null}
      {hasCreate === true ? <CanAccessCreateButton /> : null}
    </div>
  )
}

/**
 * The kit's `List` with the list contract's paging and backend3's actions.
 *
 * @param props - the kit `List`'s props; `perPage`, `pagination` and
 *   `actions` default as the module documentation says, and every other
 *   prop passes through unchanged.
 * @returns the list page.
 */
export function ResourceList<RecordType extends RaRecord = RaRecord>({
  perPage = DEFAULT_PER_PAGE,
  pagination = <ResourceListPagination />,
  actions = <ResourceListActions />,
  ...rest
}: ListProps<RecordType>) {
  return (
    <List<RecordType>
      {...rest}
      perPage={perPage}
      pagination={pagination}
      actions={actions}
    />
  )
}
