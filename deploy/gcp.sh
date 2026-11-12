#!/usr/bin/env bash
# Deploys Kwil to Google Cloud. Untested against a real project: read each step before running.
#
# Creates: Artifact Registry repo, Cloud SQL Postgres, a Cloud Storage bucket, secrets,
# three Cloud Run services (api, worker, blog), a Cloud Tasks queue and a Cloud Scheduler job.
# The web app is deployed separately to Firebase Hosting (see the end of this file).
#
# Prerequisites: gcloud logged in, billing enabled, and these exported:
#   PROJECT_ID, REGION (e.g. europe-west1), BLOG_BASE_DOMAIN (e.g. kwil.blog), APP_URL (the web app's URL)
#   OPENROUTER_API_KEY, CLERK_JWKS_URL, CLERK_ISSUER, CLERK_WEBHOOK_SECRET
# Optional: CLOUDFLARE_API_TOKEN, CLOUDFLARE_ZONE_ID, CLOUDFLARE_CNAME_TARGET, RESEND_API_KEY, MAIL_FROM
set -euo pipefail

: "${PROJECT_ID:?}" "${REGION:?}" "${BLOG_BASE_DOMAIN:?}" "${APP_URL:?}"
: "${OPENROUTER_API_KEY:?}" "${CLERK_JWKS_URL:?}" "${CLERK_ISSUER:?}" "${CLERK_WEBHOOK_SECRET:?}"

INSTANCE=kwil-db
BUCKET="${PROJECT_ID}-kwil-uploads"
QUEUE=kwil-jobs
REPO=kwil
IMAGE="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO}/kwil:$(git rev-parse --short HEAD)"
SA_NAME=kwil-runtime
SA="${SA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"

gcloud config set project "$PROJECT_ID"
gcloud services enable run.googleapis.com sqladmin.googleapis.com cloudtasks.googleapis.com \
  cloudscheduler.googleapis.com secretmanager.googleapis.com artifactregistry.googleapis.com \
  cloudbuild.googleapis.com storage.googleapis.com

# Service account the three services run as.
gcloud iam service-accounts describe "$SA" >/dev/null 2>&1 || gcloud iam service-accounts create "$SA_NAME"
for role in roles/cloudsql.client roles/cloudtasks.enqueuer roles/secretmanager.secretAccessor roles/storage.objectAdmin roles/iam.serviceAccountUser; do
  gcloud projects add-iam-policy-binding "$PROJECT_ID" --member "serviceAccount:$SA" --role "$role" --condition=None >/dev/null
done

# Image.
gcloud artifacts repositories describe "$REPO" --location "$REGION" >/dev/null 2>&1 || \
  gcloud artifacts repositories create "$REPO" --repository-format docker --location "$REGION"
gcloud builds submit backend --tag "$IMAGE"

# Database.
if ! gcloud sql instances describe "$INSTANCE" >/dev/null 2>&1; then
  gcloud sql instances create "$INSTANCE" --database-version POSTGRES_16 --region "$REGION" --tier db-f1-micro
  gcloud sql databases create kwil --instance "$INSTANCE"
fi
DB_PASSWORD_SECRET=kwil-db-password
if ! gcloud secrets describe "$DB_PASSWORD_SECRET" >/dev/null 2>&1; then
  DB_PASSWORD="$(openssl rand -hex 24)"
  gcloud sql users create kwil --instance "$INSTANCE" --password "$DB_PASSWORD"
  printf '%s' "$DB_PASSWORD" | gcloud secrets create "$DB_PASSWORD_SECRET" --data-file=-
fi
DB_PASSWORD="$(gcloud secrets versions access latest --secret "$DB_PASSWORD_SECRET")"
CONNECTION="${PROJECT_ID}:${REGION}:${INSTANCE}"
DATABASE_URL="postgres://kwil:${DB_PASSWORD}@localhost/kwil?host=/cloudsql/${CONNECTION}"

# Uploads bucket and job queue.
gcloud storage buckets describe "gs://$BUCKET" >/dev/null 2>&1 || gcloud storage buckets create "gs://$BUCKET" --location "$REGION"
gcloud tasks queues describe "$QUEUE" --location "$REGION" >/dev/null 2>&1 || \
  gcloud tasks queues create "$QUEUE" --location "$REGION" --max-attempts 1

# Secrets.
put_secret() {
  local name=$1 value=$2
  [ -z "$value" ] && return 0
  if gcloud secrets describe "$name" >/dev/null 2>&1; then
    printf '%s' "$value" | gcloud secrets versions add "$name" --data-file=- >/dev/null
  else
    printf '%s' "$value" | gcloud secrets create "$name" --data-file=- >/dev/null
  fi
}
gcloud secrets describe kwil-internal-secret >/dev/null 2>&1 || put_secret kwil-internal-secret "$(openssl rand -hex 32)"
put_secret kwil-database-url "$DATABASE_URL"
put_secret kwil-openrouter-key "$OPENROUTER_API_KEY"
put_secret kwil-clerk-webhook-secret "$CLERK_WEBHOOK_SECRET"
put_secret kwil-cloudflare-token "${CLOUDFLARE_API_TOKEN:-}"
put_secret kwil-resend-key "${RESEND_API_KEY:-}"
INTERNAL_SECRET="$(gcloud secrets versions access latest --secret kwil-internal-secret)"

