#!/bin/sh
# Copyright 2026 Tor Norbye
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Installer for markfmt release archives. Fetches a prebuilt archive and SHA256SUMS.txt,
# verifies the archive checksum, and installs the binary into $MARKFMT_INSTALL_DIR (~/.local/bin).

set -eu

REPO="${MARKFMT_REPO:-tnorbye/markfmt}"
RELEASE_TAG="__MARKFMT_RELEASE_TAG__"
INSTALL_DIR="${MARKFMT_INSTALL_DIR:-$HOME/.local/bin}"
VERSION_ARG="${MARKFMT_VERSION:-}"
TARGET="${MARKFMT_TARGET:-}"
BASE_URL="${MARKFMT_BASE_URL:-https://github.com/${REPO}/releases/download}"
QUIET=0

usage() {
    cat <<'EOF'
Install markfmt from a GitHub Release archive.

Usage: install.sh [OPTIONS]

Options:
  -d, --dir <DIR>       Install directory (default: $HOME/.local/bin)
  -v, --version <VER>   Version or tag to install (e.g. 1.0.0 or v1.0.0)
  -q, --quiet           Suppress informational output
  -h, --help            Print this help message

Environment variables:
  MARKFMT_INSTALL_DIR   Override install directory
  MARKFMT_VERSION       Override version or tag to install
  MARKFMT_TARGET        Override target triple
  MARKFMT_BASE_URL      Override release download base URL
  MARKFMT_REPO          Override GitHub repository (default: tnorbye/markfmt)
EOF
}

err() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        -d|--dir)
            [ $# -ge 2 ] || err "Missing value for $1"
            INSTALL_DIR="$2"
            shift 2
            ;;
        --dir=*)
            INSTALL_DIR="${1#--dir=}"
            shift
            ;;
        -v|--version)
            [ $# -ge 2 ] || err "Missing value for $1"
            VERSION_ARG="$2"
            shift 2
            ;;
        --version=*)
            VERSION_ARG="${1#--version=}"
            shift
            ;;
        -q|--quiet)
            QUIET=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            err "Unknown option: $1 (run with --help for usage)"
            ;;
    esac
done

[ -n "$INSTALL_DIR" ] || err "Install directory must not be empty"

download() {
    url="$1"
    dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$dest"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$dest" "$url"
    else
        err "Either curl or wget is required to download markfmt"
    fi
}

resolve_latest_tag() {
    latest_url="https://github.com/${REPO}/releases/latest"
    if command -v curl >/dev/null 2>&1; then
        effective_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "$latest_url")"
    elif command -v wget >/dev/null 2>&1; then
        effective_url="$(wget --max-redirect=10 --spider -S "$latest_url" 2>&1 | awk '/^[[:space:]]*Location:/ { sub(/\r$/, "", $2); print $2 }' | tail -n 1)"
    else
        err "Either curl or wget is required to resolve the latest markfmt release"
    fi

    resolved="${effective_url##*/releases/tag/}"
    if [ -z "$resolved" ] || [ "$resolved" = "$effective_url" ] || [ "${resolved#*/}" != "$resolved" ]; then
        err "Could not resolve latest release tag from ${latest_url}"
    fi
    printf '%s\n' "$resolved"
}

if [ -n "$VERSION_ARG" ]; then
    case "$VERSION_ARG" in
        v*) TAG="$VERSION_ARG" ;;
        *) TAG="v${VERSION_ARG}" ;;
    esac
else
    case "$RELEASE_TAG" in
        ''|__MARKFMT_*) TAG="$(resolve_latest_tag)" ;;
        *) TAG="$RELEASE_TAG" ;;
    esac
fi

VERSION="${TAG#v}"
[ -n "$VERSION" ] || err "Resolved release tag '${TAG}' is empty"

