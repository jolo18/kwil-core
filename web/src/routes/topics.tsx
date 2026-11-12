import { useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Check, ListTree, Pencil, Plus, Sparkles, Trash2, X } from 'lucide-react'
import { useState } from 'react'
import { Badge, Button, Card, CardContent, Dialog, DialogContent, EmptyState, Field, Input, NativeSelect, PageHeader, Spinner, Textarea } from '@/components/ui'
import { api, type Topic } from '@/lib/api'
import { useAction } from '@/lib/hooks'

function TopicDialog({ siteId, topics, topic, open, onOpenChange }: { siteId: string; topics: Topic[]; topic?: Topic; open: boolean; onOpenChange: (o: boolean) => void }) {
  const [title, setTitle] = useState(topic?.title ?? '')
  const [description, setDescription] = useState(topic?.description ?? '')
  const [parent, setParent] = useState(topic?.parent_id ?? '')
  const save = useAction(
    () =>
      topic
        ? api.patch(`/api/topics/${topic.id}`, { title, description, parent_id: parent || null })
        : api.post(`/api/sites/${siteId}/topics`, { title, description, parent_id: parent || null }),
    { success: topic ? 'Topic saved' : 'Topic added', onSuccess: () => onOpenChange(false) },
  )
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={topic ? 'Edit topic' : 'Add topic'}>
        <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); save.mutate() }}>
          <Field label="Title"><Input value={title} onChange={(e) => setTitle(e.target.value)} required /></Field>
          <Field label="Description"><Textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={2} /></Field>
          <Field label="Parent topic">
            <NativeSelect value={parent} onChange={(e) => setParent(e.target.value)}>
              <option value="">None (a pillar topic)</option>
              {topics.filter((t) => !t.parent_id && t.id !== topic?.id).map((t) => <option key={t.id} value={t.id}>{t.title}</option>)}
            </NativeSelect>
          </Field>
          <div className="flex justify-end"><Button type="submit" loading={save.isPending}>Save</Button></div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function TopicRow({ t, child, siteId, topics }: { t: Topic; child?: boolean; siteId: string; topics: Topic[] }) {
  const [edit, setEdit] = useState(false)
  const approve = useAction(() => api.post(`/api/topics/${t.id}/approve`), { success: 'Topic approved' })
  const reject = useAction(() => api.post(`/api/topics/${t.id}/reject`), { success: 'Topic rejected' })
  const remove = useAction(() => api.del(`/api/topics/${t.id}`), { success: 'Topic removed' })
  return (
    <div className={`flex flex-col gap-2 py-3 sm:flex-row sm:items-center ${child ? 'pl-6' : ''}`} data-testid="topic">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className={child ? 'text-sm' : 'font-medium'}>{t.title}</span>
          {t.status === 'proposed' && <Badge variant="warning">proposed</Badge>}
          {t.gap && <Badge variant="outline">no coverage yet</Badge>}
        </div>
        {t.description && <p className="text-sm text-muted-foreground">{t.description}</p>}
        <p className="text-xs text-muted-foreground">{t.entries} entries · {t.pieces} planned · {t.published} published</p>
      </div>
      <div className="flex shrink-0 gap-1">
        {t.status === 'proposed' && (
          <>
            <Button size="sm" variant="success" onClick={() => approve.mutate()} aria-label={`Approve ${t.title}`}><Check /></Button>
            <Button size="sm" variant="outline" onClick={() => reject.mutate()} aria-label={`Reject ${t.title}`}><X /></Button>
          </>
        )}
        <Button size="sm" variant="ghost" onClick={() => setEdit(true)} aria-label={`Edit ${t.title}`}><Pencil /></Button>
        {t.status === 'approved' && <Button size="sm" variant="ghost" onClick={() => remove.mutate()} aria-label={`Remove ${t.title}`}><Trash2 /></Button>}
      </div>
      {edit && <TopicDialog siteId={siteId} topics={topics} topic={t} open={edit} onOpenChange={setEdit} />}
    </div>
  )
}

export function TopicsPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/topics' })
  const [adding, setAdding] = useState(false)
  const q = useQuery({ queryKey: ['topics', siteId], queryFn: () => api.get<{ topics: Topic[] }>(`/api/sites/${siteId}/topics`) })
  const propose = useAction(() => api.post(`/api/sites/${siteId}/topics/propose`), { success: 'Proposing a topic map. It will appear here in a moment.' })
  const approveAll = useAction(() => api.post(`/api/sites/${siteId}/topics/approve`), { success: (r) => `Approved ${(r as { approved: number }).approved} topics` })
  if (q.isLoading) return <Spinner />
  const topics = q.data?.topics ?? []
  const pillars = topics.filter((t) => !t.parent_id)
  const proposed = topics.filter((t) => t.status === 'proposed').length
  return (
    <div>
      <PageHeader
        title="Topic map"
        description="The pillar topics this site should own, and the subtopics that give it depth. The calendar plans from approved topics and fills the gaps first."
        actions={
          <>
            <Button variant="outline" onClick={() => propose.mutate()} loading={propose.isPending}><Sparkles /> {topics.length ? 'Propose again' : 'Propose a topic map'}</Button>
            <Button variant="outline" onClick={() => setAdding(true)}><Plus /> Add topic</Button>
            {proposed > 0 && <Button onClick={() => approveAll.mutate()} loading={approveAll.isPending}><Check /> Approve all {proposed}</Button>}
          </>
        }
      />
      {topics.length === 0 ? (
        <EmptyState icon={<ListTree />} title="No topic map yet" action={<Button onClick={() => propose.mutate()} loading={propose.isPending}><Sparkles /> Propose from my library</Button>}>
          Kwil proposes pillar topics and subtopics from your interview and library. You edit and approve them.
        </EmptyState>
      ) : (
        <div className="flex flex-col gap-4">
          {pillars.map((p) => (
            <Card key={p.id}>
              <CardContent className="divide-y pt-2">
                <TopicRow t={p} siteId={siteId} topics={topics} />
                {topics.filter((c) => c.parent_id === p.id).map((c) => <TopicRow key={c.id} t={c} child siteId={siteId} topics={topics} />)}
              </CardContent>
            </Card>
          ))}
        </div>
      )}
      {adding && <TopicDialog siteId={siteId} topics={topics} open={adding} onOpenChange={setAdding} />}
    </div>
  )
}
