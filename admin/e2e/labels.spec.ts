/**
 * The „Címkék” list in a real browser against backend3 (parity SHR-02,
 * SHR-03, LBL-01): server-side paging, 25 rows by default with 10, 25, 50
 * and 100 on offer; a sort by any column header, with the id as the
 * tie-break; and names in the Hungarian alphabet (ADR 0020 §4). Every test
 * also passes the CSP and console guard of `fixtures.ts`, which a missing
 * Hungarian key fails in the CI build.
 *
 * Each check reads the list's own answers (`GET /api/admin/labels`) and
 * holds the table to them, so it sees both what the server ordered and what
 * the screen shows. It needs the seed account and the seed's labels
 * (`seed-labels.ts`): `LOCO_ENV=e2e cargo loco db seed` after the server has
 * migrated `backend3_e2e` (the E2E CI job does both). No test here writes a
 * label, so the tests share the seeded list in parallel.
 */
import type { Locator, Page } from "@playwright/test"

import { expect, signIn, test } from "./fixtures.ts"
import { HUNGARIAN_ORDER, SEED_LABELS } from "./seed-labels.ts"

/** A label as the list answers it (the `LabelResponse` binding). */
interface Label {
  readonly id: number
  readonly name: string
  readonly color: string
}

/** One page of the list (the `ListResponse` binding). */
interface LabelPage {
  readonly data: readonly Label[]
  readonly total: number
}

/** The labels' list endpoint. */
const LIST_PATH = "/api/admin/labels"

/**
 * Runs `action` and returns the answer of the list request it causes, the
 * first whose query has every value of `expected`.
 *
 * @param page - the test's page.
 * @param expected - query parameters the request must carry.
 * @param action - what makes the screen ask, such as a click.
 * @returns the page of labels the server answered.
 */
async function listAnswer(
  page: Page,
  expected: Readonly<Record<string, string>>,
  action: () => Promise<unknown>
): Promise<LabelPage> {
  const answer = page.waitForResponse((response) => {
    const url = new URL(response.url())
    return (
      url.pathname === LIST_PATH &&
      Object.entries(expected).every(
        ([name, value]) => url.searchParams.get(name) === value
      )
    )
  })
  await action()
  const response = await answer
  expect(response.status(), response.url()).toBe(200)
  return (await response.json()) as LabelPage
}

/**
 * The name cells of the table's rows, top to bottom.
 *
 * @param page - the test's page.
 * @returns the locator of the first cell of every body row.
 */
function nameCells(page: Page): Locator {
  return page.locator("table tbody tr td:first-child")
}

/**
 * The names of `labels`, in order.
 *
 * @param labels - the rows of an answer.
 * @returns their names.
 */
function names(labels: readonly Label[]): string[] {
  return labels.map((label) => label.name)
}

/**
 * The ids of `labels`, in order.
 *
 * @param labels - the rows of an answer.
 * @returns their ids.
 */
function ids(labels: readonly Label[]): number[] {
  return labels.map((label) => label.id)
}

/**
 * The CSS form of a `#rrggbb` colour, as the browser reports a computed
 * colour.
 *
 * @param hex - the colour.
 * @returns `rgb(r, g, b)`.
 */
function rgb(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((start) =>
    Number.parseInt(hex.slice(start, start + 2), 16)
  )
  return `rgb(${r}, ${g}, ${b})`
}

/**
 * Checks that `labels` are in the order a sort on `field` gives: the field
 * in the Hungarian alphabet, and among rows with the same value, the higher
 * id first (SHR-03), whatever the direction.
 *
 * The alphabet is `Intl.Collator("hu")` of the test runner, so Node's ICU,
 * not the browser's: this function runs in Playwright's Node process over
 * the rows the server answered. It holds the server's `hu_icu` order
 * (ADR 0020 §4) to the ICU `hu` order that ADR 0021 §5 names for sorting on
 * the client.
 *
 * @param labels - the rows of an answer.
 * @param field - the sorted field.
 * @param order - the direction.
 * @returns how many neighbours share a value, so a caller can check that
 *   the tie-break was put to the test.
 */
function checkOrder(
  labels: readonly Label[],
  field: "name" | "color",
  order: "ASC" | "DESC"
): number {
  const collator = new Intl.Collator("hu")
  let ties = 0
  for (let index = 1; index < labels.length; index += 1) {
    const before = labels[index - 1]
    const after = labels[index]
    const pair = `${before.name} (${before[field]}, id ${before.id}), then ${after.name} (${after[field]}, id ${after.id})`
    if (before[field] === after[field]) {
      ties += 1
      expect(before.id, `${pair}: a tie goes to the higher id`).toBeGreaterThan(
        after.id
      )
    } else {
      const compared = collator.compare(before[field], after[field])
      expect(
        order === "ASC" ? compared : -compared,
        `${pair}: ${field} ${order}`
      ).toBeLessThan(0)
    }
  }
  return ties
}

