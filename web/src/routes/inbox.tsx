import { Link } from '@tanstack/react-router'
import { AlertTriangle, Bell, CalendarDays, CheckCheck, FileText, GitMerge, Inbox as InboxIcon, ListTree, MessageCircleQuestion, Network } from 'lucide-react'
import type { ReactNode } from 'react'
import { EntryCard } from '@/components/entry-card'
import { QuestionForm } from '@/components/question-form'
import { Badge, Button, Card, CardContent, CardDescription, CardHeader, CardTitle, EmptyState, PageHeader, Spinner } from '@/components/ui'
import { api, type InboxSite } from '@/lib/api'
import { useAction, useInbox } from '@/lib/hooks'
import { contentTypeLabels, formatDate } from '@/lib/utils'

function Row({ icon, title, children, action }: { icon: ReactNode; title: ReactNode; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="flex flex-col gap-3 rounded-lg border bg-card p-3 sm:flex-row sm:items-center">
      <div className="flex min-w-0 flex-1 items-start gap-3">
        <div className="mt-0.5 text-muted-foreground [&_svg]:size-4">{icon}</div>
        <div className="min-w-0">
          <div className="text-sm font-medium">{title}</div>
          {children && <div className="text-sm text-muted-foreground">{children}</div>}
        </div>
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  )
}

function SiteInbox({ s }: { s: InboxSite }) {
  const p = { siteId: s.site.id }
  const readNote = useAction((id: string) => api.post(`/api/notifications/${id}/read`), { invalidate: [['inbox']] })
  const approveAll = useAction(() => api.post(`/api/sites/${s.site.id}/entries/approve`, { ids: s.entries.map((e) => e.id) }), {
    success: (r) => `Approved ${(r as { approved: number }).approved} entries`,
  })
  const coverage = s.questions.filter((q) => q.kind === 'coverage')
  const interview = s.questions.filter((q) => q.kind !== 'coverage')
  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between">
        <div>
          <CardTitle>
            <Link to="/sites/$siteId" params={p} className="hover:underline">{s.site.name}</Link>
          </CardTitle>
          <CardDescription>{s.count === 0 ? 'Nothing waiting on you.' : `${s.count} thing${s.count === 1 ? '' : 's'} waiting on you`}</CardDescription>
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {s.notifications.map((n) => (
          <Row key={n.id} icon={n.kind === 'budget_paused' ? <AlertTriangle className="text-warning" /> : <Bell />} title={n.message}
            action={<Button size="sm" variant="ghost" onClick={() => readNote.mutate(n.id)}>Dismiss</Button>} />
        ))}
        {s.drafts.map((d) => (
          <Row key={d.id} icon={<FileText />} title={<>Review draft: {d.title}</>}
            action={<Button size="sm" asChild><Link to="/sites/$siteId/posts/$postId" params={{ siteId: s.site.id, postId: d.id }}>Review</Link></Button>}>
            {contentTypeLabels[d.content_type]} {d.review_notes && `· ${d.review_notes}`}
          </Row>
        ))}
        {coverage.length > 0 && (
          <div className="flex flex-col gap-2 rounded-lg border border-warning/50 bg-warning/5 p-3">
            <div className="flex items-center gap-2 text-sm font-medium"><MessageCircleQuestion className="h-4 w-4" /> A planned piece is on hold until you answer</div>
            {coverage.map((q) => <QuestionForm key={q.id} question={q} />)}
          </div>
        )}
        {s.proposed_pieces > 0 && (
          <Row icon={<CalendarDays />} title={`Approve the calendar: ${s.proposed_pieces} planned piece${s.proposed_pieces === 1 ? '' : 's'}`}
            action={<Button size="sm" asChild><Link to="/sites/$siteId/calendar" params={p}>Review calendar</Link></Button>} />
        )}
        {s.proposed_topics > 0 && (
          <Row icon={<ListTree />} title={`Approve the topic map: ${s.proposed_topics} proposed topics`}
            action={<Button size="sm" asChild><Link to="/sites/$siteId/topics" params={p}>Review topics</Link></Button>} />
        )}
        {s.conflicts > 0 && (
          <Row icon={<GitMerge />} title={`${s.conflicts} conflicting entr${s.conflicts === 1 ? 'y' : 'ies'} to resolve`}
            action={<Button size="sm" asChild><Link to="/sites/$siteId/library" params={p} search={{ tab: 'conflicts' }}>Resolve</Link></Button>} />
        )}
        {s.proposed_graph > 0 && (
          <Row icon={<Network />} title={`${s.proposed_graph} proposed entities or relationships`}
            action={<Button size="sm" variant="outline" asChild><Link to="/sites/$siteId/graph" params={p}>Review graph</Link></Button>} />
        )}
        {s.held.filter((h) => h.status === 'failed').map((h) => (
          <Row key={h.id} icon={<AlertTriangle className="text-destructive" />} title={`Drafting failed: ${h.title}`}
            action={<Button size="sm" variant="outline" asChild><Link to="/sites/$siteId/calendar" params={p}>Open calendar</Link></Button>}>
            {h.hold_reason}
          </Row>
        ))}
        {interview.length > 0 && (
          <Row icon={<MessageCircleQuestion />} title={`${interview.length} interview question${interview.length === 1 ? '' : 's'} to answer`}
            action={<Button size="sm" asChild><Link to="/sites/$siteId/interview" params={p}>Answer</Link></Button>} />
        )}
        {s.entries.length > 0 && (
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between gap-2">
              <div className="text-sm font-medium">Library entries waiting for your approval ({s.entries.length})</div>
              <Button size="sm" variant="outline" onClick={() => approveAll.mutate()} loading={approveAll.isPending}><CheckCheck /> Approve all</Button>
            </div>
            {s.entries.slice(0, 8).map((e) => <EntryCard key={e.id} entry={e} />)}
            {s.entries.length > 8 && (
              <Link to="/sites/$siteId/library" params={p} search={{ tab: 'proposed' }} className="text-sm text-primary hover:underline">See all {s.entries.length}</Link>
            )}
          </div>
        )}
        {s.entries_waiting_on_ai > 0 && <p className="text-xs text-muted-foreground">{s.entries_waiting_on_ai} more entries are waiting for the AI verifier.</p>}
        {s.held.filter((h) => h.status === 'held').length > 0 && coverage.length === 0 && (
          <p className="text-xs text-muted-foreground">
            On hold: {s.held.filter((h) => h.status === 'held').map((h) => `${h.title} (${formatDate(h.scheduled_for)})`).join(', ')}
          </p>
        )}
      </CardContent>
    </Card>
  )
}

export function InboxPage() {
  const inbox = useInbox()
  if (inbox.isLoading) return <Spinner />
  const data = inbox.data
  return (
    <div>
      <PageHeader title="Inbox" description="Everything waiting on you, across all your sites." actions={data && data.total > 0 ? <Badge>{data.total} waiting</Badge> : undefined} />
      {!data || data.sites.length === 0 ? (
        <EmptyState icon={<InboxIcon />} title="No sites yet" action={<Button asChild><Link to="/sites/new">Create a site</Link></Button>}>
          Create a site, answer a short interview, and Kwil will plan and draft posts from what you tell it.
        </EmptyState>
      ) : (
        <div className="flex flex-col gap-5">
          {data.notifications.map((n) => <Row key={n.id} icon={<Bell />} title={n.message} />)}
          {[...data.sites].sort((a, b) => b.count - a.count).map((s) => <SiteInbox key={s.site.id} s={s} />)}
        </div>
      )}
    </div>
  )
}
