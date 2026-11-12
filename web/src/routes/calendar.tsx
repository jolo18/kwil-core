import { Link, useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { CalendarDays, Check, Pencil, Sparkles, X, Zap } from 'lucide-react'
import { useState } from 'react'
import { Badge, Button, Callout, Card, CardContent, Dialog, DialogContent, EmptyState, Field, Input, NativeSelect, PageHeader, Spinner, Textarea } from '@/components/ui'
import { api, type Calendar, type Piece, type Topic } from '@/lib/api'
import { useAction } from '@/lib/hooks'
import { contentTypeLabels, formatDate } from '@/lib/utils'

const statusVariant: Record<string, 'default' | 'warning' | 'success' | 'destructive' | 'outline' | 'muted'> = {
  proposed: 'outline',
  approved: 'default',
  drafting: 'muted',
  held: 'warning',
  drafted: 'success',
  failed: 'destructive',
}

function weekKey(date: string) {
  const d = new Date(`${date}T00:00:00`)
  const monday = new Date(d)
  monday.setDate(d.getDate() - ((d.getDay() + 6) % 7))
  return monday.toISOString().slice(0, 10)
}

function EditPiece({ piece, topics, open, onOpenChange }: { piece: Piece; topics: Topic[]; open: boolean; onOpenChange: (o: boolean) => void }) {
  const [form, setForm] = useState({
    title: piece.title,
    angle: piece.angle,
    content_type: piece.content_type,
    scheduled_for: piece.scheduled_for,
    topic_id: piece.topic_id ?? '',
    target_keyword: piece.target_keyword ?? '',
    timeliness: piece.timeliness,
  })
  const save = useAction(() => api.patch(`/api/pieces/${piece.id}`, { ...form, topic_id: form.topic_id || undefined }), { success: 'Saved', onSuccess: () => onOpenChange(false) })
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>) => setForm({ ...form, [k]: e.target.value })
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title="Edit planned piece">
        <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); save.mutate() }}>
          <Field label="Title"><Input value={form.title} onChange={set('title')} /></Field>
          <Field label="Angle" hint="What the piece should argue or show."><Textarea value={form.angle} onChange={set('angle')} rows={2} /></Field>
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Type">
              <NativeSelect value={form.content_type} onChange={set('content_type')}>
                {Object.entries(contentTypeLabels).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
              </NativeSelect>
            </Field>
            <Field label="Date"><Input type="date" value={form.scheduled_for} onChange={set('scheduled_for')} /></Field>
            <Field label="Topic">
              <NativeSelect value={form.topic_id} onChange={set('topic_id')}>
                <option value="">None</option>
                {topics.filter((t) => t.status === 'approved').map((t) => <option key={t.id} value={t.id}>{t.parent_id ? '— ' : ''}{t.title}</option>)}
              </NativeSelect>
            </Field>
            <Field label="Lifespan">
              <NativeSelect value={form.timeliness} onChange={set('timeliness')}>
                <option value="timeless">Timeless</option>
                <option value="timely">Timely</option>
              </NativeSelect>
            </Field>
          </div>
          <Field label="Target keyword (optional)" hint="Pieces without search volume are fine: buyer guides and reports help AI answers too."><Input value={form.target_keyword} onChange={set('target_keyword')} /></Field>
          <div className="flex justify-end"><Button type="submit" loading={save.isPending}>Save</Button></div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function PieceRow({ p, siteId, topics }: { p: Piece; siteId: string; topics: Topic[] }) {
  const [edit, setEdit] = useState(false)
  const approve = useAction(() => api.post(`/api/pieces/${p.id}/approve`), { success: 'Approved' })
  const reject = useAction(() => api.post(`/api/pieces/${p.id}/reject`), { success: 'Removed from the calendar' })
  const draftNow = useAction(() => api.post(`/api/pieces/${p.id}/draft-now`), { success: 'Drafting now. The draft will land in your review queue.' })
  return (
    <div className="flex flex-col gap-2 py-3 sm:flex-row sm:items-center" data-testid="piece">
      <div className="w-28 shrink-0 text-sm text-muted-foreground">{formatDate(p.scheduled_for, { weekday: 'short', day: 'numeric', month: 'short' })}</div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{p.title}</span>
          <Badge variant={statusVariant[p.status]}>{p.status}</Badge>
        </div>
        <div className="text-xs text-muted-foreground">
          {contentTypeLabels[p.content_type]}{p.topic_title ? ` · ${p.topic_title}` : ''}{p.timeliness === 'timely' ? ' · timely' : ''}
        </div>
        {p.hold_reason && <p className="mt-1 text-xs text-[oklch(0.5_0.13_70)] dark:text-warning">{p.hold_reason}</p>}
      </div>
      <div className="flex shrink-0 flex-wrap gap-1">
        {p.status === 'proposed' && (
          <>
            <Button size="sm" variant="success" onClick={() => approve.mutate()} aria-label={`Approve ${p.title}`}><Check /></Button>
            <Button size="sm" variant="outline" onClick={() => reject.mutate()} aria-label={`Remove ${p.title}`}><X /></Button>
          </>
        )}
        {['approved', 'failed'].includes(p.status) && <Button size="sm" variant="outline" onClick={() => draftNow.mutate()} loading={draftNow.isPending}><Zap /> Draft now</Button>}
        {p.post_id && (
          <Button size="sm" variant="outline" asChild>
            <Link to="/sites/$siteId/posts/$postId" params={{ siteId, postId: p.post_id }}>Open draft</Link>
          </Button>
        )}
        {['proposed', 'approved', 'held', 'failed'].includes(p.status) && <Button size="sm" variant="ghost" onClick={() => setEdit(true)} aria-label={`Edit ${p.title}`}><Pencil /></Button>}
        {['approved', 'held', 'failed'].includes(p.status) && <Button size="sm" variant="ghost" onClick={() => reject.mutate()} aria-label={`Remove ${p.title}`}><X /></Button>}
      </div>
      {edit && <EditPiece piece={p} topics={topics} open={edit} onOpenChange={setEdit} />}
    </div>
  )
}

export function CalendarPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/calendar' })
  const q = useQuery({ queryKey: ['calendar', siteId], queryFn: () => api.get<Calendar>(`/api/sites/${siteId}/calendar`) })
  const topics = useQuery({ queryKey: ['topics', siteId], queryFn: () => api.get<{ topics: Topic[] }>(`/api/sites/${siteId}/topics`) })
  const propose = useAction(() => api.post(`/api/sites/${siteId}/calendar/propose`), { success: 'Planning the next three weeks…' })
  const approveAll = useAction(() => api.post(`/api/sites/${siteId}/calendar/approve`), { success: (r) => `Approved ${(r as { approved: number }).approved} pieces` })
  if (q.isLoading) return <Spinner />
  const cal = q.data!
  const proposed = cal.pieces.filter((p) => p.status === 'proposed').length
  const weeks = new Map<string, Piece[]>()
  cal.pieces.forEach((p) => {
    const k = weekKey(p.scheduled_for)
    weeks.set(k, [...(weeks.get(k) ?? []), p])
  })
  const hasTopics = (topics.data?.topics ?? []).some((t) => t.status === 'approved')
  return (
    <div>
      <PageHeader
        title="Calendar"
        description={`A rolling plan, ${cal.cadence_per_week} post${cal.cadence_per_week === 1 ? '' : 's'} a week. Approve it once a week; each approved piece is drafted on its date.`}
        actions={
          <>
            <Button variant="outline" onClick={() => propose.mutate()} loading={propose.isPending} disabled={!hasTopics}><Sparkles /> Plan next weeks</Button>
            {proposed > 0 && <Button onClick={() => approveAll.mutate()} loading={approveAll.isPending}><Check /> Approve {proposed} planned</Button>}
          </>
        }
      />
      <div className="mb-5 flex flex-col gap-3">
        {cal.drafting_paused && <Callout variant="warning" title="Drafting is paused">The monthly AI budget is spent. Raise it in Settings to resume.</Callout>}
        {cal.mix.warning && <Callout variant="warning" title="Content mix">{cal.mix.warning}</Callout>}
        {cal.gaps.length > 0 && (
          <Callout title="Topics with no coverage yet">
            <div className="mt-1 flex flex-wrap gap-1.5">{cal.gaps.map((g) => <Badge key={g.id} variant="outline">{g.title}</Badge>)}</div>
          </Callout>
        )}
      </div>
      {cal.pieces.length === 0 ? (
        <EmptyState icon={<CalendarDays />} title="Nothing planned" action={hasTopics ? <Button onClick={() => propose.mutate()} loading={propose.isPending}><Sparkles /> Plan the next three weeks</Button> : <Button asChild variant="outline"><Link to="/sites/$siteId/topics" params={{ siteId }}>Approve a topic map first</Link></Button>}>
          Kwil proposes pieces from gaps in your topic map, keeps comparisons in proportion, and schedules them at your cadence.
        </EmptyState>
      ) : (
        <div className="flex flex-col gap-4">
          {[...weeks.entries()].map(([week, pieces]) => (
            <Card key={week}>
              <CardContent className="pt-4">
                <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Week of {formatDate(week, { day: 'numeric', month: 'long' })}</div>
                <div className="divide-y">{pieces.map((p) => <PieceRow key={p.id} p={p} siteId={siteId} topics={topics.data?.topics ?? []} />)}</div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  )
}
