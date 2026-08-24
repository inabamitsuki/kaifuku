#!/bin/sh
# Build kaifuku natively on Alpine Linux.
# Installs the required system packages, then runs cargo build --release.
set -e

if [ "$(id -u)" -eq 0 ]; then
    apk add --no-cache \
        build-base musl-dev gcc make cmake pkgconfig \
        rust cargo \
        libjpeg-turbo-dev zlib-dev \
        ncurses-dev \
        libx11-dev libxext-dev libxft-dev libxinerama-dev libxfixes-dev \
        libxrender-dev libxcursor-dev libxrandr-dev libxi-dev \
        mesa-dev mesa-egl mesa-gl \
        pango-dev cairo-dev glib-dev fontconfig-dev
fi

cargo build --release

echo "Build complete: $(pwd)/target/release/kaifuku"
