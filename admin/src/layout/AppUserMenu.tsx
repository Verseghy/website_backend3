/**
 * backend3's user menu in the header: the signed-in account's name and
 * address, and „Kijelentkezés”, which ends every session of the account on
 * the server (ADR 0006 §2).
 *
 * Why not the kit's `UserMenu`: its „Kijelentkezés” calls ra-core's
 * `logout()` with no arguments, the same call that ra-core makes by itself
 * after it found no session. The auth provider could not tell the two apart,
 * and a sign-out that the server refused would fail unseen. This menu signs
 * out through `useSignOut` (`src/auth/use-sign-out.ts`), which asks the
 * provider to end the session and shows a failure.
 *
 * It is adapted from the kit's `src/components/admin/user-menu.tsx`
 * (shadcn-admin-kit v2.0.0, MIT; `THIRD_PARTY.md` lists this file with the
 * kit's licence file), so it looks and behaves like the kit's menu: the same
 * shadcn/ui parts and classes, the same `children` slot between the account
 * and the sign-out, and the same ra-core `UserMenuContext`, whose `onClose`
 * lets an item written in the kit's pattern (`useUserMenu().onClose`) close
 * the menu. The changes: the sign-out goes through `useSignOut`, the label
 * adds the account's e-mail address, the trigger has a Hungarian
 * `aria-label`, and the avatar has no image.
 *
 * The avatar shows the first letter of the name, as the kit's menu does; the
 * avatar's source is parity item AUTH-08 (ADR 0006).
 */
import { Children, useMemo, useState, type ReactNode } from "react"
import { LogOut } from "lucide-react"
import { useGetIdentity, UserMenuContext } from "ra-core"

import { useSignOut } from "@/auth/use-sign-out"
import { Avatar, AvatarFallback } from "@/components/ui/avatar"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useT } from "@/i18n/keys"

/** The props of {@link AppUserMenu}. */
export type AppUserMenuProps = {
  /**
   * Further menu items, shown between the account and „Kijelentkezés” with
   * a separator after them, as in the kit's `UserMenu`. They may call
   * ra-core's `useUserMenu()` for `onClose`.
   */
  children?: ReactNode
}

/**
 * The header's user menu.
 *
 * @param props - {@link AppUserMenuProps}.
 * @returns the avatar button, which opens the account's name, address, the
 *   given items and the sign-out item. Until `getIdentity` answers, the
 *   button and the label are empty.
 */
export function AppUserMenu({ children }: AppUserMenuProps) {
  const t = useT()
  const { identity } = useGetIdentity()
  const signOut = useSignOut()
  const [open, setOpen] = useState(false)
  // One value for the menu's lifetime (`setOpen` is stable), so the items
  // that read the context do not re-render each time the menu does.
  const userMenu = useMemo(() => ({ onClose: () => setOpen(false) }), [])
  // `getIdentity` answers an `AdminIdentity` (`auth-provider.ts`); ra-core
  // types it as its general `UserIdentity`, whose extra members are `any`.
  const fullName: string = identity?.fullName ?? ""
  const email: string =
    typeof identity?.email === "string" ? identity.email : ""

  return (
    <UserMenuContext.Provider value={userMenu}>
      <DropdownMenu open={open} onOpenChange={setOpen}>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              className="relative ml-2 h-8 w-8 cursor-pointer rounded-full"
              aria-label={t("ra.auth.user_menu")}
            />
          }
        >
          <Avatar className="h-8 w-8">
            <AvatarFallback>{fullName.charAt(0)}</AvatarFallback>
          </Avatar>
        </DropdownMenuTrigger>
        <DropdownMenuContent className="w-56" align="end">
          <DropdownMenuGroup>
            <DropdownMenuLabel className="font-normal">
              <div className="flex flex-col space-y-1">
                <p className="text-sm leading-none font-medium">{fullName}</p>
                <p className="text-xs leading-none text-muted-foreground">
                  {email}
                </p>
              </div>
            </DropdownMenuLabel>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          {children}
          {Children.count(children) > 0 && <DropdownMenuSeparator />}
          <DropdownMenuItem
            className="cursor-pointer"
            onClick={() => {
              void signOut()
            }}
          >
            <LogOut />
            {t("ra.auth.logout")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </UserMenuContext.Provider>
  )
}
