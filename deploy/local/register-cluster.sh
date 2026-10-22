#!/usr/bin/env bash
# Register the local StarRocks cluster in Stellar and make it active.
#
#   ./register-cluster.sh [stellar-url]
#
# Uses the read-only monitoring account created on the StarRocks side
# (see ../../../scripts/permissions/).
set -euo pipefail

STELLAR_URL="${1:-http://localhost:8080}"
SR_HOST="${SR_HOST:-starrocks-fe}"
SR_QUERY_PORT="${SR_QUERY_PORT:-9030}"
SR_HTTP_PORT="${SR_HTTP_PORT:-8030}"
SR_USER="${SR_USER:-starrocks_monitor}"
SR_CLUSTER_NAME="${SR_CLUSTER_NAME:-local-starrocks}"
ADMIN_USER="${ADMIN_USER:-admin}"

# Credentials are read from the environment so nothing secret is stored here.
: "${SR_PASSWORD:?set SR_PASSWORD to the StarRocks account password}"
: "${ADMIN_PASSWORD:?set ADMIN_PASSWORD to the Stellar admin password}"

log() { printf '\n== %s\n' "$*"; }

log "logging in to $STELLAR_URL"
TOKEN=$(curl -sS -X POST "$STELLAR_URL/api/auth/login" \
  -H 'Content-Type: application/json' \
  -d "$(jq -n --arg u "$ADMIN_USER" --arg p "$ADMIN_PASSWORD" '{username:$u, password:$p}')" | jq -r '.token // empty')

if [ -z "$TOKEN" ]; then
  echo "login failed" >&2
  exit 1
fi
AUTH=(-H "Authorization: Bearer $TOKEN")

log "testing the connection to $SR_HOST:$SR_QUERY_PORT"
curl -sS -X POST "$STELLAR_URL/api/clusters/health/test" "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d "$(jq -n --arg h "$SR_HOST" --arg u "$SR_USER" --arg p "$SR_PASSWORD" \
        --argjson hp "$SR_HTTP_PORT" --argjson qp "$SR_QUERY_PORT" \
        '{fe_host:$h, fe_http_port:$hp, fe_query_port:$qp, username:$u, password:$p,
          cluster_type:"starrocks", deployment_mode:"shared_nothing"}')" | jq .

log "registering cluster $SR_CLUSTER_NAME"
CLUSTER_ID=$(curl -sS -X POST "$STELLAR_URL/api/clusters" "${AUTH[@]}" \
  -H 'Content-Type: application/json' \
  -d "$(jq -n --arg n "$SR_CLUSTER_NAME" --arg h "$SR_HOST" --arg u "$SR_USER" \
        --arg p "$SR_PASSWORD" --argjson hp "$SR_HTTP_PORT" --argjson qp "$SR_QUERY_PORT" \
        '{name:$n, description:"StarRocks 4.0.0 (dbx connection, local docker)",
          fe_host:$h, fe_http_port:$hp, fe_query_port:$qp, username:$u, password:$p,
          enable_ssl:false, connection_timeout:10, tags:["local","dev"],
          catalog:"default_catalog", deployment_mode:"shared_nothing",
          cluster_type:"starrocks"}')" | jq -r '.id // empty')

if [ -z "$CLUSTER_ID" ]; then
  echo "cluster registration failed" >&2
  exit 1
fi
echo "cluster id: $CLUSTER_ID"

log "activating cluster $CLUSTER_ID"
curl -sS -X PUT "$STELLAR_URL/api/clusters/$CLUSTER_ID/activate" "${AUTH[@]}" | jq .

log "cluster health"
curl -sS "$STELLAR_URL/api/clusters/$CLUSTER_ID/health" "${AUTH[@]}" | jq .
