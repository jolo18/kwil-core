// Authentication behind one interface, mirroring the backend: Clerk when a publishable key is
// configured, otherwise the local development login (no sign-in screen).

import { ClerkProvider, OrganizationSwitcher, SignIn, UserButton, useAuth, useOrganization } from '@clerk/clerk-react'
import { useEffect, type ReactNode } from 'react'
import { setTokenGetter } from './api'

export const CLERK_KEY = import.meta.env.VITE_CLERK_PUBLISHABLE_KEY as string | undefined
export const authMode: 'clerk' | 'dev' = CLERK_KEY ? 'clerk' : 'dev'

function ClerkBridge({ children }: { children: ReactNode }) {
  const { isLoaded, isSignedIn, getToken } = useAuth()
  const { organization } = useOrganization()
  useEffect(() => {
    setTokenGetter(() => getToken())
    return () => setTokenGetter(null)
  }, [getToken])
  if (!isLoaded) return null
  if (!isSignedIn) {
    return (
      <div className="flex min-h-full items-center justify-center p-6">
        <SignIn />
      </div>
    )
  }
  if (!organization) {
    return (
      <div className="flex min-h-full flex-col items-center justify-center gap-4 p-6 text-center">
        <p className="text-sm text-muted-foreground">Choose or create an organization to continue.</p>
        <OrganizationSwitcher hidePersonal />
      </div>
    )
  }
  // Remount the app when the active organization changes, so every query refetches.
  return <div key={organization.id} className="contents">{children}</div>
}

export function AuthProvider({ children }: { children: ReactNode }) {
  if (authMode === 'clerk') {
    return (
      <ClerkProvider publishableKey={CLERK_KEY!}>
        <ClerkBridge>{children}</ClerkBridge>
      </ClerkProvider>
    )
  }
  return <>{children}</>
}

/** Organization switcher and user menu, or a development badge. */
export function AccountControls({ orgName }: { orgName?: string }) {
  if (authMode === 'clerk') {
    return (
      <div className="flex items-center gap-3">
        <OrganizationSwitcher hidePersonal />
        <UserButton />
      </div>
    )
  }
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="truncate font-medium">{orgName ?? 'Dev Organization'}</span>
      <span className="rounded-full bg-warning/20 px-2 py-0.5 text-xs font-medium text-[oklch(0.5_0.13_70)] dark:text-warning">dev login</span>
    </div>
  )
}
