import { Link, useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { CheckCircle2, Circle, ExternalLink } from 'lucide-react'
import { Badge, Button, Callout, Card, CardContent, CardDescription, CardHeader, CardTitle, PageHeader, Spinner } from '@/components/ui'
import { api, type Calendar, type Question, type Topic } from '@/lib/api'
import { useAction, useSite } from '@/lib/hooks'
import { contentTypeLabels, formatDate, money } from '@/lib/utils'

function Step({ done, title, children, to, cta }: { done: boolean; title: string; children: React.ReactNode; to: string; cta: string }) {
  const { siteId } = useParams({ strict: false }) as { siteId: string }
  return (
    <div className="flex items-start gap-3 py-3">
      {done ? <CheckCircle2 className="mt-0.5 h-5 w-5 shrink-0 text-success" /> : <Circle className="mt-0.5 h-5 w-5 shrink-0 text-muted-foreground" />}
      <div className="min-w-0 flex-1">
        <div className="font-medium">{title}</div>
        <div className="text-sm text-muted-foreground">{children}</div>
      </div>
      {!done && <Button size="sm" variant="outline" asChild><Link to={to} params={{ siteId }}>{cta}</Link></Button>}
    </div>
  )
}

export function SiteOverviewPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/' })
  const site = useSite(siteId)
  const questions = useQuery({ queryKey: ['questions', siteId], queryFn: () => api.get<{ questions: Question[] }>(`/api/sites/${siteId}/questions`) })
  const topics = useQuery({ queryKey: ['topics', siteId], queryFn: () => api.get<{ topics: Topic[] }>(`/api/sites/${siteId}/topics`) })
  const calendar = useQuery({ queryKey: ['calendar', siteId], queryFn: () => api.get<Calendar>(`/api/sites/${siteId}/calendar`) })
  const resume = useAction(() => api.patch(`/api/sites/${siteId}`, { resume_drafting: true }), { success: 'Drafting resumed' })
  if (site.isLoading || !site.data) return <Spinner />
  const s = site.data
  const answered = questions.data?.questions.filter((q) => q.answer && q.kind === 'core').length ?? 0
  const approvedEntries = s.entry_counts?.approved ?? 0
  const approvedTopics = topics.data?.topics.filter((t) => t.status === 'approved').length ?? 0
  const pieces = calendar.data?.pieces ?? []
  const approvedPieces = pieces.filter((p) => p.status !== 'proposed').length
  const published = s.post_counts?.published ?? 0
  const upcoming = pieces.filter((p) => ['approved', 'proposed', 'held'].includes(p.status)).slice(0, 5)
  return (
    <div>
      <PageHeader
        title={s.name}
        description={s.product_description || 'No description yet.'}
        actions={
          <Button variant="outline" asChild>
            <a href={s.blog_url} target="_blank" rel="noreferrer"><ExternalLink /> View blog</a>
          </Button>
        }
      />
      {s.drafting_paused && (
        <Callout variant="warning" title="Drafting is paused" className="mb-5">
          The monthly AI budget of {money(s.monthly_budget_usd)} is spent ({money(s.month_spend_usd)} so far).{' '}
          <Link to="/sites/$siteId/settings" params={{ siteId }} className="font-medium underline">Raise the budget</Link> or{' '}
          <button className="font-medium underline" onClick={() => resume.mutate()}>resume</button> if there is room.
        </Callout>
      )}
      <div className="grid gap-5 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Getting to your first post</CardTitle>
            <CardDescription>Your thinking goes in once. After that, Kwil plans and drafts from it.</CardDescription>
          </CardHeader>
          <CardContent className="divide-y pt-0">
            <Step done={answered >= 8} title="Answer the interview" to="/sites/$siteId/interview" cta="Answer">{answered} of 8 core questions answered.</Step>
            <Step done={approvedEntries >= 5} title="Approve your library" to="/sites/$siteId/library" cta="Review">{approvedEntries} approved entries. Drafts only use what you approve.</Step>
            <Step done={approvedTopics > 0} title="Approve the topic map" to="/sites/$siteId/topics" cta="Plan topics">{approvedTopics} approved topics.</Step>
            <Step done={approvedPieces > 0} title="Approve the calendar" to="/sites/$siteId/calendar" cta="Plan calendar">{approvedPieces} approved pieces at {s.cadence_per_week} a week.</Step>
            <Step done={published > 0} title="Review and publish" to="/sites/$siteId/posts" cta="Open posts">{published} published · {s.post_counts?.in_review ?? 0} waiting for review.</Step>
          </CardContent>
        </Card>
        <div className="flex flex-col gap-5">
          <Card>
            <CardHeader><CardTitle>This month</CardTitle></CardHeader>
            <CardContent className="flex flex-col gap-2 text-sm">
              <div className="flex justify-between"><span className="text-muted-foreground">AI spend</span><span className="font-medium">{money(s.month_spend_usd)} of {money(s.monthly_budget_usd)}</span></div>
              <div className="h-2 overflow-hidden rounded-full bg-muted">
                <div className="h-full bg-primary" style={{ width: `${Math.min(100, ((s.month_spend_usd ?? 0) / Math.max(s.monthly_budget_usd, 0.01)) * 100)}%` }} />
              </div>
              <div className="flex justify-between"><span className="text-muted-foreground">Library</span><span>{approvedEntries} approved · {s.entry_counts?.proposed ?? 0} proposed</span></div>
              <div className="flex justify-between"><span className="text-muted-foreground">Review mode</span><span>{s.review_mode === 'auto' ? 'Automatic' : 'You approve'}</span></div>
            </CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle>Coming up</CardTitle></CardHeader>
            <CardContent className="flex flex-col gap-2">
              {upcoming.length === 0 && <p className="text-sm text-muted-foreground">Nothing planned yet.</p>}
              {upcoming.map((p) => (
                <div key={p.id} className="flex items-start justify-between gap-2 text-sm">
                  <div className="min-w-0">
                    <div className="truncate font-medium">{p.title}</div>
                    <div className="text-xs text-muted-foreground">{formatDate(p.scheduled_for, { weekday: 'short', day: 'numeric', month: 'short' })} · {contentTypeLabels[p.content_type]}</div>
                  </div>
                  <Badge variant={p.status === 'held' ? 'warning' : p.status === 'proposed' ? 'outline' : 'default'}>{p.status}</Badge>
                </div>
              ))}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}
