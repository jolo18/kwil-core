import { useNavigate, useParams, useSearch } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { BookOpen, CheckCheck, Plus } from 'lucide-react'
import { useState } from 'react'
import { ApprovedBy, EntryCard, StatusBadge } from '@/components/entry-card'
import {
  Badge, Button, Card, CardContent, Dialog, DialogContent, EmptyState, Field, Input, NativeSelect, PageHeader, Spinner, Tabs, TabsContent, TabsList, TabsTrigger, Textarea,
} from '@/components/ui'
import { api, type Conflict, type Entry, type EntryDetail } from '@/lib/api'
import { useAction } from '@/lib/hooks'
import { entryKindLabels, formatDateTime, sourceLabels } from '@/lib/utils'

const KINDS = Object.keys(entryKindLabels)

function AddEntry({ siteId }: { siteId: string }) {
  const [open, setOpen] = useState(false)
  const [kind, setKind] = useState('opinion')
  const [body, setBody] = useState('')
  const add = useAction(() => api.post<Entry>(`/api/sites/${siteId}/entries`, { kind, body }), {
    success: (e) => `Added E${e.num}`,
    onSuccess: () => { setBody(''); setOpen(false) },
  })
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <Button onClick={() => setOpen(true)}><Plus /> Add entry</Button>
      <DialogContent title="Add a library entry" description="One claim, readable on its own. Entries you add are approved straight away.">
        <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); add.mutate() }}>
          <Field label="Type">
            <NativeSelect value={kind} onChange={(e) => setKind(e.target.value)}>
              {KINDS.map((k) => <option key={k} value={k}>{entryKindLabels[k]}</option>)}
            </NativeSelect>
          </Field>
          <Field label="Entry" hint={`${body.trim().split(/\s+/).filter(Boolean).length} words · aim for under 80`}>
            <Textarea value={body} onChange={(e) => setBody(e.target.value)} rows={4} placeholder="We think hourly billing punishes agencies for getting faster." />
          </Field>
          <div className="flex justify-end"><Button type="submit" loading={add.isPending} disabled={!body.trim()}>Add entry</Button></div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

