#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
prefix="$project_root/.codex-native"
source_dir="$prefix/src/gtk4-layer-shell"
build_dir="$prefix/build/gtk4-layer-shell"
layer_shell_version=1.3.0

mkdir -p "$prefix/lib/pkgconfig" "$prefix/src" "$prefix/build"

if ! PKG_CONFIG_PATH="$prefix/lib/pkgconfig" \
    pkg-config --atleast-version="$layer_shell_version" gtk4-layer-shell-0 2>/dev/null; then
    if [ ! -d "$source_dir/.git" ]; then
        git clone --depth 1 --branch "v$layer_shell_version" \
            https://github.com/wmww/gtk4-layer-shell.git "$source_dir"
    fi

    if [ ! -f "$build_dir/build.ninja" ]; then
        meson setup "$build_dir" "$source_dir" \
            --prefix="$prefix" \
            --libdir=lib \
            --buildtype=release \
            -Dexamples=false \
            -Dtests=false \
            -Ddocs=false
    else
        meson setup --reconfigure "$build_dir" "$source_dir" \
            --prefix="$prefix" \
            --libdir=lib \
            --buildtype=release \
            -Dexamples=false \
            -Dtests=false \
            -Ddocs=false
    fi

    meson compile -C "$build_dir"
    meson install -C "$build_dir"
fi

if [ ! -f "$prefix/lib/libpam.so.0" ]; then
    cc -fPIC -shared \
        -Wl,-soname,libpam.so.0 \
        -o "$prefix/lib/libpam.so.0" \
        "$project_root/scripts/pam-link-shim.c"
fi
ln -sfn libpam.so.0 "$prefix/lib/libpam.so"

echo "Persistent Codex native dependencies are ready in $prefix"
