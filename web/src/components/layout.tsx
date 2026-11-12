import { Link, Outlet, useParams, useRouterState } from '@tanstack/react-router'
import {
  BookOpen,
  CalendarDays,
  FileText,
  Globe,
  Inbox as InboxIcon,
  LayoutDashboard,
  ListTree,
  Menu,
  MessagesSquare,
  Network,
  Plus,
  Settings,
  X,
} from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import { AccountControls } from '@/lib/auth'
import { useInbox, useJobWatcher, useMe, useSites } from '@/lib/hooks'
import { cn } from '@/lib/utils'
import { Badge, Spinner } from './ui'

function NavItem({ to, params, icon, children, count, exact }: { to: string; params?: Record<string, string>; icon: ReactNode; children: ReactNode; count?: number; exact?: boolean }) {
  return (
    <Link
      to={to}
      params={params as never}
      activeOptions={{ exact: exact ?? false }}
      className="flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground [&.active]:bg-accent [&.active]:font-medium [&.active]:text-primary [&_svg]:size-4"
    >
      {icon}
      <span className="flex-1 truncate">{children}</span>
      {count ? <Badge className="h-5 min-w-5 justify-center px-1.5">{count}</Badge> : null}
    </Link>
  )
}

function SiteNav({ siteId, inboxCount }: { siteId: string; inboxCount?: number }) {
  const jobs = useJobWatcher(siteId)
  const p = { siteId }
  return (
    <div className="mt-1 flex flex-col gap-0.5 border-l pl-2 ml-3">
      <NavItem to="/sites/$siteId" params={p} icon={<LayoutDashboard />} exact count={inboxCount}>Overview</NavItem>
      <NavItem to="/sites/$siteId/interview" params={p} icon={<MessagesSquare />}>Interview</NavItem>
      <NavItem to="/sites/$siteId/library" params={p} icon={<BookOpen />}>Library</NavItem>
      <NavItem to="/sites/$siteId/topics" params={p} icon={<ListTree />}>Topic map</NavItem>
      <NavItem to="/sites/$siteId/calendar" params={p} icon={<CalendarDays />}>Calendar</NavItem>
      <NavItem to="/sites/$siteId/posts" params={p} icon={<FileText />}>Posts</NavItem>
      <NavItem to="/sites/$siteId/sources" params={p} icon={<Globe />}>Sources</NavItem>
      <NavItem to="/sites/$siteId/graph" params={p} icon={<Network />}>Graph</NavItem>
      <NavItem to="/sites/$siteId/settings" params={p} icon={<Settings />}>Settings</NavItem>
      {(jobs.data?.active ?? 0) > 0 && (
        <div className="flex items-center gap-2 px-2.5 py-1.5 text-xs text-muted-foreground" data-testid="jobs-running">
          <Spinner className="h-3.5 w-3.5" /> {jobs.data!.active} job{jobs.data!.active === 1 ? '' : 's'} running
        </div>
      )}
    </div>
  )
}

function Sidebar({ onNavigate }: { onNavigate?: () => void }) {
  const sites = useSites()
  const inbox = useInbox()
  const params = useParams({ strict: false }) as { siteId?: string }
  return (
    <nav className="flex h-full flex-col gap-4 overflow-y-auto p-3" onClick={(e) => (e.target as HTMLElement).closest('a') && onNavigate?.()}>
      <Link to="/" className="flex items-center gap-2 px-2 py-1">
        <img src="/favicon.svg" alt="" className="h-7 w-7" />
        <span className="text-lg font-semibold tracking-tight">Kwil</span>
      </Link>
      <div className="flex flex-col gap-0.5">
        <NavItem to="/" icon={<InboxIcon />} exact count={inbox.data?.total}>Inbox</NavItem>
      </div>
      <div className="flex flex-col gap-0.5">
        <div className="flex items-center justify-between px-2.5 pb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Sites
          <Link to="/sites/new" className="rounded p-0.5 hover:bg-muted hover:text-foreground" aria-label="New site">
            <Plus className="h-4 w-4" />
          </Link>
        </div>
        {sites.isLoading && <Spinner className="mx-2.5 my-1 h-4 w-4" />}
        {sites.data?.map((s) => (
          <div key={s.id}>
            <NavItem to="/sites/$siteId" params={{ siteId: s.id }} icon={<span className="flex h-4 w-4 items-center justify-center rounded bg-primary/15 text-[10px] font-bold text-primary">{s.name[0]?.toUpperCase()}</span>}>
              {s.name}
            </NavItem>
            {params.siteId === s.id && <SiteNav siteId={s.id} inboxCount={inbox.data?.sites.find((x) => x.site.id === s.id)?.count} />}
          </div>
        ))}
        {sites.data?.length === 0 && (
          <Link to="/sites/new" className="mx-2.5 text-sm text-primary hover:underline">Create your first site</Link>
        )}
      </div>
    </nav>
  )
}

export function AppLayout() {
  const me = useMe()
  const [open, setOpen] = useState(false)
  const path = useRouterState({ select: (s) => s.location.pathname })
  useEffect(() => setOpen(false), [path])
  return (
    <div className="flex h-full">
      <aside className="hidden w-64 shrink-0 border-r bg-card md:block">
        <Sidebar />
      </aside>
      {open && (
        <div className="fixed inset-0 z-40 md:hidden">
          <div className="absolute inset-0 bg-black/40" onClick={() => setOpen(false)} />
          <aside className="absolute inset-y-0 left-0 w-72 max-w-[85%] border-r bg-card shadow-xl">
            <button className="absolute right-3 top-3 rounded p-1 hover:bg-muted" onClick={() => setOpen(false)} aria-label="Close menu">
              <X className="h-5 w-5" />
            </button>
            <Sidebar onNavigate={() => setOpen(false)} />
          </aside>
        </div>
      )}
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center justify-between gap-3 border-b bg-card/80 px-4 backdrop-blur">
          <button className={cn('rounded p-1.5 hover:bg-muted md:hidden')} onClick={() => setOpen(true)} aria-label="Open menu">
            <Menu className="h-5 w-5" />
          </button>
          <div className="hidden text-sm text-muted-foreground md:block">
            {me.data?.ai_provider === 'demo' && <Badge variant="warning">Demo AI: set an OpenRouter key for real writing</Badge>}
          </div>
          <AccountControls orgName={me.data?.org_name} />
        </header>
        <main className="min-w-0 flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 lg:py-8">
            {me.isError ? <div className="text-sm text-destructive">Can't reach the API: {(me.error as Error).message}</div> : <Outlet />}
          </div>
        </main>
      </div>
    </div>
  )
}
