// Typed client for the Kwil API. Types mirror the Rust models in backend/src/models.rs.

export const API_URL = (import.meta.env.VITE_API_URL as string | undefined) ?? (import.meta.env.DEV ? '' : 'http://localhost:8080')

let tokenGetter: (() => Promise<string | null>) | null = null

/** Set by the auth provider: Clerk supplies a session token; the dev login supplies none. */
export function setTokenGetter(fn: (() => Promise<string | null>) | null) {
  tokenGetter = fn
}

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function headers(json: boolean): Promise<HeadersInit> {
  const h: Record<string, string> = {}
  if (json) h['content-type'] = 'application/json'
  const token = tokenGetter ? await tokenGetter() : null
  if (token) h.authorization = `Bearer ${token}`
  return h
}

export async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const isForm = body instanceof FormData
  const res = await fetch(`${API_URL}${path}`, {
    method,
    headers: await headers(body !== undefined && !isForm),
    body: body === undefined ? undefined : isForm ? body : JSON.stringify(body),
  })
  const text = await res.text()
  const data = text ? safeJson(text) : null
  if (!res.ok) {
    const message = (data && typeof data === 'object' && 'error' in data ? (data as { error: string }).error : null) ?? `Request failed (${res.status})`
    throw new ApiError(res.status, message)
  }
  return data as T
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}

export async function download(path: string, filename: string) {
  const res = await fetch(`${API_URL}${path}`, { headers: await headers(false) })
  if (!res.ok) throw new ApiError(res.status, 'Download failed')
  const blob = await res.blob()
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  URL.revokeObjectURL(url)
}

export const api = {
  get: <T>(path: string) => request<T>('GET', path),
  post: <T>(path: string, body: unknown = {}) => request<T>('POST', path, body),
  patch: <T>(path: string, body: unknown) => request<T>('PATCH', path, body),
  del: <T>(path: string) => request<T>('DELETE', path),
}

// ---------------------------------------------------------------- types

export interface Me {
  user_id: string
  email: string | null
  org_id: string
  org_name: string
  role: 'owner' | 'member'
  auth_mode: string
  ai_provider: string
}

export interface Branding {
  primary_color?: string
  font?: string
  logo_url?: string
  footer?: string
  nav?: { label: string; url: string }[]
}

export interface Site {
  id: string
  name: string
  slug: string
  product_description: string
  audience: string
  website_url: string | null
  cadence_per_week: number
  monthly_budget_usd: number
  drafting_paused: boolean
  model_draft: string
  model_mid: string
  model_cheap: string
  review_mode: 'person' | 'auto'
  strict_gates: boolean
  author_name: string
  branding: Branding
  github_repo: string | null
  github_branch: string
  github_folder: string
  github_token_set: boolean
  profile_md: string
  last_crawled_at: string | null
  blog_url: string
  created_at: string
  entry_counts?: Record<string, number>
  post_counts?: Record<string, number>
  month_spend_usd?: number
}

export interface Source {
  kind: string
  reference: string
  title: string
  quote: string
}

export interface Entry {
  id: string
  site_id: string
  num: number
  kind: string
  body: string
  status: 'proposed' | 'approved' | 'rejected' | 'superseded'
  approved_by: 'person' | 'ai' | 'rule' | null
  approved_at: string | null
  gate: 'person' | 'ai' | 'rule'
  gate_reason: string
  version: number
  created_at: string
  updated_at: string
  sources?: Source[]
  topics?: { id: string; title: string }[]
}

export interface EntryDetail extends Entry {
  versions: { id: string; version: number; kind: string; body: string; status: string; approved_by: string | null; changed_by: string; changed_at: string }[]
  links: { id: string; kind: string; status: string; note: string; other_num: number; other_body: string; direction: string }[]
  entities: { id: string; kind: string; name: string }[]
  replaces: { num: number; body: string } | null
}

