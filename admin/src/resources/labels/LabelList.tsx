/**
 * The „Címkék” list (parity LBL-01: "list shows name and colour"; SHR-02,
 * SHR-03).
 *
 * A `ResourceList` (25 rows to a page, 10, 25, 50 or 100 on offer, no "all")
 * over `GET /api/admin/labels`, with a sortable header on each column: a
 * click sorts by the column (`sort=name` or `sort=color`), a second click
 * turns the order round. The server sorts names and colours in the
 * Hungarian alphabet (`COLLATE hu_icu`, ADR 0020 §4) and breaks every tie
 * by id, descending (SHR-03), so rows with the same colour keep a fixed
 * order across pages. The labels open sorted by name, A to Zs, the
 * server's default (`domain::labels::LIST`).
 *
 * The column titles are `resources.labels.fields.*` of `hu.ts`; the page
 * title and the sidebar entry are `resources.labels.name`. A row opens
 * nothing yet and the list selects no rows: the forms and the trash come
 * with their own slices, which turn both on together with the pages they
 * lead to.
 */
import type { SortPayload } from "ra-core"

import type { LabelResponse } from "@/bindings/LabelResponse"
import { DataTable } from "@/components/admin"
import { ResourceList } from "@/resources/ResourceList"

import { LabelColorField } from "./LabelColorField"

/**
 * The labels' default order: by name, A to Zs, as the server orders them
 * when a request names no sort (`domain::labels::LIST`).
 */
export const LABEL_LIST_SORT: SortPayload = { field: "name", order: "ASC" }

/**
 * The list screen of the `labels` resource.
 *
 * @returns the list page.
 */
export function LabelList() {
  return (
    <ResourceList<LabelResponse> sort={LABEL_LIST_SORT}>
      <DataTable<LabelResponse> rowClick={false} bulkActionButtons={false}>
        <DataTable.Col source="name" />
        <DataTable.Col source="color">
          <LabelColorField />
        </DataTable.Col>
      </DataTable>
    </ResourceList>
  )
}
