import { useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Plus, RefreshCw, Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Badge, Button, Callout, Card, CardContent, CardDescription, CardHeader, CardTitle, Field, Input, NativeSelect, PageHeader, Spinner, Switch, Tabs, TabsContent, TabsList, TabsTrigger, Textarea } from '@/components/ui'
import { api, type Costs, type Digest, type Domain, type FixedPage, type Job, type Site } from '@/lib/api'
import { useAction, useSite } from '@/lib/hooks'
import { formatDate, formatDateTime, money } from '@/lib/utils'

function useSiteForm(site: Site | undefined) {
  const [form, setForm] = useState<Partial<Site> & { github_token?: string }>({})
  useEffect(() => { if (site) setForm({ ...site, github_token: '' }) }, [site])
  return [form, setForm] as const
}

function General({ site }: { site: Site }) {
  const [f, setF] = useSiteForm(site)
  const save = useAction(() => api.patch(`/api/sites/${site.id}`, { name: f.name, product_description: f.product_description, audience: f.audience, website_url: f.website_url ?? '', author_name: f.author_name }), { success: 'Saved' })
  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-5">
        <Field label="Name"><Input value={f.name ?? ''} onChange={(e) => setF({ ...f, name: e.target.value })} /></Field>
        <Field label="What it does"><Textarea value={f.product_description ?? ''} onChange={(e) => setF({ ...f, product_description: e.target.value })} rows={2} /></Field>
        <Field label="Who it's for"><Input value={f.audience ?? ''} onChange={(e) => setF({ ...f, audience: e.target.value })} /></Field>
        <Field label="Website" hint="Crawled for product facts."><Input value={f.website_url ?? ''} onChange={(e) => setF({ ...f, website_url: e.target.value })} placeholder="https://example.com" /></Field>
        <Field label="Author name on posts"><Input value={f.author_name ?? ''} onChange={(e) => setF({ ...f, author_name: e.target.value })} /></Field>
        <div className="flex justify-end"><Button onClick={() => save.mutate()} loading={save.isPending}>Save</Button></div>
      </CardContent>
    </Card>
  )
}

