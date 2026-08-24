# syntax=docker/dockerfile:1
# Multi-stage Alpine build for kaifuku (PhotoRec-based recovery GUI)

FROM alpine:latest AS builder

RUN apk add --no-cache \
    build-base musl-dev gcc make cmake pkgconfig \
    rust cargo \
    libjpeg-turbo-dev zlib-dev \
    ncurses-dev \
    libx11-dev libxext-dev libxft-dev libxinerama-dev libxfixes-dev \
    libxrender-dev libxcursor-dev libxrandr-dev libxi-dev \
    mesa-dev mesa-egl mesa-gl \
    pango-dev cairo-dev glib-dev fontconfig-dev

WORKDIR /build
COPY . .

RUN cargo build --release

FROM alpine:latest AS runtime

RUN apk add --no-cache \
    libgcc libstdc++ \
    libjpeg-turbo zlib \
    ncurses-libs \
    libx11 libxext libxft libxinerama libxfixes \
    libxrender libxcursor libxrandr libxi \
    mesa-egl mesa-gl \
    pango cairo glib fontconfig

WORKDIR /opt/kaifuku
COPY --from=builder /build/target/release/kaifuku /opt/kaifuku/kaifuku

ENTRYPOINT ["/opt/kaifuku/kaifuku"]
