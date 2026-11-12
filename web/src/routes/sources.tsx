import { Link, useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { FileUp, Globe, RefreshCw } from 'lucide-react'
import { useRef, useState } from 'react'
import { Badge, Button, Card, CardContent, CardDescription, CardHeader, CardTitle, PageHeader } from '@/components/ui'
import { api, type Upload } from '@/lib/api'
import { useAction, useSite } from '@/lib/hooks'
import { formatDateTime } from '@/lib/utils'

export function SourcesPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/sources' })
  const site = useSite(siteId)
  const uploads = useQuery({ queryKey: ['uploads', siteId], queryFn: () => api.get<{ uploads: Upload[] }>(`/api/sites/${siteId}/uploads`) })
  const crawl = useAction(() => api.post(`/api/sites/${siteId}/crawl`), { success: 'Crawling your website. Proposed entries will appear in the library.' })
  const [dragging, setDragging] = useState(false)
  const input = useRef<HTMLInputElement>(null)
  const upload = useAction(
    (files: FileList) => {
      const form = new FormData()
      Array.from(files).forEach((f) => form.append('file', f))
      return api.post(`/api/sites/${siteId}/uploads`, form)
    },
    { success: 'Uploaded. Kwil is reading the files now.', invalidate: [['uploads', siteId]] },
  )
  const s = site.data
  return (
    <div className="max-w-4xl">
      <PageHeader title="Sources" description="Where library entries come from besides the interview. Product facts from your own site and files are checked by the AI verifier; everything else waits for you." />
      <div className="grid gap-5 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2"><Globe className="h-4 w-4" /> Website</CardTitle>
            <CardDescription>
              Crawls up to 200 pages from your sitemap, respecting robots.txt. It re-crawls every ten days to catch changes such as pricing.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-3 text-sm">
            {s?.website_url ? (
              <>
                <div className="truncate"><span className="text-muted-foreground">URL:</span> {s.website_url}</div>
                <div><span className="text-muted-foreground">Last crawled:</span> {s.last_crawled_at ? formatDateTime(s.last_crawled_at) : 'never'}</div>
                <Button onClick={() => crawl.mutate()} loading={crawl.isPending} className="self-start"><RefreshCw /> Crawl now</Button>
              </>
            ) : (
              <p>Add your website URL in <Link to="/sites/$siteId/settings" params={{ siteId }} className="text-primary underline">settings</Link> to crawl it.</p>
            )}
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2"><FileUp className="h-4 w-4" /> Documents and transcripts</CardTitle>
            <CardDescription>PDF, DOCX, Markdown or text, plus sales call transcripts as TXT, VTT or SRT. Up to 20 MB each.</CardDescription>
          </CardHeader>
          <CardContent>
            <div
              className={`flex cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed p-6 text-center text-sm ${dragging ? 'border-primary bg-accent' : ''}`}
              onClick={() => input.current?.click()}
              onDragOver={(e) => { e.preventDefault(); setDragging(true) }}
              onDragLeave={() => setDragging(false)}
              onDrop={(e) => { e.preventDefault(); setDragging(false); if (e.dataTransfer.files.length) upload.mutate(e.dataTransfer.files) }}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => e.key === 'Enter' && input.current?.click()}
            >
              <FileUp className="h-6 w-6 text-muted-foreground" />
              {upload.isPending ? 'Uploading…' : 'Drop files here or click to choose'}
              <input ref={input} type="file" multiple hidden accept=".pdf,.docx,.md,.markdown,.txt,.vtt,.srt" onChange={(e) => e.target.files?.length && upload.mutate(e.target.files)} aria-label="Upload files" />
            </div>
          </CardContent>
        </Card>
      </div>
      <h2 className="mb-3 mt-8 text-sm font-medium text-muted-foreground">Uploaded files</h2>
      <div className="flex flex-col gap-2">
        {uploads.data?.uploads.length === 0 && <p className="text-sm text-muted-foreground">No files yet.</p>}
        {uploads.data?.uploads.map((u) => (
          <div key={u.id} className="flex items-center justify-between gap-3 rounded-lg border bg-card p-3 text-sm">
            <div className="min-w-0">
              <div className="truncate font-medium">{u.filename}</div>
              <div className="text-xs text-muted-foreground">{(u.size_bytes / 1024).toFixed(0)} KB · {formatDateTime(u.created_at)}{u.error ? ` · ${u.error}` : ''}</div>
            </div>
            <Badge variant={u.status === 'processed' ? 'success' : u.status === 'failed' ? 'destructive' : 'muted'}>{u.status}</Badge>
          </div>
        ))}
      </div>
    </div>
  )
}