function Writing({ site }: { site: Site }) {
  const [f, setF] = useSiteForm(site)
  const save = useAction(
    () => api.patch(`/api/sites/${site.id}`, {
      cadence_per_week: Number(f.cadence_per_week), monthly_budget_usd: Number(f.monthly_budget_usd), review_mode: f.review_mode, strict_gates: f.strict_gates,
      model_draft: f.model_draft, model_mid: f.model_mid, model_cheap: f.model_cheap,
    }),
    { success: 'Saved' },
  )
  const costs = useQuery({ queryKey: ['costs', site.id], queryFn: () => api.get<Costs>(`/api/sites/${site.id}/costs`) })
  return (
    <div className="flex flex-col gap-5">
      <Card>
        <CardContent className="grid gap-5 pt-5 sm:grid-cols-2">
          <Field label="Posts per week" hint="Start at 3 and go daily once the library is deep enough not to repeat itself.">
            <NativeSelect value={f.cadence_per_week ?? 3} onChange={(e) => setF({ ...f, cadence_per_week: Number(e.target.value) })}>
              {[1, 2, 3, 4, 5, 6, 7].map((n) => <option key={n} value={n}>{n === 7 ? 'Daily (7)' : n}</option>)}
            </NativeSelect>
          </Field>
          <Field label="Monthly AI budget (USD)" hint="Drafting pauses when it's spent.">
            <Input type="number" min={0} step={1} value={f.monthly_budget_usd ?? 0} onChange={(e) => setF({ ...f, monthly_budget_usd: Number(e.target.value) })} />
          </Field>
          <div className="flex items-start justify-between gap-4 sm:col-span-2">
            <div>
              <div className="text-sm font-medium">Publish automatically</div>
              <p className="text-sm text-muted-foreground">Drafts that pass the structure checks, cite an approved entry in every paragraph and pass a separate AI review are scheduled without you. Anything else waits in the review queue.</p>
            </div>
            <Switch checked={f.review_mode === 'auto'} onCheckedChange={(c) => setF({ ...f, review_mode: c ? 'auto' : 'person' })} aria-label="Publish automatically" />
          </div>
          <div className="flex items-start justify-between gap-4 sm:col-span-2">
            <div>
              <div className="text-sm font-medium">A person approves every library change</div>
              <p className="text-sm text-muted-foreground">Turns off the AI verifier and automatic graph updates for this site.</p>
            </div>
            <Switch checked={!!f.strict_gates} onCheckedChange={(c) => setF({ ...f, strict_gates: c })} aria-label="Strict approval" />
          </div>
          <Field label="Drafting model" hint="Strongest tier, via OpenRouter"><Input value={f.model_draft ?? ''} onChange={(e) => setF({ ...f, model_draft: e.target.value })} /></Field>
          <Field label="Extraction and review model"><Input value={f.model_mid ?? ''} onChange={(e) => setF({ ...f, model_mid: e.target.value })} /></Field>
          <Field label="Tagging model"><Input value={f.model_cheap ?? ''} onChange={(e) => setF({ ...f, model_cheap: e.target.value })} /></Field>
          <div className="flex items-end justify-end sm:col-span-2"><Button onClick={() => save.mutate()} loading={save.isPending}>Save</Button></div>
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>AI cost this month</CardTitle><CardDescription>Recorded on every call, against this site and organization.</CardDescription></CardHeader>
        <CardContent className="text-sm">
          {costs.data && (
            <>
              <div className="mb-3 flex flex-wrap gap-4">
                <div><span className="text-muted-foreground">This site:</span> <span className="font-medium">{money(costs.data.month_spend_usd)}</span> of {money(costs.data.monthly_budget_usd)}</div>
                <div><span className="text-muted-foreground">Whole organization:</span> {money(costs.data.org_month_spend_usd)}</div>
                {costs.data.drafting_paused && <Badge variant="warning">paused</Badge>}
              </div>
              <div className="overflow-x-auto">
                <table className="w-full text-left">
                  <thead className="text-xs text-muted-foreground"><tr><th className="py-1 pr-4">Task</th><th className="pr-4">Model</th><th className="pr-4">Calls</th><th>Cost</th></tr></thead>
                  <tbody>{costs.data.by_task.map((r) => <tr key={r.task + r.model} className="border-t"><td className="py-1.5 pr-4">{r.task.replace(/_/g, ' ')}</td><td className="pr-4 text-muted-foreground">{r.model}</td><td className="pr-4">{r.calls}</td><td>{money(r.cost_usd)}</td></tr>)}</tbody>
                </table>
              </div>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function Blog({ site }: { site: Site }) {
  const [b, setB] = useState(site.branding ?? {})
  useEffect(() => setB(site.branding ?? {}), [site])
  const save = useAction(() => api.patch(`/api/sites/${site.id}`, { branding: b }), { success: 'Branding saved' })
  const pages = useQuery({ queryKey: ['pages', site.id], queryFn: () => api.get<{ pages: FixedPage[] }>(`/api/sites/${site.id}/pages`) })
  const [page, setPage] = useState({ title: '', body_md: '' })
  const addPage = useAction(() => api.post(`/api/sites/${site.id}/pages`, page), { success: 'Page added', onSuccess: () => setPage({ title: '', body_md: '' }) })
  const delPage = useAction((id: string) => api.del(`/api/pages/${id}`), { success: 'Page removed' })
  const nav = b.nav ?? []
  return (
    <div className="flex flex-col gap-5">
      <Card>
        <CardHeader><CardTitle>Branding</CardTitle><CardDescription>One clean theme, styled with your brand. Live at <a className="text-primary underline" href={site.blog_url} target="_blank" rel="noreferrer">{site.blog_url}</a></CardDescription></CardHeader>
        <CardContent className="grid gap-4 sm:grid-cols-2">
          <Field label="Logo URL"><Input value={b.logo_url ?? ''} onChange={(e) => setB({ ...b, logo_url: e.target.value })} placeholder="https://…/logo.svg" /></Field>
          <Field label="Primary colour">
            <div className="flex gap-2">
              <input type="color" value={b.primary_color ?? '#2563eb'} onChange={(e) => setB({ ...b, primary_color: e.target.value })} className="h-9 w-12 rounded border" aria-label="Pick colour" />
              <Input value={b.primary_color ?? ''} onChange={(e) => setB({ ...b, primary_color: e.target.value })} />
            </div>
          </Field>
          <Field label="Font">
            <NativeSelect value={b.font ?? 'system'} onChange={(e) => setB({ ...b, font: e.target.value })}>
              <option value="system">Sans serif</option><option value="serif">Serif</option><option value="humanist">Humanist</option><option value="mono">Monospace</option>
            </NativeSelect>
          </Field>
          <Field label="Footer text"><Input value={b.footer ?? ''} onChange={(e) => setB({ ...b, footer: e.target.value })} /></Field>
          <div className="flex flex-col gap-2 sm:col-span-2">
            <div className="text-sm font-medium">Navigation links</div>
            {nav.map((n, i) => (
              <div key={i} className="flex gap-2">
                <Input value={n.label} placeholder="Label" onChange={(e) => setB({ ...b, nav: nav.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)) })} />
                <Input value={n.url} placeholder="https://" onChange={(e) => setB({ ...b, nav: nav.map((x, j) => (j === i ? { ...x, url: e.target.value } : x)) })} />
                <Button variant="ghost" size="icon" onClick={() => setB({ ...b, nav: nav.filter((_, j) => j !== i) })} aria-label="Remove link"><Trash2 /></Button>
              </div>
            ))}
            <Button variant="outline" size="sm" className="self-start" onClick={() => setB({ ...b, nav: [...nav, { label: '', url: '' }] })}><Plus /> Add link</Button>
          </div>
          <div className="flex justify-end sm:col-span-2"><Button onClick={() => save.mutate()} loading={save.isPending}>Save branding</Button></div>
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Pages</CardTitle><CardDescription>Fixed pages such as About or Contact, linked in the blog's navigation.</CardDescription></CardHeader>
        <CardContent className="flex flex-col gap-3">
          {pages.data?.pages.map((p) => (
            <div key={p.id} className="flex items-center justify-between rounded-md border p-2 text-sm">
              <span>{p.title} <span className="text-muted-foreground">/{p.slug}</span></span>
              <Button size="sm" variant="ghost" onClick={() => delPage.mutate(p.id)} aria-label={`Remove ${p.title}`}><Trash2 /></Button>
            </div>
          ))}
          <form className="flex flex-col gap-2" onSubmit={(e) => { e.preventDefault(); addPage.mutate() }}>
            <Input placeholder="Page title, e.g. About" value={page.title} onChange={(e) => setPage({ ...page, title: e.target.value })} />
            <Textarea placeholder="Page content in Markdown" value={page.body_md} onChange={(e) => setPage({ ...page, body_md: e.target.value })} rows={4} />
            <Button type="submit" variant="outline" className="self-end" disabled={!page.title.trim()} loading={addPage.isPending}>Add page</Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}

function Domains({ site }: { site: Site }) {
  const q = useQuery({ queryKey: ['domains', site.id], queryFn: () => api.get<{ domains: Domain[]; cname_target: string }>(`/api/sites/${site.id}/domains`) })
  const [host, setHost] = useState('')
  const add = useAction(() => api.post(`/api/sites/${site.id}/domains`, { hostname: host }), { success: 'Domain added. Add the DNS record below.', onSuccess: () => setHost('') })
  const check = useAction((id: string) => api.post(`/api/domains/${id}/check`), { success: 'Status refreshed' })
  const remove = useAction((id: string) => api.del(`/api/domains/${id}`), { success: 'Domain removed' })
  return (
    <Card>
      <CardHeader>
        <CardTitle>Domains</CardTitle>
        <CardDescription>Every site is live at <a className="text-primary underline" href={site.blog_url} target="_blank" rel="noreferrer">{site.blog_url}</a>. Connect your own domain with one DNS record; HTTPS is automatic.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); add.mutate() }}>
          <Input placeholder="blog.yourcompany.com" value={host} onChange={(e) => setHost(e.target.value)} aria-label="Domain" />
          <Button type="submit" loading={add.isPending} disabled={!host.trim()}>Connect</Button>
        </form>
        {q.data?.domains.map((d) => (
          <div key={d.id} className="flex flex-col gap-2 rounded-lg border p-3 text-sm">
            <div className="flex items-center justify-between gap-2">
              <span className="font-medium">{d.hostname}</span>
              <div className="flex items-center gap-1">
                <Badge variant={d.status === 'active' ? 'success' : d.status === 'failed' ? 'destructive' : 'warning'}>{d.status}</Badge>
                <Button size="sm" variant="ghost" onClick={() => check.mutate(d.id)} aria-label="Refresh status"><RefreshCw /></Button>
                <Button size="sm" variant="ghost" onClick={() => remove.mutate(d.id)} aria-label="Remove domain"><Trash2 /></Button>
              </div>
            </div>
            {d.status !== 'active' && (
              <div className="rounded-md bg-muted p-2 font-mono text-xs">
                CNAME {d.hostname} → {q.data?.cname_target}
              </div>
            )}
          </div>
        ))}
      </CardContent>
    </Card>
  )
}

