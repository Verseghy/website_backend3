/**
 * What `authProvider.canAccess` answers (ADR 0007 §7): whether the
 * signed-in account may take an action on an ra-core resource, and on one
 * record of it.
 *
 * Why this module exists: the server computes the account's access map from
 * its access table (`src/domain/authz.rs`, the rows of ADR 0007 §4) and sends
 * it in `/auth/me`; the admin answers every access question from that
 * answer, without a request. The admin never decides access on its own: it
 * hides what the server forbids, and the server still checks every request
 * (ADR 0007 §5).
 *
 * The record-level rules of ADR 0007 §5 (a published post, the user guards of
 * §6) are the one part the admin has to compute itself, because they read
 * the record. {@link RECORD_RULES} mirrors them, one function per resource;
 * none exists yet, and the posts and users slices add theirs next to the
 * Rust rule they mirror. The case table that the Rust and the Vitest tests
 * both run (`tests/fixtures/authz_cases.json`; `./authz-cases.test.ts`)
 * keeps the two sides from drifting apart.
 *
 * Everything here is pure: the caller passes the `/auth/me` answer.
 */
import type { AccessAction } from "@/bindings/AccessAction"
import type { MeResponse } from "@/bindings/MeResponse"

import { accessActionsOf, isAccessAction } from "./authz"

/**
 * The part of a `/auth/me` answer that an access question reads: the
 * access map, and what a record-level rule may need about the account (its
 * public id for the "no self-changes" guard, its effective permissions,
 * and whether it holds the `admin` role).
 */
export type AccessSubject = Pick<
  MeResponse,
  "pid" | "permissions" | "is_admin" | "access"
>

/**
 * An access question, as ra-core passes it to `authProvider.canAccess`.
 */
export interface AccessQuestion {
  /** The action, such as `list` or `edit`; any text ra-core sends. */
  readonly action: string
  /** The ra-core resource name. */
  readonly resource: string
  /** The record the question is about, if any. */
  readonly record?: unknown
}

/**
 * A record-level rule: whether `subject` may take `action` on `record`,
 * once the access map has allowed `action` on the resource.
 *
 * @param action - the action, one the access map allowed.
 * @param record - the record, a plain object as the data provider gave it.
 * @param subject - the signed-in account.
 * @returns whether the rule allows it.
 */
export type RecordRule = (
  action: AccessAction,
  record: Readonly<Record<string, unknown>>,
  subject: AccessSubject
) => boolean

/**
 * The record-level rules, by ra-core resource name: the admin's mirror of
 * the rules in `src/domain/authz.rs` (ADR 0007 §5, §7).
 *
 * Empty until the slices that add the rules: posts (`publish posts` for a
 * published post and for „kiemelt”) and users (the guards of ADR 0007 §6).
 * Each adds its function here, the same rule in Rust, and the cases with a
 * record to `tests/fixtures/authz_cases.json`; `./authz-cases.test.ts` fails
 * for a rule without such a case.
 *
 * ra-core caches each answer under the record's id only (`useCanAccess`'s
 * query key holds `recordId`, not the record), so a rule that reads a field
 * that can change, such as a post's `published`, keeps its old answer for
 * that record until the question is asked again. The slice that adds such
 * a rule calls `invalidateAccessChecks` (`./access-checks.ts`) after the
 * record changed, for example in the save's `onSuccess`, and tests it.
 */
export const RECORD_RULES: Readonly<Record<string, RecordRule>> = {}

/**
 * Whether `value` is a record that a rule can read: a plain object, not
 * `null` and not an array.
 *
 * @param value - the `record` of an access question.
 * @returns `true` for a plain object.
 */
function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

/**
 * Answers an access question from the account's `/auth/me` answer.
 *
 * @param subject - the signed-in account, as `/auth/me` answered it.
 * @param question - what ra-core asks: the action, the resource and, if
 *   any, the record.
 * @param rules - the record-level rules; {@link RECORD_RULES} in the
 *   application, probe rules in the tests.
 * @returns `true` when the access map lists the action for the resource and
 *   the resource's record-level rule, if it has one and a record is given,
 *   allows the record. An action this build does not know (ra-core may ask
 *   for `export` or `clone`) and a resource the map does not list are
 *   refused: deny by default (ADR 0007 §5). A record that is not a plain
 *   object is treated as no record, so only the access map answers.
 */
export function decideAccess(
  subject: AccessSubject,
  question: AccessQuestion,
  rules: Readonly<Record<string, RecordRule>> = RECORD_RULES
): boolean {
  const { action, resource, record } = question
  if (
    !isAccessAction(action) ||
    !accessActionsOf(subject.access, resource).includes(action)
  ) {
    return false
  }
  // `Object.hasOwn`: a resource named like a prototype member has no rule.
  if (isRecord(record) && Object.hasOwn(rules, resource)) {
    return rules[resource](action, record, subject)
  }
  return true
}