if [ -z "$TARGET" ]; then
    OS="$(uname -s)"
    ARCH="$(uname -m)"
    case "$OS" in
        Linux)
            case "$ARCH" in
                x86_64|amd64) TARGET="x86_64-unknown-linux-musl" ;;
                aarch64|arm64) TARGET="aarch64-unknown-linux-musl" ;;
                *) err "Unsupported Linux architecture: ${ARCH}" ;;
            esac
            ;;
        Darwin)
            # Shells running under Rosetta 2 on Apple Silicon report x86_64.
            if [ "$ARCH" = "x86_64" ] && command -v sysctl >/dev/null 2>&1; then
                if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
                    ARCH="arm64"
                fi
            fi
            case "$ARCH" in
                arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
                x86_64|amd64) TARGET="x86_64-apple-darwin" ;;
                *) err "Unsupported macOS architecture: ${ARCH}" ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            case "$ARCH" in
                x86_64|amd64) TARGET="x86_64-pc-windows-msvc" ;;
                *) err "Unsupported Windows architecture: ${ARCH}" ;;
            esac
            ;;
        *)
            err "Unsupported operating system: ${OS}"
            ;;
    esac
fi

case "$TARGET" in
    *-windows-*)
        EXT="zip"
        BIN="markfmt.exe"
        ;;
    *)
        EXT="tar.gz"
        BIN="markfmt"
        ;;
esac

STAGE="markfmt-v${VERSION}-${TARGET}"
ARCHIVE="${STAGE}.${EXT}"
DOWNLOAD_BASE="${BASE_URL%/}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

if [ "$QUIET" -eq 0 ]; then
    printf 'Downloading %s...\n' "$ARCHIVE"
fi

download "${DOWNLOAD_BASE}/${TAG}/${ARCHIVE}" "${TMP_DIR}/${ARCHIVE}"
download "${DOWNLOAD_BASE}/${TAG}/SHA256SUMS.txt" "${TMP_DIR}/SHA256SUMS.txt"

EXPECTED="$(awk -v f="$ARCHIVE" '$2 == f || $2 == "*" f { print tolower($1); exit }' "${TMP_DIR}/SHA256SUMS.txt")"
[ -n "$EXPECTED" ] || err "Missing checksum for ${ARCHIVE} in SHA256SUMS.txt"

if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "${TMP_DIR}/${ARCHIVE}" | awk '{print tolower($1)}')"
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TMP_DIR}/${ARCHIVE}" | awk '{print tolower($1)}')"
elif command -v openssl >/dev/null 2>&1; then
    ACTUAL="$(openssl dgst -sha256 "${TMP_DIR}/${ARCHIVE}" | awk '{print tolower($NF)}')"
else
    err "Need sha256sum, shasum, or openssl to verify download checksum"
fi

[ "$ACTUAL" = "$EXPECTED" ] || err "Checksum verification failed for ${ARCHIVE}: expected ${EXPECTED}, got ${ACTUAL}"

case "$EXT" in
    tar.gz)
        tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"
        ;;
    zip)
        if command -v unzip >/dev/null 2>&1; then
            unzip -q "${TMP_DIR}/${ARCHIVE}" -d "$TMP_DIR"
        else
            tar -xf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"
        fi
        ;;
esac

[ -f "${TMP_DIR}/${STAGE}/${BIN}" ] || err "Archive ${ARCHIVE} did not contain ${STAGE}/${BIN}"

mkdir -p "$INSTALL_DIR"
chmod 755 "${TMP_DIR}/${STAGE}/${BIN}"
rm -f "${INSTALL_DIR}/${BIN}"
cp "${TMP_DIR}/${STAGE}/${BIN}" "${INSTALL_DIR}/${BIN}"

if [ "$QUIET" -eq 0 ]; then
    printf 'Installed markfmt %s to %s/%s\n' "$VERSION" "$INSTALL_DIR" "$BIN"
    case ":${PATH:-}:" in
        *":${INSTALL_DIR}:"*) ;;
        *)
            printf 'Add it to your PATH with:\n  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
            ;;
    esac
fi