export interface Question {
  id: string
  site_id: string
  piece_id: string | null
  kind: 'core' | 'followup' | 'coverage'
  topic: string
  prompt: string
  answer: string | null
  answered_at: string | null
}

export interface Topic {
  id: string
  parent_id: string | null
  title: string
  slug: string
  description: string
  status: 'proposed' | 'approved' | 'rejected'
  position: number
  pieces: number
  published: number
  entries: number
  gap: boolean
}

export interface Piece {
  id: string
  topic_id: string | null
  topic_title: string | null
  content_type: string
  title: string
  angle: string
  target_keyword: string | null
  timeliness: 'timely' | 'timeless'
  scheduled_for: string
  status: 'proposed' | 'approved' | 'rejected' | 'drafting' | 'held' | 'drafted' | 'failed'
  hold_reason: string
  post_id: string | null
  post_status: string | null
}

export interface Calendar {
  pieces: Piece[]
  cadence_per_week: number
  drafting_paused: boolean
  mix: { comparison: number; total: number; share: number; limit: number; warning: string | null }
  gaps: { id: string; title: string }[]
}

export interface Post {
  id: string
  site_id: string
  piece_id: string | null
  topic_id: string | null
  content_type: string
  title: string
  slug: string
  meta_description: string
  body_md: string
  ai_draft_md: string
  status: 'in_review' | 'scheduled' | 'published' | 'rejected'
  review_notes: string
  timeliness: 'timely' | 'timeless'
  author_name: string
  publish_at: string | null
  published_at: string | null
  last_updated_at: string
  github_committed_at: string | null
  created_at: string
}

export interface Finding {
  rule: string
  message: string
  block: number
  excerpt: string
}

export interface PostDetail {
  post: Post
  revisions: { id: string; title: string; body_md: string; author: 'ai' | 'person'; created_at: string }[]
  cited_entries: Entry[]
  findings: Finding[]
  citation_coverage: number
  piece: (Piece & { brief: { outline?: string[] } | null }) | null
  public_url: string
}

export interface Notification {
  id: string
  site_id: string | null
  kind: string
  message: string
  created_at: string
}

export interface InboxSite {
  site: { id: string; name: string; slug: string }
  count: number
  questions: Question[]
  entries: Entry[]
  entries_waiting_on_ai: number
  drafts: Post[]
  proposed_pieces: number
  proposed_topics: number
  conflicts: number
  proposed_graph: number
  held: (Piece & { scheduled_for: string })[]
  notifications: Notification[]
}

export interface Inbox {
  total: number
  sites: InboxSite[]
  notifications: Notification[]
}

export interface Job {
  id: string
  kind: string
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'dead'
  attempts: number
  last_error: string | null
  created_at: string
  updated_at: string
}

export interface Conflict {
  id: string
  kind: string
  note: string
  new: { id: string; num: number; body: string; status: string }
  existing: { id: string; num: number; body: string; status: string }
}

export interface Entity {
  id: string
  kind: string
  name: string
  status: string
  approved_by: string | null
  entry_count: number
}

export interface Edge {
  id: string
  from_entity: string
  to_entity: string
  kind: string
  status: string
  approved_by: string | null
}

export interface Domain {
  id: string
  hostname: string
  status: 'pending' | 'active' | 'failed'
  verification: Record<string, unknown>
  created_at: string
}

export interface FixedPage {
  id: string
  slug: string
  title: string
  body_md: string
  nav_position: number
}

export interface Upload {
  id: string
  filename: string
  size_bytes: number
  status: string
  error: string | null
  created_at: string
}

export interface Costs {
  month_spend_usd: number
  monthly_budget_usd: number
  drafting_paused: boolean
  org_month_spend_usd: number
  by_task: { task: string; model: string; calls: number; cost_usd: number }[]
}

export interface Digest {
  id: string
  period_start: string
  period_end: string
  items: { entries: { id: string; num: number; kind: string; body: string }[]; entities: { id: string; kind: string; name: string }[] }
}