function GitHubSettings({ site }: { site: Site }) {
  const [f, setF] = useSiteForm(site)
  const save = useAction(() => api.patch(`/api/sites/${site.id}`, { github_repo: f.github_repo ?? '', github_branch: f.github_branch, github_folder: f.github_folder, ...(f.github_token ? { github_token: f.github_token } : {}) }), { success: 'Saved' })
  return (
    <Card>
      <CardHeader><CardTitle>GitHub publishing</CardTitle><CardDescription>For sites built from files in git (Astro, Next.js, Hugo…). Published posts are committed as Markdown with front matter.</CardDescription></CardHeader>
      <CardContent className="grid gap-4 sm:grid-cols-2">
        <Field label="Repository" hint="owner/name"><Input value={f.github_repo ?? ''} onChange={(e) => setF({ ...f, github_repo: e.target.value })} placeholder="acme/website" /></Field>
        <Field label="Branch"><Input value={f.github_branch ?? 'main'} onChange={(e) => setF({ ...f, github_branch: e.target.value })} /></Field>
        <Field label="Folder"><Input value={f.github_folder ?? ''} onChange={(e) => setF({ ...f, github_folder: e.target.value })} /></Field>
        <Field label="Access token" hint={site.github_token_set ? 'A token is saved. Enter a new one to replace it.' : 'A fine-grained token with Contents: write on that repository.'}>
          <Input type="password" value={f.github_token ?? ''} onChange={(e) => setF({ ...f, github_token: e.target.value })} autoComplete="off" />
        </Field>
        <div className="flex justify-end sm:col-span-2"><Button onClick={() => save.mutate()} loading={save.isPending}>Save</Button></div>
      </CardContent>
    </Card>
  )
}

