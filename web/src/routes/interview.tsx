import { Link, useParams } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { QuestionForm } from '@/components/question-form'
import { Button, Card, CardContent, EmptyState, PageHeader, Spinner } from '@/components/ui'
import { api, type Question } from '@/lib/api'

export function InterviewPage() {
  const { siteId } = useParams({ from: '/app/sites/$siteId/interview' })
  const q = useQuery({ queryKey: ['questions', siteId], queryFn: () => api.get<{ questions: Question[] }>(`/api/sites/${siteId}/questions`) })
  if (q.isLoading) return <Spinner />
  const all = q.data?.questions ?? []
  const open = all.filter((x) => !x.answer)
  const answered = all.filter((x) => x.answer)
  return (
    <div className="max-w-3xl">
      <PageHeader
        title="Interview"
        description="Your answers become library entries. Kwil only writes from what you approve, so the more specific you are, the more the posts sound like you."
      />
      {open.length === 0 ? (
        <EmptyState title="All questions answered" action={<Button asChild><Link to="/sites/$siteId/library" params={{ siteId }} search={{ tab: 'proposed' }}>Review proposed entries</Link></Button>}>
          New questions appear here when Kwil needs more from you for a planned post.
        </EmptyState>
      ) : (
        <div className="flex flex-col gap-3">
          <p className="text-sm text-muted-foreground">{open.length} open question{open.length === 1 ? '' : 's'}</p>
          {open.map((x) => <QuestionForm key={x.id} question={x} />)}
        </div>
      )}
      {answered.length > 0 && (
        <div className="mt-10">
          <h2 className="mb-3 text-sm font-medium text-muted-foreground">Answered</h2>
          <div className="flex flex-col gap-2">
            {answered.map((x) => (
              <Card key={x.id}>
                <CardContent className="pt-4 text-sm">
                  <div className="font-medium">{x.prompt}</div>
                  <p className="mt-1 whitespace-pre-wrap text-muted-foreground">{x.answer}</p>
                </CardContent>
              </Card>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
