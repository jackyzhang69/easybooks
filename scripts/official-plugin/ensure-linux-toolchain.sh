#!/usr/bin/env bash
# Ensure the linux-x64 cross lane has its toolchain: zig + cargo-zigbuild.
#
# Sourced (not executed) so PATH changes apply to the caller:
#   source scripts/official-plugin/ensure-linux-toolchain.sh
#   bash scripts/stage-package
#
# Idempotent: an already-usable zig/cargo-zigbuild costs one version check.

ZIG_VERSION="0.17.0"
ZIGBUILD_VERSION="0.24.0"
_ELT_TMP="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"

if ! command -v zig >/dev/null 2>&1; then
  case "$(uname -m)" in
    arm64 | aarch64) _elt_arch="aarch64" ;;
    x86_64) _elt_arch="x86_64" ;;
    *)
      echo "ensure-linux-toolchain: unsupported host architecture $(uname -m)" >&2
      return 1 2>/dev/null || exit 1
      ;;
  esac
  _elt_unpack="$_ELT_TMP/zig-unpack"
  rm -rf "$_elt_unpack"
  mkdir -p "$_elt_unpack"
  _elt_url="https://ziglang.org/download/${ZIG_VERSION}/zig-${_elt_arch}-macos-${ZIG_VERSION}.tar.xz"
  curl -fsSL -o "$_ELT_TMP/zig.tar.xz" "$_elt_url"
  tar -xJf "$_ELT_TMP/zig.tar.xz" -C "$_elt_unpack"
  _elt_bin="$(find "$_elt_unpack" -type f -name zig -perm -u+x | head -1)"
  if [ -z "$_elt_bin" ]; then
    echo "ensure-linux-toolchain: the zig archive contained no zig binary" >&2
    return 1 2>/dev/null || exit 1
  fi
  export PATH="$(dirname "$_elt_bin"):$PATH"
fi

# The rust standard library for the target is not installed by default; a
# missing core/std fails the cross build with E0463.
if command -v rustup >/dev/null 2>&1; then
  rustup target list --installed | grep -qx "x86_64-unknown-linux-gnu" \
    || rustup target add x86_64-unknown-linux-gnu
fi

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  cargo install cargo-zigbuild --locked --version "$ZIGBUILD_VERSION"
  export PATH="$HOME/.cargo/bin:$PATH"
fi

zig version
cargo-zigbuild --version
