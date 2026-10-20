#!/usr/bin/env bash
# Publishes signed syntax packs to the quark-packs R2 bucket, served as
# $PACK_BASE_URL through its Cloudflare custom domain.
#
#   publish.sh PACKS_DIR
#
# PACKS_DIR holds <target>/index.json and the packs, as the Syntax packs
# workflow's sign job leaves them (`syntax-pack index --base-url`). Order:
#
# 1. `syntax-pack verify` every target's index: signed by $PACK_INDEX_KEY,
#    every pinned language, local files matching, immutable URLs. Nothing
#    is uploaded unless every target passes.
# 2. Upload each file to the key its URL names, never replacing an object
#    already there. URLs carry the file's SHA-256, so an existing object
#    should already be identical.
# 3. Download every file through the public URL and check its SHA-256.
# 4. Only then replace each <target>/index.json (a single PUT, so readers
#    see the old index or the new one) and check the public copies.
#
# Environment: SYNTAX_PACK (the tool binary), PACK_BASE_URL, PACK_INDEX_KEY,
# TARGETS (space-separated), PACK_R2_ENDPOINT, PACK_R2_BUCKET, and the
# bucket credentials in AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY (the
# `aws` CLI talks to R2's S3 API). Object keys are the URL path below the
# host, so PACK_BASE_URL https://host/v1 maps to keys under v1/.
# PACK_DRY_RUN=1 stops after step 1 and prints the URLs it would upload.
set -euo pipefail

packs=${1:?usage: publish.sh PACKS_DIR}
: "${SYNTAX_PACK:?}" "${PACK_BASE_URL:?}" "${PACK_INDEX_KEY:?}" "${TARGETS:?}"
base=${PACK_BASE_URL%/}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for target in $TARGETS; do
  "$SYNTAX_PACK" verify --out "$packs" --target "$target" \
    --key "$PACK_INDEX_KEY" --base-url "$base" >"$work/$target.tsv"
  echo "$target: $(wc -l <"$work/$target.tsv") files verified"
done

stage=$work/stage
for target in $TARGETS; do
  while IFS=$'\t' read -r file url _; do
    rel=${url#"$base"/}
    mkdir -p "$stage/$(dirname "$rel")"
    cp "$file" "$stage/$rel"
  done <"$work/$target.tsv"
done

if [ "${PACK_DRY_RUN:-}" = 1 ]; then
  for target in $TARGETS; do cut -f2 "$work/$target.tsv"; done
  exit 0
fi

require() { : "${PACK_R2_ENDPOINT:?}" "${PACK_R2_BUCKET:?}" "${AWS_ACCESS_KEY_ID:?}" "${AWS_SECRET_ACCESS_KEY:?}"; }
require
export AWS_DEFAULT_REGION=auto
# The URL path below the host is the key prefix: https://host/v1 -> v1.
prefix=$(printf '%s' "$base" | sed -E 's#^[a-z]+://[^/]+/?##')
s3() { aws s3api "$@" --bucket "$PACK_R2_BUCKET" --endpoint-url "$PACK_R2_ENDPOINT"; }

uploaded=0
while IFS= read -r -d '' file; do
  rel=${file#"$stage"/}
  key=${prefix:+$prefix/}$rel
  if s3 head-object --key "$key" >/dev/null 2>&1; then
    continue
  fi
  s3 put-object --key "$key" --body "$file" \
    --cache-control "public, max-age=31536000, immutable" \
    --content-type application/octet-stream >/dev/null
  uploaded=$((uploaded + 1))
done < <(find "$stage" -type f -print0)
echo "uploaded $uploaded new pack files"

sha_of_url() {
  curl -fsSL --retry 5 --retry-delay 5 --retry-all-errors "$1" | sha256sum | cut -d' ' -f1
}

for target in $TARGETS; do
  while IFS=$'\t' read -r _ url sha; do
    served=$(sha_of_url "$url")
    if [ "$served" != "$sha" ]; then
      echo "::error::$url serves SHA-256 $served, the index lists $sha"
      exit 1
    fi
  done <"$work/$target.tsv"
  echo "$target: every pack file is served"
done

for target in $TARGETS; do
  s3 put-object --key "${prefix:+$prefix/}$target/index.json" --body "$packs/$target/index.json" \
    --cache-control no-cache --content-type application/json >/dev/null
done

for target in $TARGETS; do
  expected=$(sha256sum <"$packs/$target/index.json" | cut -d' ' -f1)
  served=$(sha_of_url "$base/$target/index.json")
  if [ "$served" != "$expected" ]; then
    echo "::error::$base/$target/index.json is not the index just published"
    exit 1
  fi
  echo "$target: published $base/$target/index.json"
done
