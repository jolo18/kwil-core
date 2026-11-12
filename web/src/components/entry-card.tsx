import { Bot, Check, ShieldCheck, User, X } from 'lucide-react'
import { api, type Entry } from '@/lib/api'
import { useAction } from '@/lib/hooks'
import { entryKindLabels, sourceLabels } from '@/lib/utils'
import { Badge, Button } from './ui'

export function ApprovedBy({ by }: { by: Entry['approved_by'] }) {
  if (by === 'ai') return <Badge variant="muted" title="Approved by the AI verifier against a quoted passage"><Bot className="h-3 w-3" /> AI-verified</Badge>
  if (by === 'rule') return <Badge variant="muted"><ShieldCheck className="h-3 w-3" /> Rule</Badge>
  if (by === 'person') return <Badge variant="muted"><User className="h-3 w-3" /> You</Badge>
  return null
}

export function StatusBadge({ status }: { status: string }) {
  const variant = ({ approved: 'success', proposed: 'warning', rejected: 'destructive', superseded: 'muted' } as const)[status as 'approved'] ?? 'outline'
  return <Badge variant={variant}>{status}</Badge>
}

export function EntryCard({ entry, onOpen, showActions = true }: { entry: Entry; onOpen?: () => void; showActions?: boolean }) {
  const approve = useAction(() => api.post(`/api/entries/${entry.id}/approve`), { success: `E${entry.num} approved` })
  const reject = useAction(() => api.post(`/api/entries/${entry.id}/reject`), { success: `E${entry.num} rejected` })
  const source = entry.sources?.[0]
  return (
    <div className="flex flex-col gap-2 rounded-lg border bg-card p-3 sm:flex-row sm:items-start" data-testid="entry">
      <button className="flex min-w-0 flex-1 flex-col gap-1.5 text-left" onClick={onOpen} type="button">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="font-mono text-xs font-semibold text-primary">E{entry.num}</span>
          <Badge variant="outline">{entryKindLabels[entry.kind] ?? entry.kind}</Badge>
          {entry.status !== 'proposed' && <StatusBadge status={entry.status} />}
          {entry.status === 'approved' && <ApprovedBy by={entry.approved_by} />}
          {source && <span className="text-xs text-muted-foreground">from {sourceLabels[source.kind] ?? source.kind}{source.title && source.kind !== 'interview' ? `: ${source.title}` : ''}</span>}
        </div>
        <p className="text-sm">{entry.body}</p>
        {entry.status === 'proposed' && entry.gate === 'person' && entry.gate_reason && (
          <p className="text-xs text-muted-foreground">Needs you because {entry.gate_reason}.</p>
        )}
        {entry.status === 'proposed' && entry.gate === 'ai' && <p className="text-xs text-muted-foreground">Waiting for the AI verifier to check it against its source.</p>}
      </button>
      {showActions && entry.status === 'proposed' && (
        <div className="flex shrink-0 gap-2">
          <Button size="sm" variant="success" onClick={() => approve.mutate()} loading={approve.isPending} aria-label={`Approve E${entry.num}`}>
            <Check /> Approve
          </Button>
          <Button size="sm" variant="outline" onClick={() => reject.mutate()} loading={reject.isPending} aria-label={`Reject E${entry.num}`}>
            <X /> Reject
          </Button>
        </div>
      )}
    </div>
  )
}
