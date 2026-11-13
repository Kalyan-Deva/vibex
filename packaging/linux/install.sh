#!/bin/sh
# Install Vibex from this folder (the unpacked release tarball).
#
#   ./install.sh                    per user, into ~/.local (no root)
#   ./install.sh --prefix /usr/local     system-wide (run with sudo)
#   ./install.sh --uninstall [--prefix DIR]   remove what an install put there
#
# Your config in ~/.config/vibex is never touched.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
prefix="$HOME/.local"
uninstall=0

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) [ $# -ge 2 ] || { echo "install.sh: --prefix needs a folder" >&2; exit 2; }; prefix=$2; shift 2 ;;
        --prefix=*) prefix=${1#--prefix=}; shift ;;
        --uninstall) uninstall=1; shift ;;
        -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "install.sh: unknown option '$1' (try --help)" >&2; exit 2 ;;
    esac
done

case "$prefix" in
    /*) ;;
    *) echo "install.sh: --prefix must be an absolute path" >&2; exit 2 ;;
esac

manifest="$prefix/share/vibex/installed-files"

refresh_caches() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -f "$prefix/share/icons/hicolor/index.theme" ]; then
        gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2>/dev/null || true
    fi
}

if [ "$uninstall" = 1 ]; then
    if [ ! -f "$manifest" ]; then
        echo "install.sh: no Vibex install found under $prefix" >&2
        exit 1
    fi
    # only remove files this script installed
    while IFS= read -r f; do
        case "$f" in
            "$prefix"/*) rm -f "$f" ;;
        esac
    done < "$manifest"
    rm -f "$manifest"
    rmdir "$prefix/share/vibex/shell" "$prefix/share/vibex" "$prefix/share/doc/vibex" 2>/dev/null || true
    refresh_caches
    echo "Removed Vibex from $prefix"
    exit 0
fi

[ -x "$here/bin/vibex" ] || { echo "install.sh: run this from the unpacked Vibex folder (no bin/vibex here)" >&2; exit 1; }

mkdir -p "$prefix/share/vibex"
tmp_manifest="$manifest.new"
: > "$tmp_manifest"

# put SRC (relative to this folder) at DEST with MODE, and remember it
put() {
    src=$1 dest=$2 mode=$3
    mkdir -p "$(dirname "$dest")"
    # write next to the target then rename, so a running Vibex keeps its binary
    cp "$here/$src" "$dest.tmp.$$"
    chmod "$mode" "$dest.tmp.$$"
    mv -f "$dest.tmp.$$" "$dest"
    echo "$dest" >> "$tmp_manifest"
}

put bin/vibex "$prefix/bin/vibex" 755
for icon in "$here"/share/icons/hicolor/*/apps/vibex.png; do
    rel=${icon#"$here"/}
    put "$rel" "$prefix/$rel" 644
done
put share/vibex/shell/vibex.bash "$prefix/share/vibex/shell/vibex.bash" 644
for doc in "$here"/share/doc/vibex/*; do
    rel=${doc#"$here"/}
    put "$rel" "$prefix/$rel" 644
done

# the menu entry names vibex by full path: ~/.local/bin isn't always on the
# desktop session's PATH
desktop="$prefix/share/applications/vibex.desktop"
mkdir -p "$(dirname "$desktop")"
esc=$(printf '%s' "$prefix/bin/vibex" | sed 's/[\\&|]/\\&/g')
sed -e "s|^TryExec=vibex\$|TryExec=$esc|" -e "s|^Exec=vibex|Exec=$esc|" "$here/share/applications/vibex.desktop" > "$desktop.tmp.$$"
chmod 644 "$desktop.tmp.$$"
mv -f "$desktop.tmp.$$" "$desktop"
echo "$desktop" >> "$tmp_manifest"

mv -f "$tmp_manifest" "$manifest"
refresh_caches

echo "Installed Vibex $("$prefix/bin/vibex" --version 2>/dev/null | sed 's/^vibex //') to $prefix"
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Note: $prefix/bin isn't on your PATH; add it to use 'vibex' from a shell." ;;
esac
echo "Shell integration (prompt marks, command blocks): add to ~/.bashrc"
echo "  [ -f \"$prefix/share/vibex/shell/vibex.bash\" ] && . \"$prefix/share/vibex/shell/vibex.bash\""
echo "Quake dropdown: bind 'vibex quake' to a key in your desktop's keyboard shortcuts."
