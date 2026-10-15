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
  if [[ $# -lt 2 && ( "$1" == --version || "$1" == --install-dir || "$1" == --binary ) ]]; then
    print_message error "Missing value for $1" >&2
    usage >&2
    exit 1
  fi
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

# Local installs do not need release metadata or network access.
if [[ -n "$local_binary" ]]; then
  install_source="local binary"
else
  if [[ -z "$specific_version" ]]; then
    print_message info "Fetching latest release..."
    specific_version=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"v?([^"]+)".*/\1/')
  fi
  install_source="v${specific_version}"
fi
print_message info "Installing ${APP} (${install_source})"

# Check existing
existing_path=$(command -v "$APP" 2>/dev/null || true)
if [[ -n "$existing_path" ]]; then
  installed_version=$("$existing_path" --version 2>/dev/null | head -1 | sed 's/[^0-9.]//g' || echo "unknown")
  print_message info "${MUTED}Found existing ${APP} at ${NC}${existing_path}${MUTED} (v${installed_version}) — installing ${install_source}${NC}"
fi

expected_checksum() {
  local manifest="$1" selected="$2"
  awk -v selected="$selected" '
    BEGIN {
      expected["coven-code-windows-x86_64.zip"] = 1
      expected["coven-code-linux-x86_64.tar.gz"] = 1
      expected["coven-code-linux-aarch64.tar.gz"] = 1
      expected["coven-code-macos-x86_64.tar.gz"] = 1
      expected["coven-code-macos-aarch64.tar.gz"] = 1
    }
    {
      if (length($0) < 67 || substr($0, 65, 2) != "  ") exit 20
      digest = substr($0, 1, 64)
      name = substr($0, 67)
      if (length(digest) != 64 || digest ~ /[^0-9A-Fa-f]/) exit 21
      if (!(name in expected) || seen[name]++) exit 22
      count++
      if (name == selected) found = tolower(digest)
    }
    END {
      if (count != 5 || length(found) != 64) exit 23
      print found
    }
  ' "$manifest"
}

file_checksum() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print tolower($1)}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print tolower($1)}'
  else
    print_message error "SHA-256 verification requires sha256sum or shasum."
    return 1
  fi
}

mkdir -p "$INSTALL_DIR"

if [[ -n "$local_binary" ]]; then
  print_message info "Using local binary: $local_binary"
  # Stage then mv: overwriting an existing binary in place reuses its inode,
  # and macOS caches signature validation per inode — the upgraded binary
  # would be SIGKILLed (Code Signature Invalid) on launch.
  cp "$local_binary" "${INSTALL_DIR}/${APP}.new"
  chmod +x "${INSTALL_DIR}/${APP}.new"
  mv -f "${INSTALL_DIR}/${APP}.new" "${INSTALL_DIR}/${APP}"
else
  ARTIFACT="${APP}-${PLATFORM}-${ARCH_TAG}"
  ARCHIVE_NAME="${ARTIFACT}.tar.gz"
  DOWNLOAD_URL="https://github.com/${REPO}/releases/download/v${specific_version}/${ARCHIVE_NAME}"
  CHECKSUM_URL="https://github.com/${REPO}/releases/download/v${specific_version}/SHA256SUMS"

  tmp_dir=$(mktemp -d -t coven-code-install-XXXXXX)
  trap 'rm -rf "$tmp_dir"' EXIT

  command -v curl >/dev/null 2>&1 || { print_message error "curl is required."; exit 1; }
  command -v tar >/dev/null 2>&1 || { print_message error "tar is required."; exit 1; }
  command -v awk >/dev/null 2>&1 || { print_message error "awk is required."; exit 1; }

  print_message info "Downloading SHA256SUMS..."
  curl --fail --show-error --silent --location --proto '=https' \
    --proto-redir '=https' --tlsv1.2 "$CHECKSUM_URL" -o "${tmp_dir}/SHA256SUMS"
  print_message info "Downloading ${ARCHIVE_NAME}..."
  curl --fail --show-error --silent --location --proto '=https' \
    --proto-redir '=https' --tlsv1.2 --progress-bar "$DOWNLOAD_URL" \
    -o "${tmp_dir}/${ARCHIVE_NAME}"
  if ! expected=$(expected_checksum "${tmp_dir}/SHA256SUMS" "$ARCHIVE_NAME"); then
    print_message error "SHA256SUMS is missing, malformed, ambiguous, or incomplete."
    exit 1
  fi
  actual=$(file_checksum "${tmp_dir}/${ARCHIVE_NAME}")
  if [[ "$actual" != "$expected" ]]; then
    print_message error "Checksum mismatch for ${ARCHIVE_NAME}."
    exit 1
  fi
  print_message success "Verified SHA-256 for ${ARCHIVE_NAME}"
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

print_message success "${APP} (${install_source}) installed to ${INSTALL_DIR}/${APP}"
print_message success "${ALIAS} alias installed to ${INSTALL_DIR}"

# The unified Coven CLI keeps its own engine under ~/.coven/engine and runs that
# one ahead of anything on PATH. Say so, or this copy looks like an upgrade
# that `coven` ignores.
managed_engine_root="$HOME/.coven/engine"
if [[ -f "${managed_engine_root}/current" ]]; then
  managed_version=$(tr -d '[:space:]' < "${managed_engine_root}/current" 2>/dev/null || true)
  print_message warning "The Coven CLI already manages an engine at ${managed_engine_root}/${managed_version:-current} and runs that one, not this copy."
  print_message warning "Upgrade the engine 'coven' runs with: coven engine install. Keep this copy only if you launch '${APP}' directly; 'coven doctor' reports which one is in use."
fi
echo ""
echo -e "  ${GREEN}${APP}${NC}              ${MUTED}# Interactive TUI${NC}"
echo -e "  ${GREEN}${ALIAS}${NC}              ${MUTED}# Alias for ${APP}${NC}"
echo -e "  ${GREEN}${APP} -p \"...\"${NC}       ${MUTED}# Headless one-shot${NC}"
echo ""
echo -e "  ${MUTED}Restart your shell or run: source ${shell_rc:-~/.bashrc}${NC}"
