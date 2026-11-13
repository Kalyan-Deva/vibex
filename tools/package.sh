#!/bin/sh
# Build the release Vibex and its Linux tarball into dist/.
#
#   tools/package.sh [--target-dir DIR]
#
# Output: dist/vibex-<version>-linux-<arch>.tar.gz (+ .sha256)
# Unpack it and run ./install.sh (per user, no root; --help for options).
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

while [ $# -gt 0 ]; do
    case "$1" in
        --target-dir) export CARGO_TARGET_DIR=$2; shift 2 ;;
        *) echo "package.sh: unknown option '$1'" >&2; exit 2 ;;
    esac
done
target=${CARGO_TARGET_DIR:-$root/target}

cargo build --release -p vibex
exe="$target/release/vibex"

version=$("$exe" --version | awk '{print $2}')
arch=$(uname -m)
name="vibex-$version-linux-$arch"
stage="$target/package/$name"
rm -rf "$stage"
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/vibex/shell" "$stage/share/doc/vibex"

cp "$exe" "$stage/bin/vibex"
# drop debug symbols; the build itself is unchanged
if command -v strip >/dev/null 2>&1; then
    strip "$stage/bin/vibex"
fi
cp packaging/linux/install.sh "$stage/install.sh"
chmod 755 "$stage/install.sh" "$stage/bin/vibex"
cp packaging/linux/vibex.desktop "$stage/share/applications/"
for size in 16 24 32 48 64 128 256 512; do
    mkdir -p "$stage/share/icons/hicolor/${size}x${size}/apps"
    cp "assets/vibex-$size.png" "$stage/share/icons/hicolor/${size}x${size}/apps/vibex.png"
done
cp shell/vibex.bash "$stage/share/vibex/shell/"
cp README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md "$stage/share/doc/vibex/"

mkdir -p dist
out="$root/dist/$name.tar.gz"
# owner-neutral, sorted, so the tarball doesn't depend on who built it
tar --sort=name --owner=0 --group=0 --numeric-owner -C "$target/package" -czf "$out" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "built $out ($(du -h "$out" | cut -f1))"
echo "sha256 $(cut -d' ' -f1 "$out.sha256")"
