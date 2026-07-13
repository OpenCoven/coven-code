#!/usr/bin/env bash
# Coven Code installer for Linux and macOS.
# Upstream: Claurst installer by Kuber Mehta (GPL-3.0), rebranded for OpenCoven/coven-code.
#
# Quick install:
#   curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash
#
# Offline / pinned:
#   curl -fsSL -O https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh
#   chmod +x install.sh
#   ./install.sh --version 0.1.4

set -euo pipefail

APP=coven-code
ALIAS=coven-cave
REPO=OpenCoven/coven-code

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; MUTED='\033[0;90m'; NC='\033[0m'; BOLD='\033[1m'

print_message() {
  local type="$1"; local msg="$2"
  case "$type" in
    info)    echo -e "${BLUE}ℹ${NC}  $msg" ;;
    success) echo -e "${GREEN}✓${NC}  $msg" ;;
    warning) echo -e "${YELLOW}⚠${NC}  $msg" ;;
    error)   echo -e "${RED}✗${NC}  $msg" ;;
    *)       echo "   $msg" ;;
  esac
}

usage() {
cat <<EOF

${BOLD}Coven Code installer${NC}

USAGE:
    install.sh [OPTIONS]

OPTIONS:
    --version <ver>     Install a specific version (default: latest)
    --install-dir <dir> Override install location (default: ~/.coven-code/bin)
    --binary <path>     Use a local binary instead of downloading
    -h, --help          Show this help

EXAMPLES:
    curl -fsSL https://github.com/OpenCoven/coven-code/releases/latest/download/install.sh | bash
    ./install.sh --version 0.1.4
    ./install.sh --binary /path/to/coven-code
EOF
}

# Parse args
specific_version=""; install_dir_override=""; local_binary=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) specific_version="$2"; shift 2 ;;
    --install-dir) install_dir_override="$2"; shift 2 ;;
    --binary) local_binary="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) print_message error "Unknown option: $1"; usage; exit 1 ;;
  esac
done

print_message info "Tip: the unified Coven CLI installs and manages this engine for you — 'npm install -g @opencoven/cli', then run 'coven'. This standalone installer still works."

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$OS" in
  linux)  PLATFORM="linux" ;;
  darwin) PLATFORM="macos" ;;
  *)      print_message error "Unsupported OS: $OS"; exit 1 ;;
esac
case "$ARCH" in
  x86_64|amd64) ARCH_TAG="x86_64" ;;
  aarch64|arm64) ARCH_TAG="aarch64" ;;
  *) print_message error "Unsupported architecture: $ARCH"; exit 1 ;;
esac

INSTALL_DIR="${install_dir_override:-$HOME/.coven-code/bin}"
mkdir -p "$INSTALL_DIR"

# Resolve version
if [[ -z "$specific_version" ]]; then
  print_message info "Fetching latest release..."
  specific_version=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"v?([^"]+)".*/\1/')
fi
print_message info "Installing ${APP} v${specific_version}"

# Check existing
existing_path=$(command -v "$APP" 2>/dev/null || true)
if [[ -n "$existing_path" ]]; then
  installed_version=$("$existing_path" --version 2>/dev/null | head -1 | sed 's/[^0-9.]//g' || echo "unknown")
  print_message info "${MUTED}Found existing ${APP} at ${NC}${existing_path}${MUTED} (v${installed_version}) — upgrading to v${specific_version}${NC}"
fi

ARTIFACT="${APP}-${PLATFORM}-${ARCH_TAG}"
ARCHIVE_NAME="${ARTIFACT}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${specific_version}/${ARCHIVE_NAME}"

if [[ -n "$local_binary" ]]; then
  print_message info "Using local binary: $local_binary"
  # Stage then mv: overwriting an existing binary in place reuses its inode,
  # and macOS caches signature validation per inode — the upgraded binary
  # would be SIGKILLed (Code Signature Invalid) on launch.
  cp "$local_binary" "${INSTALL_DIR}/${APP}.new"
  chmod +x "${INSTALL_DIR}/${APP}.new"
  mv -f "${INSTALL_DIR}/${APP}.new" "${INSTALL_DIR}/${APP}"
else
  tmp_dir=$(mktemp -d -t coven-code-install-XXXXXX)
  trap 'rm -rf "$tmp_dir"' EXIT

  print_message info "Downloading ${ARCHIVE_NAME}..."
  curl -fsSL --progress-bar "$DOWNLOAD_URL" -o "${tmp_dir}/${ARCHIVE_NAME}"
  tar -xzf "${tmp_dir}/${ARCHIVE_NAME}" -C "$tmp_dir"
  cp "${tmp_dir}/${APP}" "${INSTALL_DIR}/${APP}.new"
  chmod +x "${INSTALL_DIR}/${APP}.new"
  mv -f "${INSTALL_DIR}/${APP}.new" "${INSTALL_DIR}/${APP}"
fi

ln -sf "${APP}" "${INSTALL_DIR}/${ALIAS}" || cp "${INSTALL_DIR}/${APP}" "${INSTALL_DIR}/${ALIAS}"
chmod +x "${INSTALL_DIR}/${ALIAS}"
# Remove any `coven` symlink left by older installers — that name belongs to
# the Coven daemon CLI (@opencoven/cli).
if [[ -L "${INSTALL_DIR}/coven" ]]; then
  rm -f "${INSTALL_DIR}/coven"
fi

# PATH setup
shell_rc=""
case "${SHELL:-}" in
  */zsh)  shell_rc="$HOME/.zshrc" ;;
  */bash) shell_rc="$HOME/.bashrc" ;;
  */fish) shell_rc="$HOME/.config/fish/config.fish" ;;
esac
path_line="export PATH=\"\$PATH:${INSTALL_DIR}\""
if [[ -n "$shell_rc" ]] && ! grep -qF "$INSTALL_DIR" "$shell_rc" 2>/dev/null; then
  echo "" >> "$shell_rc"
  echo "# Coven Code" >> "$shell_rc"
  echo "$path_line" >> "$shell_rc"
  print_message info "Added ${INSTALL_DIR} to PATH in ${shell_rc}"
fi

print_message success "${APP} v${specific_version} installed to ${INSTALL_DIR}/${APP}"
print_message success "${ALIAS} alias installed to ${INSTALL_DIR}"
echo ""
echo -e "  ${GREEN}${APP}${NC}              ${MUTED}# Interactive TUI${NC}"
echo -e "  ${GREEN}${ALIAS}${NC}              ${MUTED}# Alias for ${APP}${NC}"
echo -e "  ${GREEN}${APP} -p \"...\"${NC}       ${MUTED}# Headless one-shot${NC}"
echo ""
echo -e "  ${MUTED}Restart your shell or run: source ${shell_rc:-~/.bashrc}${NC}"
