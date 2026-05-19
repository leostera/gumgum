#!/bin/sh
# GumGum.dev installer
# Usage:
#   curl -fsSL https://get.gumgum.dev | sh
#   curl -fsSL https://get.gumgum.dev | sh -s -- --version 2026-05-19
#   curl -fsSL https://get.gumgum.dev | sh -s -- --gumgum-dir "$HOME/.gumgum"

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info() { printf "${GREEN}==>${NC} %s\n" "$1"; }
warn() { printf "${YELLOW}Warning:${NC} %s\n" "$1"; }
err() { printf "${RED}Error:${NC} %s\n" "$1" >&2; }

usage() {
  cat <<'EOF'
GumGum.dev installer

Usage:
  curl -fsSL https://get.gumgum.dev | sh
  curl -fsSL https://get.gumgum.dev | sh -s -- -v 2026-05-19
  curl -fsSL https://get.gumgum.dev | sh -s -- --gumgum-dir "$HOME/.gumgum"

Options:
  -v, --version VERSION   Install a specific release date/path instead of latest.
  --gumgum-dir DIR        Install into DIR instead of $HOME/.gumgum.
  -h, --help              Show this help.

Environment:
  GUMGUM_VERSION          Install a specific release date/path.
  GUMGUM_DIR              Install into this directory instead of $HOME/.gumgum.
  GUMGUM_BASE_URL         Download base URL instead of https://get.gumgum.dev.
EOF
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -v|--version)
        [ "$#" -ge 2 ] || { err "$1 requires a version"; exit 1; }
        GUMGUM_VERSION="$2"; export GUMGUM_VERSION; shift 2 ;;
      --version=*) GUMGUM_VERSION="${1#--version=}"; export GUMGUM_VERSION; shift ;;
      --gumgum-dir)
        [ "$#" -ge 2 ] || { err "$1 requires a directory"; exit 1; }
        GUMGUM_DIR="$2"; export GUMGUM_DIR; shift 2 ;;
      --gumgum-dir=*) GUMGUM_DIR="${1#--gumgum-dir=}"; export GUMGUM_DIR; shift ;;
      -h|--help) usage; exit 0 ;;
      --) shift; break ;;
      *) err "Unknown option: $1"; echo; usage; exit 1 ;;
    esac
  done
  [ "$#" -eq 0 ] || { err "Unexpected argument: $1"; exit 1; }
}

detect_platform() {
  OS=$(uname -s)
  ARCH=$(uname -m)
  case "$OS" in
    Linux*) OS_TYPE=linux ;;
    *) err "Unsupported operating system: $OS. GumGum.dev currently publishes Linux binaries only."; exit 1 ;;
  esac
  case "$ARCH" in
    x86_64|amd64) ARCH_TYPE=x86_64 ;;
    *) err "Unsupported architecture: $ARCH. GumGum.dev currently publishes x86_64 Linux binaries only."; exit 1 ;;
  esac
  LIBC=gnu
  if ldd --version 2>&1 | grep -qi musl; then LIBC=musl; fi
  PLATFORM="${ARCH_TYPE}-unknown-${OS_TYPE}-${LIBC}"
  info "Detected platform: $PLATFORM"
}

install_gumgum() {
  GUMGUM_DIR="${GUMGUM_DIR:-$HOME/.gumgum}"
  GUMGUM_BIN_DIR="$GUMGUM_DIR/bin"
  GUMGUM_VERSION="${GUMGUM_VERSION:-latest}"
  GUMGUM_BASE_URL="${GUMGUM_BASE_URL:-https://get.gumgum.dev}"
  ARCHIVE="gumgum-${PLATFORM}.tar.gz"
  URL="${GUMGUM_BASE_URL}/gumgum/${GUMGUM_VERSION}/${ARCHIVE}"

  info "Installing gumgum ($GUMGUM_VERSION) into $GUMGUM_BIN_DIR"
  mkdir -p "$GUMGUM_BIN_DIR"
  TMPDIR=$(mktemp -d)
  trap 'rm -rf "$TMPDIR"' EXIT

  info "Downloading $URL"
  if command -v curl >/dev/null 2>&1; then
    HTTP_CODE=$(curl -fsSL -w "%{http_code}" -o "$TMPDIR/gumgum.tar.gz" "$URL" || true)
    [ "$HTTP_CODE" = "200" ] || { err "Download failed with HTTP status $HTTP_CODE"; exit 1; }
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$TMPDIR/gumgum.tar.gz" "$URL" || { err "Download failed"; exit 1; }
  else
    err "Neither curl nor wget found. Please install one of them."
    exit 1
  fi

  tar xzf "$TMPDIR/gumgum.tar.gz" -C "$TMPDIR"
  FOUND=$(find "$TMPDIR" -type f -name gumgum | head -n 1)
  [ -n "$FOUND" ] || { err "gumgum binary not found in archive"; exit 1; }
  install -m 0755 "$FOUND" "$GUMGUM_BIN_DIR/gumgum"
  info "Installed $GUMGUM_BIN_DIR/gumgum"
}

add_to_path() {
  case "$(basename "${SHELL:-sh}")" in
    bash) SHELL_CONFIG="$HOME/.bashrc" ;;
    zsh) SHELL_CONFIG="$HOME/.zshrc" ;;
    fish) SHELL_CONFIG="$HOME/.config/fish/config.fish"; mkdir -p "$(dirname "$SHELL_CONFIG")" ;;
    *) warn "Unknown shell; add $GUMGUM_BIN_DIR to PATH manually"; return ;;
  esac

  if [ -f "$SHELL_CONFIG" ] && grep -Fq "$GUMGUM_BIN_DIR" "$SHELL_CONFIG"; then
    info "PATH already configured in $SHELL_CONFIG"
    return
  fi

  if [ "$(basename "${SHELL:-sh}")" = fish ]; then
    printf '\n# GumGum.dev\nfish_add_path -g %s\n' "$GUMGUM_BIN_DIR" >> "$SHELL_CONFIG"
  else
    printf '\n# GumGum.dev\nexport PATH="%s:$PATH"\n' "$GUMGUM_BIN_DIR" >> "$SHELL_CONFIG"
  fi
  warn "Restart your shell or run: source $SHELL_CONFIG"
}

verify() {
  export PATH="$GUMGUM_BIN_DIR:$PATH"
  command -v gumgum >/dev/null 2>&1 || { err "gumgum not found after install"; exit 1; }
  info "GumGum.dev installed successfully"
  echo "  gumgum --help"
  echo "  gumgum setup <host> --root-domain <domain>"
}

main() {
  parse_args "$@"
  info "GumGum.dev installer"
  detect_platform
  install_gumgum
  add_to_path
  verify
}

main "$@"
