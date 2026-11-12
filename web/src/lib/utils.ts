import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatDate(value?: string | null, opts: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short' }) {
  if (!value) return ''
  const d = value.length === 10 ? new Date(`${value}T00:00:00`) : new Date(value)
  return d.toLocaleDateString(undefined, opts)
}

export function formatDateTime(value?: string | null) {
  if (!value) return ''
  return new Date(value).toLocaleString(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })
}

export function money(n?: number) {
  return `$${(n ?? 0).toFixed(2)}`
}

export const contentTypeLabels: Record<string, string> = {
  how_to: 'How-to',
  buyer_guide: 'Buyer guide',
  alternatives: 'Alternatives',
  comparison: 'Comparison',
  listicle: 'Listicle',
  explainer: 'Explainer',
  report: 'Report',
  opinion: 'Opinion',
  case_study: 'Case study',
}

export const entryKindLabels: Record<string, string> = {
  fact: 'Fact',
  feature: 'Feature',
  audience: 'Audience',
  positioning: 'Positioning',
  opinion: 'Opinion',
  story: 'Story',
  quote: 'Quote',
  competitor: 'Competitor',
  avoid: 'Avoid',
}

export const sourceLabels: Record<string, string> = {
  interview: 'Interview',
  crawl: 'Website',
  upload: 'Upload',
  transcript: 'Transcript',
  draft_edit: 'Your edits',
  published_post: 'Published post',
  manual: 'Added by hand',
  third_party: 'Third party',
}
