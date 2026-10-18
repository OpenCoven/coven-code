#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
TMP=$(mktemp -d -t coven-installer-test-XXXXXX)
trap 'rm -rf "$TMP"' EXIT

ARCHIVE=coven-code-linux-x86_64.tar.gz
PAYLOAD="$TMP/$ARCHIVE"
printf 'inert archive bytes\n' > "$PAYLOAD"
DIGEST=$(shasum -a 256 "$PAYLOAD" | awk '{print $1}')
NAMES=(coven-code-linux-aarch64.tar.gz coven-code-linux-x86_64.tar.gz coven-code-macos-aarch64.tar.gz coven-code-macos-x86_64.tar.gz coven-code-windows-x86_64.zip)

manifest() {
  local selected=${1:-$DIGEST}
  for name in "${NAMES[@]}"; do
    if [[ "$name" == "$ARCHIVE" ]]; then printf '%s  %s\n' "$selected" "$name"; else printf '%064d  %s\n' 0 "$name"; fi
  done
}

run_case() {
  local label=$1 manifest_body=$2 expect=$3
  local case_dir="$TMP/$label" fake="$TMP/$label/fake" install="$TMP/$label/install"
  mkdir -p "$fake" "$install"
  printf 'existing\n' > "$install/coven-code"
  printf '%s' "$manifest_body" > "$case_dir/SHA256SUMS"
  cat > "$fake/uname" <<'EOF'
#!/bin/sh
[ "$1" = "-s" ] && printf 'Linux\n' || printf 'x86_64\n'
EOF
  cat > "$fake/curl" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> '$case_dir/curl.log'
out=''; url=''; while [ \$# -gt 0 ]; do case "\$1" in -o) out=\$2; shift 2;; http*) url=\$1; shift;; *) shift;; esac; done
case "\$url" in */releases/latest) printf '{"tag_name":"v9.9.9"}\n';; */SHA256SUMS) cp '$case_dir/SHA256SUMS' "\$out";; */$ARCHIVE) cp '$PAYLOAD' "\$out";; *) exit 91;; esac
EOF
  cat > "$fake/tar" <<EOF
#!/bin/sh
printf 'called\n' >> '$case_dir/tar.log'
dest=''; while [ \$# -gt 0 ]; do if [ "\$1" = '-C' ]; then dest=\$2; break; fi; shift; done
printf 'inert binary\n' > "\$dest/coven-code"
EOF
  chmod +x "$fake"/*
  if [[ "$label" == download_failure ]]; then
    cat > "$fake/curl" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> '$case_dir/curl.log'
exit 22
EOF
    chmod +x "$fake/curl"
  elif [[ "$label" == hash_tool_failure ]]; then
    printf '#!/bin/sh\nexit 127\n' > "$fake/sha256sum"
    printf '#!/bin/sh\nexit 127\n' > "$fake/shasum"
    chmod +x "$fake/sha256sum" "$fake/shasum"
  fi
  local install_args=(--version 9.9.9 --install-dir "$install")
  if [[ "$label" == latest ]]; then install_args=(--install-dir "$install"); fi
  set +e
  HOME="$case_dir/home" SHELL=/bin/false PATH="$fake:/usr/bin:/bin" bash "$ROOT/install.sh" "${install_args[@]}" >"$case_dir/out" 2>&1
  status=$?
  set -e
  if [[ "$expect" == pass ]]; then
    [[ $status -eq 0 ]] || { cat "$case_dir/out"; return 1; }
    [[ -f "$case_dir/tar.log" ]]
  else
    [[ $status -ne 0 ]]
    [[ ! -e "$case_dir/tar.log" ]]
    [[ $(cat "$install/coven-code") == existing ]]
  fi
  grep -q -- "--proto =https" "$case_dir/curl.log"
  grep -q -- "--proto-redir =https" "$case_dir/curl.log"
  while IFS= read -r args; do
    [[ " $args " == *" --connect-timeout 30 "* ]]
    [[ " $args " == *" --max-time 300 "* ]]
  done < "$case_dir/curl.log"
  if [[ "$label" == latest ]]; then [[ $(wc -l < "$case_dir/curl.log") -eq 3 ]]; fi
}

run_case valid "$(manifest)" pass
run_case latest "$(manifest)" pass
UPPER_DIGEST=$(printf '%s' "$DIGEST" | tr '[:lower:]' '[:upper:]')
run_case uppercase "$(manifest "$UPPER_DIGEST")" pass
run_case mismatch "$(manifest "$(printf '%064d' 1)")" fail
run_case missing_entry "$(manifest | sed '/macos-aarch64/d')" fail
run_case malformed "not-a-manifest" fail
run_case duplicate "$(manifest; printf '%s  %s\n' "$DIGEST" "$ARCHIVE")" fail
run_case wrong_filename "$(manifest | sed "s/$ARCHIVE/coven-code-linux-x86_64.tar.gz\/..\/evil/")" fail
run_case download_failure "$(manifest)" fail
run_case hash_tool_failure "$(manifest)" fail

printf 'install.sh checksum fixtures passed\n'
