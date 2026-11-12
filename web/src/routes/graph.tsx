import { useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Check, Network, X } from 'lucide-react'
import { useState } from 'react'
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, EmptyState, NativeSelect, PageHeader, Spinner } from '@/components/ui'
import { api, type Edge, type Entity } from '@/lib/api'
import { useAction } from '@/lib/hooks'

const kindLabel: Record<string, string> = {
  product: 'Product', feature: 'Features', persona: 'Personas', use_case: 'Use cases', competitor: 'Competitors', integration: 'Integrations', topic: 'Topics',
}
const edgeLabel: Record<string, string> = {
  has_feature: 'has feature', serves: 'serves', competes_with: 'competes with', alternative_to: 'is an alternative to', integrates_with: 'integrates with', part_of_topic: 'is part of',
}

function EntityItem({ e, entities }: { e: Entity; entities: Entity[] }) {
  const [mergeInto, setMergeInto] = useState('')
  const approve = useAction(() => api.post(`/api/entities/${e.id}/approve`), { success: `${e.name} approved` })
  const reject = useAction(() => api.post(`/api/entities/${e.id}/reject`), { success: `${e.name} removed` })
  const merge = useAction(() => api.post(`/api/entities/${e.id}/merge`, { into: mergeInto }), { success: 'Merged' })
  return (
    <div className="flex flex-col gap-2 py-2 sm:flex-row sm:items-center">
      <div className="min-w-0 flex-1">
        <span className="font-medium">{e.name}</span>
        <span className="ml-2 text-xs text-muted-foreground">{e.entry_count} entries</span>
        {e.status === 'proposed' && <Badge variant="warning" className="ml-2">proposed</Badge>}
        {e.approved_by === 'ai' && <Badge variant="muted" className="ml-2">AI</Badge>}
      </div>
      {e.kind !== 'topic' && (
        <div className="flex flex-wrap items-center gap-1">
          {e.status === 'proposed' && <Button size="sm" variant="success" onClick={() => approve.mutate()} aria-label={`Approve ${e.name}`}><Check /></Button>}
          <NativeSelect className="h-8 w-36 text-xs" value={mergeInto} onChange={(ev) => setMergeInto(ev.target.value)} aria-label={`Merge ${e.name} into`}>
            <option value="">Merge into…</option>
            {entities.filter((o) => o.kind === e.kind && o.id !== e.id).map((o) => <option key={o.id} value={o.id}>{o.name}</option>)}
          </NativeSelect>
          {mergeInto && <Button size="sm" variant="outline" onClick={() => merge.mutate()}>Merge</Button>}
          <Button size="sm" variant="ghost" onClick={() => reject.mutate()} aria-label={`Remove ${e.name}`}><X /></Button>
        </div>
      )}
    </div>
  )
}

function EdgeItem({ edge, byId }: { edge: Edge; byId: Map<string, Entity> }) {
  const approve = useAction(() => api.post(`/api/edges/${edge.id}/approve`), { success: 'Relationship approved' })
  const reject = useAction(() => api.post(`/api/edges/${edge.id}/reject`), { success: 'Relationship removed' })
  const from = byId.get(edge.from_entity)
  const to = byId.get(edge.to_entity)
  if (!from || !to) return null
  return (
    <div className="flex items-center justify-between gap-2 py-2 text-sm">
      <div className="min-w-0">
        <span className="font-medium">{from.name}</span> <span className="text-muted-foreground">{edgeLabel[edge.kind]}</span> <span className="font-medium">{to.name}</span>
        {edge.status === 'proposed' && <Badge variant="warning" className="ml-2">needs you</Badge>}
      </div>
      <div className="flex shrink-0 gap-1">
        {edge.status === 'proposed' && <Button size="sm" variant="success" onClick={() => approve.mutate()} aria-label="Approve relationship"><Check /></Button>}
        <Button size="sm" variant="ghost" onClick={() => reject.mutate()} aria-label="Remove relationship"><X /></Button>
      </div>
    </div>
  )
}

export function GraphPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/graph' })
  const q = useQuery({ queryKey: ['graph', siteId], queryFn: () => api.get<{ entities: Entity[]; edges: Edge[] }>(`/api/sites/${siteId}/graph`) })
  if (q.isLoading) return <Spinner />
  const { entities = [], edges = [] } = q.data ?? {}
  const byId = new Map(entities.map((e) => [e.id, e]))
  const groups = Object.keys(kindLabel).map((k) => [k, entities.filter((e) => e.kind === k)] as const).filter(([, list]) => list.length)
  const pendingEdges = edges.filter((e) => e.status === 'proposed')
  return (
    <div>
      <PageHeader title="Knowledge graph" description="The things your entries are about and how they relate. Drafts find relevant entries by walking two steps out from a piece's topic or the competitor it names." />
      {entities.length === 0 ? (
        <EmptyState icon={<Network />} title="No entities yet">Entities are extracted as entries are approved.</EmptyState>
      ) : (
        <div className="grid gap-5 lg:grid-cols-2">
          <div className="flex flex-col gap-4">
            {groups.map(([kind, list]) => (
              <Card key={kind}>
                <CardHeader><CardTitle className="text-sm">{kindLabel[kind]}</CardTitle></CardHeader>
                <CardContent className="divide-y">{list.map((e) => <EntityItem key={e.id} e={e} entities={entities} />)}</CardContent>
              </Card>
            ))}
          </div>
          <div className="flex flex-col gap-4">
            {pendingEdges.length > 0 && (
              <Card className="border-warning/50">
                <CardHeader><CardTitle className="text-sm">Relationships waiting for you</CardTitle></CardHeader>
                <CardContent className="divide-y">{pendingEdges.map((e) => <EdgeItem key={e.id} edge={e} byId={byId} />)}</CardContent>
              </Card>
            )}
            <Card>
              <CardHeader><CardTitle className="text-sm">Relationships</CardTitle></CardHeader>
              <CardContent className="divide-y">{edges.filter((e) => e.status === 'approved').map((e) => <EdgeItem key={e.id} edge={e} byId={byId} />)}</CardContent>
            </Card>
          </div>
        </div>
      )}
    </div>
  )
}