SECRETS="DATABASE_URL=kwil-database-url:latest,OPENROUTER_API_KEY=kwil-openrouter-key:latest,INTERNAL_SECRET=kwil-internal-secret:latest,CLERK_WEBHOOK_SECRET=kwil-clerk-webhook-secret:latest"
[ -n "${CLOUDFLARE_API_TOKEN:-}" ] && SECRETS="$SECRETS,CLOUDFLARE_API_TOKEN=kwil-cloudflare-token:latest"
[ -n "${RESEND_API_KEY:-}" ] && SECRETS="$SECRETS,RESEND_API_KEY=kwil-resend-key:latest"

COMMON_ENV="APP_ENV=production,AUTH_MODE=clerk,CLERK_JWKS_URL=${CLERK_JWKS_URL},CLERK_ISSUER=${CLERK_ISSUER},AI_PROVIDER=openrouter"
COMMON_ENV="$COMMON_ENV,STORAGE=gcs,GCS_BUCKET=${BUCKET},BLOG_BASE_DOMAIN=${BLOG_BASE_DOMAIN},APP_URL=${APP_URL},CORS_ORIGIN=${APP_URL}"
COMMON_ENV="$COMMON_ENV,CLOUD_TASKS_SERVICE_ACCOUNT=${SA},QUEUE=cloudtasks,CLOUD_TASKS_QUEUE=projects/${PROJECT_ID}/locations/${REGION}/queues/${QUEUE}"
if [ -n "${CLOUDFLARE_API_TOKEN:-}" ]; then
  COMMON_ENV="$COMMON_ENV,CDN=cloudflare,CLOUDFLARE_ZONE_ID=${CLOUDFLARE_ZONE_ID:?},CLOUDFLARE_CNAME_TARGET=${CLOUDFLARE_CNAME_TARGET:?}"
fi
if [ -n "${RESEND_API_KEY:-}" ]; then
  COMMON_ENV="$COMMON_ENV,MAILER=resend,MAIL_FROM=${MAIL_FROM:?}"
fi

deploy() {
  local name=$1 role=$2; shift 2
  gcloud run deploy "$name" --image "$IMAGE" --region "$REGION" --service-account "$SA" \
    --add-cloudsql-instances "$CONNECTION" --set-secrets "$SECRETS" --args "$role" "$@"
}

# The worker first, so the API knows where Cloud Tasks should send jobs.
deploy kwil-worker worker --no-allow-unauthenticated --set-env-vars "$COMMON_ENV,WORKER_URL=pending" --timeout 900 --concurrency 4
WORKER_URL="$(gcloud run services describe kwil-worker --region "$REGION" --format 'value(status.url)')"
ENV_WITH_WORKER="$COMMON_ENV,WORKER_URL=${WORKER_URL}"
deploy kwil-worker worker --no-allow-unauthenticated --set-env-vars "$ENV_WITH_WORKER" --timeout 900 --concurrency 4
deploy kwil-api api --allow-unauthenticated --set-env-vars "$ENV_WITH_WORKER"
deploy kwil-blog blog --allow-unauthenticated --set-env-vars "$ENV_WITH_WORKER,BLOG_PORT=8080" --memory 512Mi
API_URL="$(gcloud run services describe kwil-api --region "$REGION" --format 'value(status.url)')"

# Cloud Tasks calls the private worker with an OIDC token for the runtime service account.
gcloud run services add-iam-policy-binding kwil-worker --region "$REGION" --member "serviceAccount:$SA" --role roles/run.invoker >/dev/null

# The scheduled tick: creates due drafting, publishing, calendar, re-crawl, digest and domain work.
if gcloud scheduler jobs describe kwil-tick --location "$REGION" >/dev/null 2>&1; then
  gcloud scheduler jobs update http kwil-tick --location "$REGION" --schedule "*/5 * * * *" \
    --uri "${API_URL}/internal/cron/tick" --http-method POST --update-headers "x-internal-secret=${INTERNAL_SECRET}"
else
  gcloud scheduler jobs create http kwil-tick --location "$REGION" --schedule "*/5 * * * *" \
    --uri "${API_URL}/internal/cron/tick" --http-method POST --headers "x-internal-secret=${INTERNAL_SECRET}"
fi

cat <<EOF

Deployed.
  API:    ${API_URL}
  Worker: ${WORKER_URL} (private)
  Blog:   $(gcloud run services describe kwil-blog --region "$REGION" --format 'value(status.url)')

Next:
  1. Point *.${BLOG_BASE_DOMAIN} at the blog service (for example through Cloudflare, which
     also fronts custom domains: set its fallback origin to the blog service).
  2. In Clerk, add a webhook to ${API_URL}/webhooks/clerk for organization, membership and user events.
  3. Deploy the web app:
       cd web && VITE_API_URL=${API_URL} VITE_CLERK_PUBLISHABLE_KEY=pk_live_... npm run build
       npx firebase-tools deploy --only hosting --project ${PROJECT_ID}
EOF
