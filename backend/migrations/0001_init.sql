-- Tenancy -----------------------------------------------------------------

create table organizations (
    id uuid primary key,
    external_id text not null unique,
    name text not null,
    created_at timestamptz not null default now()
);

create table users (
    id uuid primary key,
    external_id text not null unique,
    email text,
    name text,
    created_at timestamptz not null default now()
);

create table memberships (
    org_id uuid not null references organizations(id) on delete cascade,
    user_id uuid not null references users(id) on delete cascade,
    role text not null check (role in ('owner', 'member')),
    primary key (org_id, user_id)
);

create table sites (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    name text not null,
    slug text not null unique,
    product_description text not null default '',
    audience text not null default '',
    website_url text,
    cadence_per_week int not null default 3 check (cadence_per_week between 1 and 7),
    monthly_budget_usd double precision not null default 20,
    drafting_paused boolean not null default false,
    model_draft text not null default 'anthropic/claude-opus-5.5',
    model_mid text not null default 'anthropic/claude-sonnet-5.5',
    model_cheap text not null default 'anthropic/claude-haiku-5.5',
    review_mode text not null default 'person' check (review_mode in ('person', 'auto')),
    strict_gates boolean not null default false,
    author_name text not null default '',
    branding jsonb not null default '{}'::jsonb,
    github_repo text,
    github_branch text not null default 'main',
    github_folder text not null default 'content/blog',
    github_token text,
    profile_md text not null default '',
    profile_updated_at timestamptz,
    last_crawled_at timestamptz,
    created_at timestamptz not null default now()
);
create index sites_org on sites(org_id);

-- Sources and evidence ------------------------------------------------------

create table sources (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    kind text not null check (kind in ('interview', 'crawl', 'upload', 'transcript', 'draft_edit', 'published_post', 'manual', 'third_party')),
    reference text not null default '',
    title text not null default '',
    content_hash text,
    created_at timestamptz not null default now()
);
create index sources_site on sources(site_id);

create table passages (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    source_id uuid not null references sources(id) on delete cascade,
    ordinal int not null,
    text text not null
);
create index passages_source on passages(source_id);

-- Knowledge library ---------------------------------------------------------

create table entries (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    num int not null,
    kind text not null check (kind in ('fact', 'feature', 'audience', 'positioning', 'opinion', 'story', 'quote', 'competitor', 'avoid')),
    body text not null,
    status text not null check (status in ('proposed', 'approved', 'rejected', 'superseded')),
    approved_by text check (approved_by in ('person', 'ai', 'rule')),
    approved_at timestamptz,
    gate text not null default 'person' check (gate in ('rule', 'ai', 'person')),
    gate_reason text not null default '',
    replaces_entry_id uuid references entries(id) on delete set null,
    version int not null default 1,
    search tsvector generated always as (to_tsvector('english', body)) stored,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    unique (site_id, num)
);
create index entries_site_status on entries(site_id, status);
create index entries_search on entries using gin(search);

create table entry_versions (
    id uuid primary key,
    entry_id uuid not null references entries(id) on delete cascade,
    version int not null,
    kind text not null,
    body text not null,
    status text not null,
    approved_by text,
    changed_by text not null,
    changed_at timestamptz not null default now(),
    unique (entry_id, version)
);

create table entry_sources (
    entry_id uuid not null references entries(id) on delete cascade,
    source_id uuid not null references sources(id) on delete cascade,
    passage_id uuid references passages(id) on delete set null,
    quote text not null default '',
    primary key (entry_id, source_id)
);

create table entry_links (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    from_entry uuid not null references entries(id) on delete cascade,
    to_entry uuid not null references entries(id) on delete cascade,
    kind text not null check (kind in ('refines', 'contradicts', 'replaces')),
    status text not null default 'open' check (status in ('open', 'resolved')),
    note text not null default '',
    created_at timestamptz not null default now(),
    unique (from_entry, to_entry, kind)
);

-- Topic map ------------------------------------------------------------------

create table topics (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    parent_id uuid references topics(id) on delete cascade,
    title text not null,
    slug text not null,
    description text not null default '',
    status text not null default 'proposed' check (status in ('proposed', 'approved', 'rejected')),
    position int not null default 0,
    created_at timestamptz not null default now(),
    unique (site_id, slug)
);

create table entry_topics (
    entry_id uuid not null references entries(id) on delete cascade,
    topic_id uuid not null references topics(id) on delete cascade,
    primary key (entry_id, topic_id)
);

-- Knowledge graph ------------------------------------------------------------

create table entities (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    kind text not null check (kind in ('product', 'feature', 'persona', 'use_case', 'competitor', 'integration', 'topic')),
    name text not null,
    topic_id uuid references topics(id) on delete cascade,
    status text not null default 'proposed' check (status in ('proposed', 'approved', 'rejected')),
    approved_by text check (approved_by in ('person', 'ai', 'rule')),
    created_at timestamptz not null default now()
);
create unique index entities_unique_name on entities(site_id, kind, lower(name)) where status <> 'rejected';

create table entity_edges (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    from_entity uuid not null references entities(id) on delete cascade,
    to_entity uuid not null references entities(id) on delete cascade,
    kind text not null check (kind in ('has_feature', 'serves', 'competes_with', 'alternative_to', 'integrates_with', 'part_of_topic')),
    status text not null default 'proposed' check (status in ('proposed', 'approved', 'rejected')),
    approved_by text check (approved_by in ('person', 'ai', 'rule')),
    created_at timestamptz not null default now(),
    unique (from_entity, to_entity, kind)
);