function Activity({ site }: { site: Site }) {
  const jobs = useQuery({ queryKey: ['jobs', site.id], queryFn: () => api.get<{ jobs: Job[]; active: number }>(`/api/sites/${site.id}/jobs`) })
  const digests = useQuery({ queryKey: ['digests', site.id], queryFn: () => api.get<{ digests: Digest[] }>(`/api/sites/${site.id}/digests`) })
  return (
    <div className="flex flex-col gap-5">
      <Card>
        <CardHeader><CardTitle>Weekly digests</CardTitle><CardDescription>What the AI approved on its own each week, emailed to organization owners.</CardDescription></CardHeader>
        <CardContent className="flex flex-col gap-3 text-sm">
          {digests.data?.digests.length === 0 && <p className="text-muted-foreground">The first digest arrives a week after the site is created.</p>}
          {digests.data?.digests.map((d) => (
            <div key={d.id} className="rounded-md border p-3">
              <div className="mb-1 font-medium">{formatDate(d.period_start)} – {formatDate(d.period_end)}</div>
              {d.items.entries.length === 0 && d.items.entities.length === 0 ? <p className="text-muted-foreground">No AI approvals.</p> : (
                <ul className="list-disc pl-5">
                  {d.items.entries.map((e) => <li key={e.id}>E{e.num} ({e.kind}): {e.body}</li>)}
                  {d.items.entities.map((e) => <li key={e.id}>New {e.kind}: {e.name}</li>)}
                </ul>
              )}
            </div>
          ))}
        </CardContent>
      </Card>
      <Card>
        <CardHeader><CardTitle>Background jobs</CardTitle></CardHeader>
        <CardContent className="flex flex-col gap-1.5 text-sm">
          {jobs.data?.jobs.map((j) => (
            <div key={j.id} className="flex flex-col gap-0.5 border-b py-1.5 last:border-0">
              <div className="flex items-center justify-between gap-2">
                <span>{j.kind.replace(/_/g, ' ')}</span>
                <span className="flex items-center gap-2 text-xs text-muted-foreground">{formatDateTime(j.created_at)} <Badge variant={j.status === 'succeeded' ? 'success' : j.status === 'dead' ? 'destructive' : j.status === 'failed' ? 'warning' : 'muted'}>{j.status}</Badge></span>
              </div>
              {j.last_error && j.status !== 'succeeded' && <span className="text-xs text-destructive">{j.last_error}</span>}
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  )
}

export function SettingsPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/settings' })
  const site = useSite(siteId)
  if (site.isLoading || !site.data) return <Spinner />
  const s = site.data
  return (
    <div className="max-w-4xl">
      <PageHeader title="Settings" description={s.name} />
      {s.drafting_paused && <Callout variant="warning" className="mb-4" title="Drafting is paused">The monthly budget is spent. Raise it under Writing to resume.</Callout>}
      <Tabs defaultValue="general">
        <TabsList>
          <TabsTrigger value="general">General</TabsTrigger>
          <TabsTrigger value="writing">Writing and budget</TabsTrigger>
          <TabsTrigger value="blog">Blog</TabsTrigger>
          <TabsTrigger value="domains">Domains</TabsTrigger>
          <TabsTrigger value="github">GitHub</TabsTrigger>
          <TabsTrigger value="activity">Activity</TabsTrigger>
        </TabsList>
        <TabsContent value="general"><General site={s} /></TabsContent>
        <TabsContent value="writing"><Writing site={s} /></TabsContent>
        <TabsContent value="blog"><Blog site={s} /></TabsContent>
        <TabsContent value="domains"><Domains site={s} /></TabsContent>
        <TabsContent value="github"><GitHubSettings site={s} /></TabsContent>
        <TabsContent value="activity"><Activity site={s} /></TabsContent>
      </Tabs>
    </div>
  )
}
