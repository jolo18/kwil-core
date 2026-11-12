import { Link, useNavigate, useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import type { Editor } from '@tiptap/react'
import { AlertTriangle, CheckCircle2, Download, ExternalLink, FileText, GitBranch, RotateCcw, Save, Send, Undo2, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { MarkdownEditor } from '@/components/editor'
import { Badge, Button, Callout, Card, CardContent, CardHeader, CardTitle, EmptyState, Field, Input, NativeSelect, PageHeader, Spinner, Tabs, TabsContent, TabsList, TabsTrigger, Textarea } from '@/components/ui'
import { api, download, type Entry, type Finding, type Post, type PostDetail } from '@/lib/api'
import { useAction, useSite } from '@/lib/hooks'
import { contentTypeLabels, entryKindLabels, formatDate, formatDateTime } from '@/lib/utils'

const postStatus: Record<string, { label: string; variant: 'warning' | 'default' | 'success' | 'muted' }> = {
  in_review: { label: 'In review', variant: 'warning' },
  scheduled: { label: 'Scheduled', variant: 'default' },
  published: { label: 'Published', variant: 'success' },
  rejected: { label: 'Rejected', variant: 'muted' },
}

export function PostsPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/posts/' })
  const [tab, setTab] = useState('in_review')
  const q = useQuery({ queryKey: ['posts', siteId, tab], queryFn: () => api.get<{ posts: Post[] }>(`/api/sites/${siteId}/posts?status=${tab === 'all' ? '' : tab}`) })
  const posts = q.data?.posts ?? []
  return (
    <div>
      <PageHeader title="Posts" description="Drafts arrive here on their scheduled date. Approve them to publish on the hosted blog." />
      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="in_review">Review queue</TabsTrigger>
          <TabsTrigger value="scheduled">Scheduled</TabsTrigger>
          <TabsTrigger value="published">Published</TabsTrigger>
          <TabsTrigger value="all">All</TabsTrigger>
        </TabsList>
        <TabsContent value={tab}>
          {q.isLoading ? <Spinner /> : posts.length === 0 ? (
            <EmptyState icon={<FileText />} title="Nothing here">Approved calendar items are drafted on their date and appear in the review queue.</EmptyState>
          ) : (
            <div className="flex flex-col gap-2">
              {posts.map((p) => (
                <Link key={p.id} to="/sites/$siteId/posts/$postId" params={{ siteId, postId: p.id }} className="flex flex-col gap-1 rounded-lg border bg-card p-4 hover:border-primary/40 sm:flex-row sm:items-center" data-testid="post-row">
                  <div className="min-w-0 flex-1">
                    <div className="font-medium">{p.title}</div>
                    <div className="text-xs text-muted-foreground">
                      {contentTypeLabels[p.content_type]} · {p.published_at ? `published ${formatDate(p.published_at)}` : p.publish_at ? `publishes ${formatDateTime(p.publish_at)}` : `drafted ${formatDate(p.created_at)}`}
                    </div>
                    {p.review_notes && <div className="text-xs text-muted-foreground">{p.review_notes}</div>}
                  </div>
                  <Badge variant={postStatus[p.status].variant}>{postStatus[p.status].label}</Badge>
                </Link>
              ))}
            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}

function FindingList({ findings, coverage }: { findings: Finding[]; coverage: number }) {
  return (
    <div className="flex flex-col gap-2 text-sm" data-testid="findings">
      {findings.length === 0 ? (
        <div className="flex items-center gap-2 text-success"><CheckCircle2 className="h-4 w-4" /> Structure checks pass</div>
      ) : (
        findings.map((f, i) => (
          <div key={i} className="flex gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <div>
              <div>{f.message}</div>
              {f.excerpt && <div className="text-xs italic text-muted-foreground">“{f.excerpt}…”</div>}
            </div>
          </div>
        ))
      )}
      <div className="text-xs text-muted-foreground">{Math.round(coverage * 100)}% of paragraphs cite a library entry</div>
    </div>
  )
}

function CitationPanel({ siteId, cited, focus, onInsert }: { siteId: string; cited: Entry[]; focus: number | null; onInsert: (num: number) => void }) {
  const [search, setSearch] = useState('')
  const approved = useQuery({
    queryKey: ['entries', siteId, 'approved', '', search],
    queryFn: () => api.get<{ entries: Entry[] }>(`/api/sites/${siteId}/entries?status=approved&q=${encodeURIComponent(search)}`),
  })
  const focused = cited.find((e) => e.num === focus) ?? approved.data?.entries.find((e) => e.num === focus)
  return (
    <div className="flex flex-col gap-3 text-sm">
      {focused && (
        <div className="rounded-md border border-primary/40 bg-accent p-3">
          <div className="mb-1 font-mono text-xs font-semibold text-primary">E{focused.num} · {entryKindLabels[focused.kind]}</div>
          {focused.body}
        </div>
      )}
      <div>
        <div className="mb-1.5 text-xs font-medium text-muted-foreground">Cited in this post ({cited.length})</div>
        <div className="flex max-h-56 flex-col gap-1.5 overflow-y-auto">
          {cited.map((e) => (
            <div key={e.id} className="rounded-md border p-2">
              <span className="mr-1 font-mono text-xs font-semibold text-primary">E{e.num}</span>{e.body}
            </div>
          ))}
        </div>
      </div>
      <div>
        <div className="mb-1.5 text-xs font-medium text-muted-foreground">Insert a citation at the cursor</div>
        <Input placeholder="Search approved entries" value={search} onChange={(e) => setSearch(e.target.value)} />
        <div className="mt-2 flex max-h-56 flex-col gap-1 overflow-y-auto">
          {approved.data?.entries.slice(0, 20).map((e) => (
            <button key={e.id} type="button" className="rounded-md p-1.5 text-left hover:bg-muted" onClick={() => onInsert(e.num)}>
              <span className="mr-1 font-mono text-xs font-semibold text-primary">E{e.num}</span>
              <span className="line-clamp-2">{e.body}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}

export function PostEditorPage() {
  const { siteId, postId } = useParams({ from: '/app/sites/$siteId/posts/$postId' })
  const navigate = useNavigate()
  const site = useSite(siteId)
  const q = useQuery({ queryKey: ['post', postId], queryFn: () => api.get<PostDetail>(`/api/posts/${postId}`) })
  const [body, setBody] = useState<string | null>(null)
  const [meta, setMeta] = useState({ title: '', meta_description: '', timeliness: 'timeless', slug: '' })
  const [findings, setFindings] = useState<{ findings: Finding[]; citation_coverage: number } | null>(null)
  const [focus, setFocus] = useState<number | null>(null)
  const [publishAt, setPublishAt] = useState('')
  const editorRef = useRef<Editor | null>(null)
  const detail = q.data

  useEffect(() => {
    if (detail) {
      setMeta({ title: detail.post.title, meta_description: detail.post.meta_description, timeliness: detail.post.timeliness, slug: detail.post.slug })
      setFindings({ findings: detail.findings, citation_coverage: detail.citation_coverage })
    }
  }, [detail])

  // Live structure checks while editing.
  useEffect(() => {
    if (body === null || !detail) return
    const t = setTimeout(() => {
      api.post<{ findings: Finding[]; citation_coverage: number }>('/api/check', { body_md: body, content_type: detail.post.content_type }).then(setFindings).catch(() => {})
    }, 600)
    return () => clearTimeout(t)
  }, [body, detail])

  const dirty = useMemo(() => {
    if (!detail) return false
    return (body !== null && body.trim() !== detail.post.body_md.trim()) || meta.title !== detail.post.title || meta.meta_description !== detail.post.meta_description || meta.timeliness !== detail.post.timeliness || meta.slug !== detail.post.slug
  }, [body, meta, detail])

  const save = useAction(() => api.patch(`/api/posts/${postId}`, { ...meta, ...(body !== null ? { body_md: body } : {}) }), { success: 'Saved', invalidate: [['post', postId], ['posts']] })
  const approve = useAction(
    async () => {
      if (dirty) await api.patch(`/api/posts/${postId}`, { ...meta, ...(body !== null ? { body_md: body } : {}) })
      return api.post<Post>(`/api/posts/${postId}/approve`, publishAt ? { publish_at: new Date(publishAt).toISOString() } : {})
    },
    { success: (p) => `Approved. Publishes ${formatDateTime(p.publish_at)}.`, invalidate: [['post', postId], ['posts']] },
  )
  const reject = useAction(() => api.post(`/api/posts/${postId}/reject`, { notes: '' }), { success: 'Draft rejected', onSuccess: () => navigate({ to: '/sites/$siteId/posts', params: { siteId } }) })
  const redraft = useAction(() => api.post(`/api/posts/${postId}/redraft`), { success: 'Writing a new draft…', onSuccess: () => navigate({ to: '/sites/$siteId/posts', params: { siteId } }) })
  const unpublish = useAction(() => api.post(`/api/posts/${postId}/unpublish`), { success: 'Unpublished', invalidate: [['post', postId], ['posts']] })
  const github = useAction(() => api.post(`/api/posts/${postId}/github`), { success: 'Committing to GitHub…' })

  const insertCitation = useCallback((num: number) => {
    editorRef.current?.chain().focus().insertContent({ type: 'citation', attrs: { num } }).run()
  }, [])

  if (q.isLoading || !detail) return <Spinner />
  const p = detail.post
  const status = postStatus[p.status]
  const brief = detail.piece?.brief
  return (
    <div>
      <PageHeader
        title={meta.title || p.title}
        description={<span className="flex flex-wrap items-center gap-2"><Badge variant={status.variant}>{status.label}</Badge>{contentTypeLabels[p.content_type]}{p.publish_at && p.status === 'scheduled' && ` · publishes ${formatDateTime(p.publish_at)}`}{p.published_at && ` · published ${formatDate(p.published_at)}`}</span>}
        actions={
          <>
            <Button variant="outline" size="sm" onClick={() => download(`/api/posts/${postId}/markdown`, `${p.slug}.md`)}><Download /> Markdown</Button>
            {p.status === 'published' && <Button variant="outline" size="sm" asChild><a href={detail.public_url} target="_blank" rel="noreferrer"><ExternalLink /> View live</a></Button>}
            {site.data?.github_repo && site.data.github_token_set && <Button variant="outline" size="sm" onClick={() => github.mutate()}><GitBranch /> Commit</Button>}
            <Button variant="outline" size="sm" onClick={() => save.mutate()} disabled={!dirty} loading={save.isPending}><Save /> Save</Button>
          </>
        }
      />
      {p.review_notes && p.status === 'in_review' && <Callout variant="warning" className="mb-4">{p.review_notes}</Callout>}
      <div className="grid gap-5 lg:grid-cols-[1fr_320px]">
        <div className="flex min-w-0 flex-col gap-4">
          <Card>
            <CardContent className="grid gap-4 pt-5 sm:grid-cols-2">
              <Field label="Title" className="sm:col-span-2"><Input value={meta.title} onChange={(e) => setMeta({ ...meta, title: e.target.value })} /></Field>
              <Field label="Meta description" className="sm:col-span-2" hint={`${meta.meta_description.length} characters · about 150 is ideal`}>
                <Textarea value={meta.meta_description} onChange={(e) => setMeta({ ...meta, meta_description: e.target.value })} rows={2} />
              </Field>
              <Field label="URL slug"><Input value={meta.slug} onChange={(e) => setMeta({ ...meta, slug: e.target.value })} /></Field>
              <Field label="Lifespan">
                <NativeSelect value={meta.timeliness} onChange={(e) => setMeta({ ...meta, timeliness: e.target.value })}>
                  <option value="timeless">Timeless</option>
                  <option value="timely">Timely</option>
                </NativeSelect>
              </Field>
            </CardContent>
          </Card>
          <MarkdownEditor key={p.id} value={p.body_md} onChange={setBody} onCitationClick={setFocus} onReady={(e) => (editorRef.current = e)} />
          <Card>
            <CardContent className="flex flex-col gap-3 pt-5 sm:flex-row sm:items-end">
              {p.status === 'in_review' && (
                <>
                  <Field label="Publish at (optional)" hint="Defaults to the planned date" className="sm:w-60">
                    <Input type="datetime-local" value={publishAt} onChange={(e) => setPublishAt(e.target.value)} />
                  </Field>
                  <div className="flex flex-1 flex-wrap justify-end gap-2">
                    {p.piece_id && <Button variant="ghost" onClick={() => redraft.mutate()} loading={redraft.isPending}><RotateCcw /> Redraft</Button>}
                    <Button variant="outline" onClick={() => reject.mutate()} loading={reject.isPending}><X /> Reject</Button>
                    <Button variant="success" onClick={() => approve.mutate()} loading={approve.isPending}><Send /> Approve</Button>
                  </div>
                </>
              )}
              {(p.status === 'published' || p.status === 'scheduled') && (
                <div className="flex flex-1 justify-end gap-2">
                  <Button variant="outline" onClick={() => unpublish.mutate()} loading={unpublish.isPending}><Undo2 /> {p.status === 'published' ? 'Unpublish' : 'Back to review'}</Button>
                  {p.status === 'published' && <Button onClick={() => save.mutate()} disabled={!dirty} loading={save.isPending}><Save /> Update live post</Button>}
                </div>
              )}
            </CardContent>
          </Card>
        </div>
        <div className="flex flex-col gap-4">
          <Card>
            <CardHeader><CardTitle className="text-sm">Page structure</CardTitle></CardHeader>
            <CardContent>{findings && <FindingList findings={findings.findings} coverage={findings.citation_coverage} />}</CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle className="text-sm">Citations</CardTitle></CardHeader>
            <CardContent><CitationPanel siteId={siteId} cited={detail.cited_entries} focus={focus} onInsert={insertCitation} /></CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle className="text-sm">Brief and history</CardTitle></CardHeader>
            <CardContent>
              <Tabs defaultValue="brief">
                <TabsList><TabsTrigger value="brief">Brief</TabsTrigger><TabsTrigger value="revisions">Revisions</TabsTrigger></TabsList>
                <TabsContent value="brief" className="text-sm">
                  {detail.piece ? (
                    <div className="flex flex-col gap-2">
                      {detail.piece.angle && <p>{detail.piece.angle}</p>}
                      {brief?.outline && <ol className="list-decimal pl-5 text-muted-foreground">{brief.outline.map((o, i) => <li key={i}>{o}</li>)}</ol>}
                    </div>
                  ) : <p className="text-muted-foreground">No brief.</p>}
                </TabsContent>
                <TabsContent value="revisions" className="flex flex-col gap-1.5 text-sm">
                  {detail.revisions.map((r) => <div key={r.id} className="flex justify-between"><span>{r.author === 'ai' ? 'AI draft' : 'Your edit'}</span><span className="text-muted-foreground">{formatDateTime(r.created_at)}</span></div>)}
                </TabsContent>
              </Tabs>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  )
}
