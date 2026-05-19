#!/usr/bin/env bash
set -euo pipefail

VERSION=${VERSION:-$(git rev-parse --short HEAD 2>/dev/null || date -u +%Y%m%d%H%M%S)}
DIST_ROOT=${DIST_ROOT:-dist}

if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
fi

BUCKET=${R2_BUCKET:?R2_BUCKET is required}
ENDPOINT=${R2_ENDPOINT:?R2_ENDPOINT is required}
export AWS_ACCESS_KEY_ID=${R2_ACCESS_KEY_ID:?R2_ACCESS_KEY_ID is required}
export AWS_SECRET_ACCESS_KEY=${R2_SECRET_ACCESS_KEY:?R2_SECRET_ACCESS_KEY is required}

upload() {
  local src=$1
  local key=$2
  local content_type=$3
  echo "→ uploading $src to r2://$BUCKET/$key"
  aws s3 cp "$src" "s3://$BUCKET/$key" \
    --endpoint-url "$ENDPOINT" \
    --content-type "$content_type"
}

upload "$DIST_ROOT/install.sh" "install.sh" "text/x-shellscript; charset=utf-8"
upload "$DIST_ROOT/gumgum/$VERSION/release.json" "gumgum/$VERSION/release.json" "application/json; charset=utf-8"
upload "$DIST_ROOT/gumgum/$VERSION/release.json" "gumgum/latest/release.json" "application/json; charset=utf-8"
for archive in "$DIST_ROOT/gumgum/$VERSION"/*.tar.gz; do
  upload "$archive" "gumgum/$VERSION/$(basename "$archive")" "application/gzip"
done

echo "METRIC upload_ok=1"
