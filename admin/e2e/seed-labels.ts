/**
 * The labels that `LOCO_ENV=e2e cargo loco db seed` creates in the E2E
 * database: `SEED_LABELS` of `src/domain/seed.rs`, which the label list's
 * tests (`labels.spec.ts`) page through and sort.
 *
 * Every label is invented. There are 30, more than one page of 25 and no
 * more than the largest page of 100; each colour is shared by six labels,
 * so a sort by colour has ties that only the id breaks; and ten of them are
 * ADR 0020's ten words, whose Hungarian order no other collation gives.
 * `tests/seed/mod.rs` fails when these lines differ from the Rust constant,
 * so keep each label on its own line, as written.
 */
export const SEED_LABELS = [
  { name: "ábra", color: "#1E88E5" },
  { name: "alma", color: "#43A047" },
  { name: "cukor", color: "#E53935" },
  { name: "csiga", color: "#FDD835" },
  { name: "olló", color: "#8E24AA" },
  { name: "óra", color: "#1E88E5" },
  { name: "öröm", color: "#43A047" },
  { name: "őz", color: "#E53935" },
  { name: "zúgó", color: "#FDD835" },
  { name: "zsák", color: "#8E24AA" },
  { name: "Próbacímke 01", color: "#1E88E5" },
  { name: "Próbacímke 02", color: "#43A047" },
  { name: "Próbacímke 03", color: "#E53935" },
  { name: "Próbacímke 04", color: "#FDD835" },
  { name: "Próbacímke 05", color: "#8E24AA" },
  { name: "Próbacímke 06", color: "#1E88E5" },
  { name: "Próbacímke 07", color: "#43A047" },
  { name: "Próbacímke 08", color: "#E53935" },
  { name: "Próbacímke 09", color: "#FDD835" },
  { name: "Próbacímke 10", color: "#8E24AA" },
  { name: "Próbacímke 11", color: "#1E88E5" },
  { name: "Próbacímke 12", color: "#43A047" },
  { name: "Próbacímke 13", color: "#E53935" },
  { name: "Próbacímke 14", color: "#FDD835" },
  { name: "Próbacímke 15", color: "#8E24AA" },
  { name: "Próbacímke 16", color: "#1E88E5" },
  { name: "Próbacímke 17", color: "#43A047" },
  { name: "Próbacímke 18", color: "#E53935" },
  { name: "Próbacímke 19", color: "#FDD835" },
  { name: "Próbacímke 20", color: "#8E24AA" },
] as const

/**
 * ADR 0020's ten words in the Hungarian alphabet, the order of `hu_icu`
 * (ADR 0020, "Ordering of the same ten words"): every `c…` before every
 * `cs…`, every `z…` before every `zs…`, and `ö`, `ő` after `o`, `ó`.
 */
export const HUNGARIAN_ORDER = [
  "ábra",
  "alma",
  "cukor",
  "csiga",
  "olló",
  "óra",
  "öröm",
  "őz",
  "zúgó",
  "zsák",
] as const