/**
 * The names of ADR 0020's ten words among `labels`, in their order.
 *
 * @param labels - the rows of an answer.
 * @returns the ten words, in the order the answer has them.
 */
function tenWords(labels: readonly Label[]): string[] {
  const words = new Set<string>(HUNGARIAN_ORDER)
  return names(labels).filter((name) => words.has(name))
}

test.describe("the „Címkék” list", () => {
  test("pages through the labels, 25 to a page, with 10, 25, 50 and 100 on offer (SHR-02)", async ({
    page,
  }) => {
    await signIn(page)
    const first = await listAnswer(
      page,
      { page: "1", perPage: "25", sort: "name", order: "ASC" },
      () =>
        page
          .locator('[data-sidebar="content"]')
          .getByRole("link", { name: "Címkék", exact: true })
          .click()
    )
    await expect(
      page.getByRole("heading", { name: "Címkék", exact: true })
    ).toBeVisible()
    expect(
      first.total,
      "the seed's labels: run `LOCO_ENV=e2e cargo loco db seed` first"
    ).toBeGreaterThanOrEqual(SEED_LABELS.length)
    expect(first.data).toHaveLength(25)
    await expect(nameCells(page)).toHaveText(names(first.data))
    await expect(page.getByText(`1–25 / ${first.total}`)).toBeVisible()
    // Each row shows its colour as a swatch and as text.
    const firstRow = page.locator("table tbody tr").first()
    await expect(firstRow.getByTestId("label-color-swatch")).toHaveCSS(
      "background-color",
      rgb(first.data[0].color)
    )
    await expect(firstRow).toContainText(first.data[0].color)

    // The page sizes of the list contract, and no "all".
    await page.getByRole("combobox").click()
    await expect(page.getByRole("option")).toHaveText(["10", "25", "50", "100"])

    // Ten to a page: the first ten of the same order.
    const ten = await listAnswer(
      page,
      { page: "1", perPage: "10", sort: "name", order: "ASC" },
      () => page.getByRole("option", { name: "10", exact: true }).click()
    )
    expect(ids(ten.data)).toEqual(ids(first.data.slice(0, 10)))
    await expect(nameCells(page)).toHaveText(names(ten.data))
    await expect(page.getByText(`1–10 / ${first.total}`)).toBeVisible()

    // The next page goes on where the first stopped.
    const second = await listAnswer(
      page,
      { page: "2", perPage: "10", sort: "name", order: "ASC" },
      // The kit renders the page links as Base UI buttons (`role="button"` on
      // the anchor), so they are found by that role.
      () =>
        page.getByRole("button", { name: "Ugrás a következő oldalra" }).click()
    )
    expect(ids(second.data)).toEqual(ids(first.data.slice(10, 20)))
    await expect(nameCells(page)).toHaveText(names(second.data))
    await expect(page.getByText(`11–20 / ${first.total}`)).toBeVisible()
  })

  test("sorts by a column header, with the id as the tie-break (SHR-03)", async ({
    page,
  }) => {
    await signIn(page)
    const all = await listAnswer(
      page,
      { page: "1", perPage: "100", sort: "name", order: "ASC" },
      () => page.goto("/admin/labels?perPage=100")
    )
    // The whole list is on this one page, so every tie is in view.
    expect(all.data).toHaveLength(all.total)

    const ascending = await listAnswer(
      page,
      { page: "1", perPage: "100", sort: "color", order: "ASC" },
      () => page.getByRole("button", { name: "Szín", exact: true }).click()
    )
    expect(
      checkOrder(ascending.data, "color", "ASC"),
      "the seed shares each colour among six labels"
    ).toBeGreaterThan(0)
    await expect(nameCells(page)).toHaveText(names(ascending.data))

    // A second click turns the order round; ties still go to the higher id.
    const descending = await listAnswer(
      page,
      { page: "1", perPage: "100", sort: "color", order: "DESC" },
      () => page.getByRole("button", { name: "Szín", exact: true }).click()
    )
    expect(checkOrder(descending.data, "color", "DESC")).toBeGreaterThan(0)
    await expect(nameCells(page)).toHaveText(names(descending.data))
  })

  test("lists the names in the Hungarian alphabet (ADR 0020 §4)", async ({
    page,
  }) => {
    await signIn(page)
    const ascending = await listAnswer(
      page,
      { page: "1", perPage: "100", sort: "name", order: "ASC" },
      () => page.goto("/admin/labels?perPage=100")
    )
    expect(tenWords(ascending.data)).toEqual([...HUNGARIAN_ORDER])
    checkOrder(ascending.data, "name", "ASC")
    await expect(nameCells(page)).toHaveText(names(ascending.data))

    const descending = await listAnswer(
      page,
      { page: "1", perPage: "100", sort: "name", order: "DESC" },
      () => page.getByRole("button", { name: "Név", exact: true }).click()
    )
    expect(tenWords(descending.data)).toEqual([...HUNGARIAN_ORDER].reverse())
    checkOrder(descending.data, "name", "DESC")
    await expect(nameCells(page)).toHaveText(names(descending.data))
  })
})
