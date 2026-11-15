/**
 * Buttons that appear only when the signed-in account may take their
 * action (ADR 0007 §7): the create, edit, delete, bulk-delete, restore and
 * permanent-delete („Végleges törlés”) buttons of every screen.
 *
 * Why this module exists: the kit's pages and its sidebar ask
 * `authProvider.canAccess` before they render, but none of the kit's
 * buttons does, so a list that an account may only read would still offer
 * „Létrehozás”, and a click would end in the server's `403`. The admin
 * hides what the server forbids (ADR 0007 §7): each button here asks
 * ra-core's `useCanAccess` first and renders only on a yes. The answer
 * comes from the access map of `/auth/me` without a request
 * (`./auth-provider.ts`, `./access.ts`), and when a `403` shows the map
 * changed, `invalidateAccessChecks` (`./access-checks.ts`) makes every
 * mounted button ask again.
 *
 * The conventions:
 *
 * - **The record goes with the question.** An action on a record (edit,
 *   delete, restore, purge) asks about that record, so the record-level
 *   rules of `RECORD_RULES` (a published post, the user guards of ADR 0007
 *   §6) decide it as they decide the server's answer. The button then runs
 *   in the context of the record it was checked for. Without a record there
 *   is nothing to act on, and the button stays hidden.
 * - **Create asks about the resource only.** ra-core's `useCanAccess`
 *   takes the record of the surrounding context when it is given none, which
 *   on another resource's page would hand a create question someone else's
 *   record; the check runs in an empty record context instead.
 * - **Bulk delete asks about every selected record** that the list has
 *   loaded, after the resource itself, and appears only when every answer is
 *   yes. A record selected on another page is not loaded, so only the
 *   resource's answer covers it; the server still checks each row.
 * - **Hidden until the answer, and on any doubt.** A button renders nothing
 *   while its question is open, when the answer is no, and when the
 *   question failed. ra-core's `<CanAccess>` would send the user to
 *   `/authentication-error` on a failure, which suits a page, not one of its
 *   buttons; the page itself asks its own question and goes there.
 *
 * Every screen takes these instead of the kit's `CreateButton`,
 * `EditButton`, `DeleteButton` and `BulkDeleteButton`, the kit's own pages
 * included (`src/components/admin/LOCAL_CHANGES.md`); the lint rule
 * `local/no-ungated-buttons` fails any other import of them. The restore
 * and purge buttons come with the trash: their wrappers take the button
 * as `children`. The server still decides every request itself (ADR 0007
 * §5).
 */
import type { ReactNode } from "react"
import {
  RecordContextProvider,
  useCanAccess,
  useListContext,
  useRecordContext,
  useResourceContext,
  type RaRecord,
} from "ra-core"

import type { AccessAction } from "@/bindings/AccessAction"
import {
  BulkDeleteButton,
  type BulkDeleteButtonProps,
} from "@/components/admin/bulk-delete-button"
import {
  CreateButton,
  type CreateButtonProps,
} from "@/components/admin/create-button"
import {
  DeleteButton,
  type DeleteButtonProps,
} from "@/components/admin/delete-button"
import {
  EditButton,
  type EditButtonProps,
} from "@/components/admin/edit-button"

/** Props of the internal {@link Check}. */
interface CheckProps {
  /** The action asked about. */
  readonly action: AccessAction
  /** The ra-core resource; ra-core's hook throws without one. */
  readonly resource: string | undefined
  /** The record asked about, or none for a question about the resource. */
  readonly record: RaRecord | undefined
  /** What to render when the answer is yes. */
  readonly children: ReactNode
}

/**
 * Asks `canAccess` once and renders `children` only on a yes.
 *
 * @param props - see {@link CheckProps}.
 * @returns `children` when the account may take the action, otherwise
 *   nothing: also while the question is open and when it failed.
 */
function Check({ action, resource, record, children }: CheckProps) {
  const { canAccess, error, isPending } = useCanAccess({
    action,
    resource,
    record,
  })
  return !isPending && !error && canAccess === true ? children : null
}

/** Props of {@link RecordAccessGate}. */
export interface RecordAccessGateProps {
  /** The action on the record, such as `edit` or `purge`. */
  readonly action: AccessAction
  /** The ra-core resource; by default the resource context's. */
  readonly resource?: string
  /** The record; by default the record context's. */
  readonly record?: RaRecord
  /** The button, rendered in the context of the record it was checked for. */
  readonly children: ReactNode
}

/**
 * Renders `children` only when the account may take `action` on the record.
 *
 * @param props - see {@link RecordAccessGateProps}.
 * @returns `children` inside a record context of the checked record when
 *   the answer is yes; nothing when it is not, while it is open, when it
 *   failed, and when there is no record to ask about.
 */
export function RecordAccessGate({
  action,
  resource,
  record,
  children,
}: RecordAccessGateProps) {
  const resolvedResource = useResourceContext({ resource })
  const resolvedRecord = useRecordContext({ record })
  if (resolvedRecord === undefined) {
    return null
  }
  return (
    <Check action={action} resource={resolvedResource} record={resolvedRecord}>
      <RecordContextProvider value={resolvedRecord}>
        {children}
      </RecordContextProvider>
    </Check>
  )
}

/** Props of {@link ResourceAccessGate}. */
export interface ResourceAccessGateProps {
  /** The action on the resource, such as `create`. */
  readonly action: AccessAction
  /** The ra-core resource; by default the resource context's. */
  readonly resource?: string
  /** The button, rendered in the surrounding record context. */
  readonly children: ReactNode
}

/**
 * Renders `children` only when the account may take `action` on the
 * resource, asked without a record.
 *
 * @param props - see {@link ResourceAccessGateProps}.
 * @returns `children` when the answer is yes; nothing when it is not,
 *   while it is open and when it failed.
 */