create table entry_entities (
    entry_id uuid not null references entries(id) on delete cascade,
    entity_id uuid not null references entities(id) on delete cascade,
    primary key (entry_id, entity_id)
);

-- Questions for the person (interview and coverage) ---------------------------

create table pieces (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    topic_id uuid references topics(id) on delete set null,
    content_type text not null check (content_type in ('how_to', 'buyer_guide', 'alternatives', 'comparison', 'listicle', 'explainer', 'report', 'opinion', 'case_study')),
    title text not null,
    angle text not null default '',
    target_keyword text,
    timeliness text not null default 'timeless' check (timeliness in ('timely', 'timeless')),
    scheduled_for date not null,
    status text not null default 'proposed' check (status in ('proposed', 'approved', 'rejected', 'drafting', 'held', 'drafted', 'failed')),
    hold_reason text not null default '',
    brief jsonb,
    created_at timestamptz not null default now()
);
create index pieces_site_date on pieces(site_id, scheduled_for);

create table questions (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    piece_id uuid references pieces(id) on delete cascade,
    kind text not null check (kind in ('core', 'followup', 'coverage')),
    topic text not null default '',
    prompt text not null,
    answer text,
    answered_at timestamptz,
    position int not null default 0,
    created_at timestamptz not null default now()
);
create index questions_site on questions(site_id, answered_at);

-- Posts ------------------------------------------------------------------------

create table posts (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    piece_id uuid references pieces(id) on delete set null,
    topic_id uuid references topics(id) on delete set null,
    content_type text not null default 'explainer',
    title text not null,
    slug text not null,
    meta_description text not null default '',
    body_md text not null,
    ai_draft_md text not null default '',
    status text not null default 'in_review' check (status in ('in_review', 'scheduled', 'published', 'rejected')),
    review_notes text not null default '',
    timeliness text not null default 'timeless' check (timeliness in ('timely', 'timeless')),
    author_name text not null default '',
    publish_at timestamptz,
    published_at timestamptz,
    last_updated_at timestamptz not null default now(),
    github_committed_at timestamptz,
    created_at timestamptz not null default now(),
    unique (site_id, slug)
);
create index posts_site_status on posts(site_id, status);

create table post_revisions (
    id uuid primary key,
    post_id uuid not null references posts(id) on delete cascade,
    title text not null,
    body_md text not null,
    author text not null check (author in ('ai', 'person')),
    created_at timestamptz not null default now()
);

create table fixed_pages (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    slug text not null,
    title text not null,
    body_md text not null default '',
    nav_position int not null default 0,
    created_at timestamptz not null default now(),
    unique (site_id, slug)
);

-- Jobs and costs -----------------------------------------------------------------

create table jobs (
    id uuid primary key,
    org_id uuid references organizations(id) on delete cascade,
    site_id uuid references sites(id) on delete cascade,
    kind text not null,
    payload jsonb not null default '{}'::jsonb,
    status text not null default 'queued' check (status in ('queued', 'running', 'succeeded', 'failed', 'dead')),
    attempts int not null default 0,
    max_attempts int not null default 3,
    run_after timestamptz not null default now(),
    last_error text,
    result jsonb,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);
create index jobs_ready on jobs(status, run_after);
create index jobs_site on jobs(site_id, created_at desc);

create table ai_calls (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid references sites(id) on delete cascade,
    task text not null,
    tier text not null,
    model text not null,
    input_tokens int not null default 0,
    output_tokens int not null default 0,
    cost_usd double precision not null default 0,
    created_at timestamptz not null default now()
);
create index ai_calls_site_time on ai_calls(site_id, created_at);

create table notifications (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid references sites(id) on delete cascade,
    kind text not null,
    message text not null,
    read_at timestamptz,
    created_at timestamptz not null default now()
);

-- Hosting ------------------------------------------------------------------------

create table domains (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    hostname text not null unique,
    status text not null default 'pending' check (status in ('pending', 'active', 'failed')),
    provider_id text,
    verification jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now()
);

-- Uploads, crawl state, digests, email ------------------------------------------------

create table uploads (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    filename text not null,
    content_type text not null default '',
    storage_key text not null,
    size_bytes bigint not null default 0,
    status text not null default 'queued' check (status in ('queued', 'processed', 'failed')),
    error text,
    created_at timestamptz not null default now()
);

create table crawl_pages (
    site_id uuid not null references sites(id) on delete cascade,
    url text not null,
    content_hash text not null,
    fetched_at timestamptz not null default now(),
    primary key (site_id, url)
);

create table digests (
    id uuid primary key,
    org_id uuid not null references organizations(id) on delete cascade,
    site_id uuid not null references sites(id) on delete cascade,
    period_start timestamptz not null,
    period_end timestamptz not null,
    items jsonb not null,
    created_at timestamptz not null default now()
);

create table email_outbox (
    id uuid primary key,
    org_id uuid references organizations(id) on delete cascade,
    to_address text not null,
    subject text not null,
    body text not null,
    sent_at timestamptz,
    error text,
    created_at timestamptz not null default now()
);
