import { createRootRoute, createRoute, createRouter, Outlet } from '@tanstack/react-router'
import { AppLayout } from '@/components/layout'
import { CalendarPage } from '@/routes/calendar'
import { GraphPage } from '@/routes/graph'
import { InboxPage } from '@/routes/inbox'
import { InterviewPage } from '@/routes/interview'
import { LibraryPage } from '@/routes/library'
import { PostEditorPage, PostsPage } from '@/routes/posts'
import { SettingsPage } from '@/routes/settings'
import { NewSitePage } from '@/routes/site-new'
import { SiteOverviewPage } from '@/routes/site-overview'
import { SourcesPage } from '@/routes/sources'
import { TopicsPage } from '@/routes/topics'

const root = createRootRoute({ component: () => <Outlet /> })
const app = createRoute({ getParentRoute: () => root, id: 'app', component: AppLayout })

const inbox = createRoute({ getParentRoute: () => app, path: '/', component: InboxPage })
const newSite = createRoute({ getParentRoute: () => app, path: '/sites/new', component: NewSitePage })
const site = createRoute({ getParentRoute: () => app, path: '/sites/$siteId' })
const overview = createRoute({ getParentRoute: () => site, path: '/', component: SiteOverviewPage })
const interview = createRoute({ getParentRoute: () => site, path: '/interview', component: InterviewPage })
const library = createRoute({
  getParentRoute: () => site,
  path: '/library',
  component: LibraryPage,
  validateSearch: (s: Record<string, unknown>): { tab?: string } => ({ tab: typeof s.tab === 'string' ? s.tab : undefined }),
})
const topics = createRoute({ getParentRoute: () => site, path: '/topics', component: TopicsPage })
const calendar = createRoute({ getParentRoute: () => site, path: '/calendar', component: CalendarPage })
const posts = createRoute({ getParentRoute: () => site, path: '/posts' })
const postList = createRoute({ getParentRoute: () => posts, path: '/', component: PostsPage })
const postEditor = createRoute({ getParentRoute: () => posts, path: '/$postId', component: PostEditorPage })
const sources = createRoute({ getParentRoute: () => site, path: '/sources', component: SourcesPage })
const graph = createRoute({ getParentRoute: () => site, path: '/graph', component: GraphPage })
const settings = createRoute({ getParentRoute: () => site, path: '/settings', component: SettingsPage })

const routeTree = root.addChildren([
  app.addChildren([
    inbox,
    newSite,
    site.addChildren([overview, interview, library, topics, calendar, posts.addChildren([postList, postEditor]), sources, graph, settings]),
  ]),
])

export const router = createRouter({ routeTree, defaultPreload: 'intent' })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