export function ResourceAccessGate({
  action,
  resource,
  children,
}: ResourceAccessGateProps) {
  const resolvedResource = useResourceContext({ resource })
  const surrounding = useRecordContext()
  // ra-core's useCanAccess falls back to the record context when it gets no
  // record, so the question is asked in an empty one; the children get the
  // surrounding record back.
  return (
    <RecordContextProvider value={undefined}>
      <Check action={action} resource={resolvedResource} record={undefined}>
        <RecordContextProvider value={surrounding}>
          {children}
        </RecordContextProvider>
      </Check>
    </RecordContextProvider>
  )
}

/** Props of {@link SelectedRecordsAccessGate}. */
export interface SelectedRecordsAccessGateProps {
  /** The action on each selected record, such as `delete`. */
  readonly action: AccessAction
  /** The ra-core resource; by default the resource context's. */
  readonly resource?: string
  /** The bulk button, rendered in the surrounding contexts. */
  readonly children: ReactNode
}

/**
 * Renders `children` only when the account may take `action` on every
 * selected record of the surrounding list that the list has loaded.
 *
 * @param props - see {@link SelectedRecordsAccessGateProps}.
 * @returns `children` when every loaded selected record is allowed (also
 *   when none is loaded); nothing while a question is open, when an answer
 *   is no and when a question failed.
 */
export function SelectedRecordsAccessGate({
  action,
  resource,
  children,
}: SelectedRecordsAccessGateProps) {
  const resolvedResource = useResourceContext({ resource })
  const { data, selectedIds } = useListContext()
  const selected = (data ?? []).filter((record: RaRecord) =>
    selectedIds.includes(record.id)
  )
  // One check per record, nested, so the children render only inside the
  // last one, when every answer was yes; nesting needs no state to combine
  // the answers. A record already asked about answers from ra-core's cache,
  // so a growing selection waits only for the new record's answer.
  return selected.reduceRight<ReactNode>(
    (inner, record) => (
      <Check
        key={String(record.id)}
        action={action}
        resource={resolvedResource}
        record={record}
      >
        {inner}
      </Check>
    ),
    children
  )
}

/**
 * The kit's `CreateButton`, shown only when the account may create records
 * of the resource.
 *
 * @param props - the kit button's props, passed on unchanged.
 * @returns the button, or nothing.
 */
export function CanAccessCreateButton(props: CreateButtonProps) {
  return (
    <ResourceAccessGate action="create" resource={props.resource}>
      <CreateButton {...props} />
    </ResourceAccessGate>
  )
}

/**
 * The kit's `EditButton`, shown only when the account may edit the record.
 *
 * @param props - the kit button's props, passed on unchanged; `record`
 *   defaults to the record context's.
 * @returns the button, or nothing.
 */
export function CanAccessEditButton(props: EditButtonProps) {
  return (
    <RecordAccessGate
      action="edit"
      resource={props.resource}
      record={props.record}
    >
      <EditButton {...props} />
    </RecordAccessGate>
  )
}

/** Props of {@link CanAccessDeleteButton}: the kit button's, and a record. */
export type CanAccessDeleteButtonProps = DeleteButtonProps & {
  /** The record to delete; by default the record context's. */
  readonly record?: RaRecord
}

/**
 * The kit's `DeleteButton` (the soft delete into the trash, ADR 0011),
 * shown only when the account may delete the record.
 *
 * @param props - the kit button's props, passed on unchanged, and the
 *   record, which the kit button reads from the context the gate gives it.
 * @returns the button, or nothing.
 */
export function CanAccessDeleteButton({
  record,
  ...props
}: CanAccessDeleteButtonProps) {
  return (
    <RecordAccessGate action="delete" resource={props.resource} record={record}>
      <DeleteButton {...props} />
    </RecordAccessGate>
  )
}

/**
 * The kit's `BulkDeleteButton`, shown only when the account may delete
 * records of the resource and every selected record the list has loaded.
 *
 * @param props - the kit button's props, passed on unchanged.
 * @returns the button, or nothing.
 */
export function CanAccessBulkDeleteButton<
  RecordType extends RaRecord = RaRecord,
  MutationOptionsError = unknown,
>(props: BulkDeleteButtonProps<RecordType, MutationOptionsError>) {
  return (
    <ResourceAccessGate action="delete" resource={props.resource}>
      <SelectedRecordsAccessGate action="delete" resource={props.resource}>
        <BulkDeleteButton {...props} />
      </SelectedRecordsAccessGate>
    </ResourceAccessGate>
  )
}

/** Props of {@link CanAccessRestoreButton} and {@link CanAccessPurgeButton}. */
export interface CanAccessTrashButtonProps {
  /** The ra-core resource; by default the resource context's. */
  readonly resource?: string
  /** The trashed record; by default the record context's. */
  readonly record?: RaRecord
  /** The button that the trash screen supplies. */
  readonly children: ReactNode
}

/**
 * A restore button („Visszaállítás” from the trash, ADR 0011), shown only
 * when the account may restore the record.
 *
 * @param props - see {@link CanAccessTrashButtonProps}.
 * @returns the button in the record's context, or nothing.
 */
export function CanAccessRestoreButton(props: CanAccessTrashButtonProps) {
  return <RecordAccessGate action="restore" {...props} />
}

/**
 * A permanent-delete button („Végleges törlés”, ADR 0011), shown only when
 * the account may purge the record: only the `admin` role may (ADR 0007
 * §2).
 *
 * @param props - see {@link CanAccessTrashButtonProps}.
 * @returns the button in the record's context, or nothing.
 */
export function CanAccessPurgeButton(props: CanAccessTrashButtonProps) {
  return <RecordAccessGate action="purge" {...props} />
}
