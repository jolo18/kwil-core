import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useRef } from 'react'
import { useToast } from '@/components/toast'
import { api, type Inbox, type Job, type Me, type Site } from './api'

export function useMe() {
  return useQuery({ queryKey: ['me'], queryFn: () => api.get<Me>('/api/me'), staleTime: 60_000 })
}

export function useSites() {
  return useQuery({ queryKey: ['sites'], queryFn: () => api.get<{ sites: Site[] }>('/api/sites').then((r) => r.sites) })
}

export function useSite(siteId: string) {
  return useQuery({ queryKey: ['site', siteId], queryFn: () => api.get<Site>(`/api/sites/${siteId}`) })
}

export function useInbox() {
  return useQuery({ queryKey: ['inbox'], queryFn: () => api.get<Inbox>('/api/inbox'), refetchInterval: 15_000 })
}

/**
 * Watches a site's background jobs. While any are running it polls quickly, and when they
 * finish it refreshes everything shown for that site.
 */
export function useJobWatcher(siteId: string | undefined) {
  const qc = useQueryClient()
  const q = useQuery({
    queryKey: ['jobs', siteId],
    enabled: !!siteId,
    queryFn: () => api.get<{ jobs: Job[]; active: number }>(`/api/sites/${siteId}/jobs`),
    refetchInterval: (query) => ((query.state.data?.active ?? 0) > 0 ? 1500 : 10_000),
  })
  const prev = useRef(0)
  const active = q.data?.active ?? 0
  useEffect(() => {
    if (prev.current > 0 && active === 0) {
      qc.invalidateQueries({ predicate: (query) => query.queryKey[0] !== 'jobs' })
    }
    prev.current = active
  }, [active, qc])
  return q
}

/** A mutation that shows a toast and refreshes the given query keys. */
export function useAction<TArgs = void, TResult = unknown>(
  fn: (args: TArgs) => Promise<TResult>,
  opts: { success?: string | ((r: TResult) => string); invalidate?: unknown[][]; onSuccess?: (r: TResult) => void } = {},
) {
  const qc = useQueryClient()
  const toast = useToast()
  return useMutation({
    mutationFn: fn,
    onSuccess: (r) => {
      if (opts.success) toast('success', typeof opts.success === 'function' ? opts.success(r) : opts.success)
      const keys = opts.invalidate ?? []
      if (keys.length === 0) qc.invalidateQueries()
      keys.forEach((k) => qc.invalidateQueries({ queryKey: k }))
      qc.invalidateQueries({ queryKey: ['inbox'] })
      qc.invalidateQueries({ queryKey: ['jobs'] })
      opts.onSuccess?.(r)
    },
    onError: (e: Error) => toast('error', e.message),
  })
}
