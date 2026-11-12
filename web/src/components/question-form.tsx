import { useState } from 'react'
import { api, type Question } from '@/lib/api'
import { useAction } from '@/lib/hooks'
import { Badge, Button, Textarea } from './ui'

const topicLabels: Record<string, string> = {
  product: 'Product',
  audience: 'Audience',
  pain_points: 'Pain points',
  positioning: 'Positioning',
  opinions: 'Opinions',
  competitors: 'Competitors',
  stories: 'Stories',
  avoid: 'Avoid',
}

export function QuestionForm({ question }: { question: Question }) {
  const [answer, setAnswer] = useState('')
  const submit = useAction(() => api.post(`/api/questions/${question.id}/answer`, { answer }), {
    success: 'Answer saved. Proposed entries will appear shortly.',
    onSuccess: () => setAnswer(''),
  })
  return (
    <form
      className="flex flex-col gap-2 rounded-lg border bg-card p-3"
      data-testid="question"
      onSubmit={(e) => {
        e.preventDefault()
        if (answer.trim()) submit.mutate()
      }}
    >
      <div className="flex flex-wrap items-center gap-2">
        {question.topic && <Badge variant="outline">{topicLabels[question.topic] ?? question.topic}</Badge>}
        {question.kind === 'followup' && <Badge variant="muted">Follow-up</Badge>}
        {question.kind === 'coverage' && <Badge variant="warning">Needed for a planned post</Badge>}
      </div>
      <label className="text-sm font-medium" htmlFor={`q-${question.id}`}>{question.prompt}</label>
      <Textarea id={`q-${question.id}`} value={answer} onChange={(e) => setAnswer(e.target.value)} placeholder="Answer in your own words. Specifics beat polish." rows={3} />
      <div className="flex justify-end">
        <Button type="submit" size="sm" disabled={!answer.trim()} loading={submit.isPending}>Save answer</Button>
      </div>
    </form>
  )
}