export function EntryDialog({ entryId, onClose }: { entryId: string | null; onClose: () => void }) {
  const q = useQuery({ queryKey: ['entry', entryId], enabled: !!entryId, queryFn: () => api.get<EntryDetail>(`/api/entries/${entryId}`) })
  const [editing, setEditing] = useState(false)
  const [body, setBody] = useState('')
  const [kind, setKind] = useState('')
  const save = useAction(() => api.patch(`/api/entries/${entryId}`, { body, kind }), { success: 'Entry saved', onSuccess: () => setEditing(false) })
  const revert = useAction((version: number) => api.post(`/api/entries/${entryId}/revert`, { version }), { success: 'Reverted' })
  const approve = useAction(() => api.post(`/api/entries/${entryId}/approve`), { success: 'Approved' })
  const reject = useAction(() => api.post(`/api/entries/${entryId}/reject`), { success: 'Rejected' })
  const e = q.data
  return (
    <Dialog open={!!entryId} onOpenChange={(o) => { if (!o) { onClose(); setEditing(false) } }}>
      <DialogContent title={e ? `E${e.num} · ${entryKindLabels[e.kind]}` : 'Entry'} className="max-w-2xl">
        {!e ? <Spinner /> : (
          <div className="flex flex-col gap-5 text-sm">
            <div className="flex flex-wrap gap-2">
              <StatusBadge status={e.status} />
              <ApprovedBy by={e.approved_by} />
              <Badge variant="outline">version {e.version}</Badge>
              {e.topics?.map((t) => <Badge key={t.id} variant="muted">{t.title}</Badge>)}
            </div>
            {editing ? (
              <div className="flex flex-col gap-3">
                <NativeSelect value={kind} onChange={(ev) => setKind(ev.target.value)}>
                  {KINDS.map((k) => <option key={k} value={k}>{entryKindLabels[k]}</option>)}
                </NativeSelect>
                <Textarea value={body} onChange={(ev) => setBody(ev.target.value)} rows={4} />
                <div className="flex justify-end gap-2">
                  <Button variant="outline" onClick={() => setEditing(false)}>Cancel</Button>
                  <Button onClick={() => save.mutate()} loading={save.isPending}>Save</Button>
                </div>
              </div>
            ) : (
              <p className="text-base">{e.body}</p>
            )}
            {!editing && (
              <div className="flex flex-wrap gap-2">
                <Button size="sm" variant="outline" onClick={() => { setBody(e.body); setKind(e.kind); setEditing(true) }}>Edit</Button>
                {e.status === 'proposed' && <Button size="sm" variant="success" onClick={() => approve.mutate()}>Approve</Button>}
                {e.status === 'proposed' && <Button size="sm" variant="outline" onClick={() => reject.mutate()}>Reject</Button>}
              </div>
            )}
            {e.status === 'proposed' && e.gate_reason && <p className="text-muted-foreground">Gate: {e.gate} · {e.gate_reason}</p>}
            {e.replaces && <p className="text-muted-foreground">Would replace E{e.replaces.num}: “{e.replaces.body}”</p>}
            <section>
              <h3 className="mb-2 font-medium">Sources</h3>
              {(e.sources ?? []).length === 0 && <p className="text-muted-foreground">No source recorded.</p>}
              <div className="flex flex-col gap-2">
                {e.sources?.map((s, i) => (
                  <div key={i} className="rounded-md border bg-muted/50 p-3">
                    <div className="text-xs text-muted-foreground">{sourceLabels[s.kind] ?? s.kind}{s.title ? ` · ${s.title}` : ''}{s.kind === 'crawl' && s.reference ? ` · ${s.reference}` : ''}</div>
                    {s.quote && <blockquote className="mt-1 border-l-2 border-primary pl-2 italic">{s.quote}</blockquote>}
                  </div>
                ))}
              </div>
            </section>
            {e.links.length > 0 && (
              <section>
                <h3 className="mb-2 font-medium">Related entries</h3>
                {e.links.map((l) => <p key={l.id} className="text-muted-foreground"><Badge variant={l.kind === 'contradicts' ? 'destructive' : 'muted'}>{l.kind}</Badge> E{l.other_num}: {l.other_body}</p>)}
              </section>
            )}
            {e.entities.length > 0 && (
              <section>
                <h3 className="mb-2 font-medium">About</h3>
                <div className="flex flex-wrap gap-1.5">{e.entities.map((n) => <Badge key={n.id} variant="outline">{n.kind}: {n.name}</Badge>)}</div>
              </section>
            )}
            <section>
              <h3 className="mb-2 font-medium">History</h3>
              <div className="flex flex-col gap-1.5">
                {e.versions.map((v) => (
                  <div key={v.id} className="flex items-start justify-between gap-3 rounded-md border p-2">
                    <div className="min-w-0">
                      <div className="text-xs text-muted-foreground">v{v.version} · {v.status} · by {v.changed_by} · {formatDateTime(v.changed_at)}</div>
                      <div className="truncate">{v.body}</div>
                    </div>
                    {v.version !== e.version && <Button size="sm" variant="ghost" onClick={() => revert.mutate(v.version)}>Restore</Button>}
                  </div>
                ))}
              </div>
            </section>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

function Conflicts({ siteId }: { siteId: string }) {
  const q = useQuery({ queryKey: ['conflicts', siteId], queryFn: () => api.get<{ conflicts: Conflict[] }>(`/api/sites/${siteId}/conflicts`) })
  const resolve = useAction(({ id, action }: { id: string; action: string }) => api.post(`/api/links/${id}/resolve`, { action }), { success: 'Conflict resolved' })
  if (q.isLoading) return <Spinner />
  const list = q.data?.conflicts ?? []
  if (list.length === 0) return <EmptyState title="No conflicts">When a new entry contradicts one you approved, it shows up here for you to decide.</EmptyState>
  return (
    <div className="flex flex-col gap-3">
      {list.map((c) => (
        <Card key={c.id}>
          <CardContent className="grid gap-4 pt-5 md:grid-cols-2">
            <div>
              <div className="mb-1 text-xs font-medium text-muted-foreground">Existing E{c.existing.num}</div>
              <p className="text-sm">{c.existing.body}</p>
            </div>
            <div>
              <div className="mb-1 text-xs font-medium text-muted-foreground">New E{c.new.num} · {c.kind}</div>
              <p className="text-sm">{c.new.body}</p>
            </div>
            <p className="text-xs text-muted-foreground md:col-span-2">{c.note}</p>
            <div className="flex flex-wrap gap-2 md:col-span-2">
              <Button size="sm" onClick={() => resolve.mutate({ id: c.id, action: 'keep_new' })}>Use the new one</Button>
              <Button size="sm" variant="outline" onClick={() => resolve.mutate({ id: c.id, action: 'keep_existing' })}>Keep existing</Button>
              <Button size="sm" variant="ghost" onClick={() => resolve.mutate({ id: c.id, action: 'keep_both' })}>Both are true</Button>
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  )
}

function EntryList({ siteId, status }: { siteId: string; status: string }) {
  const [kind, setKind] = useState('')
  const [search, setSearch] = useState('')
  const [open, setOpen] = useState<string | null>(null)
  const q = useQuery({
    queryKey: ['entries', siteId, status, kind, search],
    queryFn: () => api.get<{ entries: Entry[] }>(`/api/sites/${siteId}/entries?status=${status}&kind=${kind}&q=${encodeURIComponent(search)}`),
  })
  const entries = q.data?.entries ?? []
  const personGated = entries.filter((e) => e.status === 'proposed' && e.gate === 'person')
  const approveAll = useAction(() => api.post(`/api/sites/${siteId}/entries/approve`, { ids: personGated.map((e) => e.id) }), {
    success: (r) => `Approved ${(r as { approved: number }).approved} entries`,
  })
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-2 sm:flex-row">
        <Input placeholder="Search entries" value={search} onChange={(e) => setSearch(e.target.value)} className="sm:max-w-xs" aria-label="Search entries" />
        <NativeSelect value={kind} onChange={(e) => setKind(e.target.value)} className="sm:w-44" aria-label="Filter by type">
          <option value="">All types</option>
          {KINDS.map((k) => <option key={k} value={k}>{entryKindLabels[k]}</option>)}
        </NativeSelect>
        {status === 'proposed' && personGated.length > 1 && (
          <Button variant="outline" className="sm:ml-auto" onClick={() => approveAll.mutate()} loading={approveAll.isPending}><CheckCheck /> Approve all {personGated.length}</Button>
        )}
      </div>
      {q.isLoading && <Spinner />}
      {!q.isLoading && entries.length === 0 && (
        <EmptyState icon={<BookOpen />} title={status === 'proposed' ? 'Nothing waiting for approval' : 'No entries yet'}>
          Entries come from the interview, your website, uploads and your edits to drafts.
        </EmptyState>
      )}
      {entries.map((e) => <EntryCard key={e.id} entry={e} onOpen={() => setOpen(e.id)} />)}
      <EntryDialog entryId={open} onClose={() => setOpen(null)} />
    </div>
  )
}

function Profile({ siteId }: { siteId: string }) {
  const q = useQuery({ queryKey: ['profile', siteId], queryFn: () => api.get<{ profile_md: string }>(`/api/sites/${siteId}/profile`) })
  return (
    <Card>
      <CardContent className="pt-5">
        <p className="mb-3 text-sm text-muted-foreground">Compiled from your approved positioning, audience, features, opinions and phrasing to avoid. It is sent with every AI request for this site.</p>
        {q.isLoading ? <Spinner /> : <pre className="overflow-x-auto whitespace-pre-wrap rounded-md bg-muted p-4 text-sm">{q.data?.profile_md}</pre>}
      </CardContent>
    </Card>
  )
}

export function LibraryPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/library' })
  const { tab } = useSearch({ from: '/app/sites/$siteId/library' })
  const navigate = useNavigate()
  const current = tab ?? 'approved'
  return (
    <div>
      <PageHeader title="Knowledge library" description="What this company knows and believes, one claim per entry. Drafts can only cite approved entries." actions={<AddEntry siteId={siteId} />} />
      <Tabs value={current} onValueChange={(t) => navigate({ to: '/sites/$siteId/library', params: { siteId }, search: { tab: t } })}>
        <TabsList>
          <TabsTrigger value="proposed">Waiting for approval</TabsTrigger>
          <TabsTrigger value="approved">Approved</TabsTrigger>
          <TabsTrigger value="conflicts">Conflicts</TabsTrigger>
          <TabsTrigger value="all">Everything</TabsTrigger>
          <TabsTrigger value="profile">Site profile</TabsTrigger>
        </TabsList>
        <TabsContent value="proposed"><EntryList siteId={siteId} status="proposed" /></TabsContent>
        <TabsContent value="approved"><EntryList siteId={siteId} status="approved" /></TabsContent>
        <TabsContent value="all"><EntryList siteId={siteId} status="" /></TabsContent>
        <TabsContent value="conflicts"><Conflicts siteId={siteId} /></TabsContent>
        <TabsContent value="profile"><Profile siteId={siteId} /></TabsContent>
      </Tabs>
    </div>
  )
}
