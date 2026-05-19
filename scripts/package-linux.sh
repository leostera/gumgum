#!/usr/bin/env bash
set -euo pipefail

TARGET=${TARGET:-x86_64-unknown-linux-gnu}
VERSION=${VERSION:-$(git rev-parse --short HEAD 2>/dev/null || date -u +%Y%m%d%H%M%S)}
DIST_ROOT=${DIST_ROOT:-dist}
CRATE=${CRATE:-gumgum-cli}
BIN=${BIN:-gumgum}
DOCKER_IMAGE=${DOCKER_IMAGE:-rust:1-bookworm}

if [ "$TARGET" != "x86_64-unknown-linux-gnu" ]; then
  echo "error: only x86_64-unknown-linux-gnu is wired for packaging right now" >&2
  exit 1
fi

command -v docker >/dev/null 2>&1 || {
  echo "error: docker is required for Linux x86_64 packaging from macOS" >&2
  exit 1
}

echo "→ building $BIN for $TARGET in $DOCKER_IMAGE"
GIT_SHA=${GIT_SHA:-$(git rev-parse --short HEAD 2>/dev/null || printf unknown)}

docker run --rm --platform linux/amd64 \
  -e GUMGUM_BUILD_VERSION="$VERSION" \
  -e GUMGUM_BUILD_SHA="$GIT_SHA" \
  -e GUMGUM_BUILD_TARGET="$TARGET" \
  -v "$PWD:/work" \
  -v "gumgum-cargo-registry:/usr/local/cargo/registry" \
  -w /work \
  "$DOCKER_IMAGE" \
  cargo build --release --target "$TARGET" -p "$CRATE"

out_dir="$DIST_ROOT/gumgum/$VERSION"
work_dir="$DIST_ROOT/work/$BIN-$TARGET"
archive="$out_dir/$BIN-$VERSION-$TARGET.tar.gz"

rm -rf "$work_dir"
mkdir -p "$work_dir" "$out_dir"
cp "target/$TARGET/release/$BIN" "$work_dir/$BIN"
chmod 0755 "$work_dir/$BIN"

cat > "$work_dir/README.txt" <<EOF
GumGum.dev $BIN
Target: $TARGET
Version: $VERSION
Install: install -m 0755 $BIN ~/.gumgum/bin/gumgum
Daemon: ~/.gumgum/bin/gumgum daemon
EOF

COPYFILE_DISABLE=1 tar --no-xattrs -C "$work_dir" -czf "$archive" .
cat > "$out_dir/release.json" <<EOF
{"version":"$VERSION","git_sha":"$GIT_SHA","target":"$TARGET","archive":"$BIN-$VERSION-$TARGET.tar.gz"}
EOF
mkdir -p "$DIST_ROOT"
cp sites/get.gumgum.dev/public/install.sh "$DIST_ROOT/install.sh"
chmod 0755 "$DIST_ROOT/install.sh"

echo "METRIC package_ok=1"
echo "→ wrote $archive"
echo "→ wrote $DIST_ROOT/install.sh"
