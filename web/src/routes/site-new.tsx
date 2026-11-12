import { useNavigate } from '@tanstack/react-router'
import { useState } from 'react'
import { Button, Card, CardContent, Field, Input, PageHeader, Textarea } from '@/components/ui'
import { api, type Site } from '@/lib/api'
import { useAction } from '@/lib/hooks'

export function NewSitePage() {
  const navigate = useNavigate()
  const [form, setForm] = useState({ name: '', product_description: '', audience: '', website_url: '' })
  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setForm({ ...form, [k]: e.target.value })
  const create = useAction(() => api.post<Site>('/api/sites', form), {
    success: (s) => `${s.name} created`,
    invalidate: [['sites']],
    onSuccess: (s) => navigate({ to: '/sites/$siteId/interview', params: { siteId: s.id } }),
  })
  return (
    <div className="max-w-2xl">
      <PageHeader title="New site" description="One site is one blog: its own library, topic map, calendar and domain." />
      <Card>
        <CardContent className="pt-5">
          <form className="flex flex-col gap-4" onSubmit={(e) => { e.preventDefault(); create.mutate() }}>
            <Field label="Product or company name">
              <Input value={form.name} onChange={set('name')} placeholder="Ledgerly" required autoFocus />
            </Field>
            <Field label="What it does" hint="One or two sentences. The interview goes deeper.">
              <Textarea value={form.product_description} onChange={set('product_description')} rows={2} placeholder="Invoicing for agencies that bill by milestone." />
            </Field>
            <Field label="Who it's for">
              <Input value={form.audience} onChange={set('audience')} placeholder="Small agencies, 5 to 50 people" />
            </Field>
            <Field label="Website (optional)" hint="Kwil can crawl it later to add product facts to the library.">
              <Input value={form.website_url} onChange={set('website_url')} placeholder="https://ledgerly.com" />
            </Field>
            <div className="flex justify-end">
              <Button type="submit" loading={create.isPending} disabled={!form.name.trim()}>Create site and start the interview</Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
