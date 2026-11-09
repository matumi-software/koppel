# The Rust Koppel for Linux: static binaries for amd64, arm64 and armv6, the .deb packages, and the image.
# Build context = src/. Docker is all it needs.
#
#   Binaries, to <dir>/<arch>/koppel:
#     docker buildx build -f koppel-bin/Dockerfile --target binaries --output type=local,dest=<dir> .
#   The .deb files (installer/linux/build.ps1 runs this):
#     docker buildx build -f koppel-bin/Dockerfile --target linux-packages --output type=local,dest=<dir> .
#   The image (the last stage, so the default target):
#     docker buildx build -f koppel-bin/Dockerfile --platform linux/amd64,linux/arm64,linux/arm/v6 -t <name> .
#
# Every image is pinned by tag and digest, so a rebuild does not move under us, and every tool by version,
# Zig also by its checksum. To bump an image, `docker buildx imagetools inspect <image>:<tag>` prints the new
# digest (the one at the top, for the whole multi-platform index). Zig's checksums are in
# https://ziglang.org/download/index.json.
#
# The UI and the Rust code build on the build machine's own architecture ($BUILDPLATFORM). cargo-zigbuild
# uses Zig as the C compiler and linker for each target (ring has C and assembly), so one toolchain builds
# all three targets and nothing runs under emulation. The binaries are static (musl), so they need no
# libraries on the machine and the image needs no libc.
#
# The arm binary is built for ARMv6 with hardware floating point (arm-unknown-linux-musleabihf), which every
# 32-bit Raspberry Pi runs, the Zero and the Pi 1 included, and so does any ARMv7 board.

ARG ZIG_VERSION=0.17.1
ARG ZIG_SHA256_X86_64=3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a
ARG ZIG_SHA256_AARCH64=a1546d5646eb184437918d6635ca6b762a0e889eac09cef720ee7a1ed3fe7795
ARG CARGO_ZIGBUILD_VERSION=0.24.1
ARG PNPM_VERSION=9.15.0

# -------- The UI ---------------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:24.22.2-bookworm-slim@sha256:01d566e8d94d8c619591d78652babfccd3c6992a3dffa9ed5ab39e11cf6c70e9 AS ui
ARG PNPM_VERSION
RUN npm install -g pnpm@$PNPM_VERSION
WORKDIR /build
# The workspace's package files first, so the install is cached across source edits.
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY ui/react-ui/package.json ui/react-ui/
COPY ui/app/package.json ui/app/
RUN pnpm install --frozen-lockfile
COPY ui ./ui
RUN pnpm --filter koppel.ui build

# -------- The Rust toolchain: Rust, Zig, cargo-zigbuild and the three targets -------------------
FROM --platform=$BUILDPLATFORM rust:1.99.1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07 AS toolchain
ARG ZIG_VERSION
ARG ZIG_SHA256_X86_64
ARG ZIG_SHA256_AARCH64
ARG CARGO_ZIGBUILD_VERSION
RUN set -eu; \
    arch="$(uname -m)"; \
    case "$arch" in \
      x86_64) sum="$ZIG_SHA256_X86_64" ;; \
      aarch64) sum="$ZIG_SHA256_AARCH64" ;; \
      *) echo "There is no Zig checksum here for a $arch build machine; build on x86_64 or aarch64." >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://ziglang.org/download/$ZIG_VERSION/zig-$arch-linux-$ZIG_VERSION.tar.xz" -o /tmp/zig.tar.xz; \
    echo "$sum  /tmp/zig.tar.xz" | sha256sum -c -; \
    mkdir /opt/zig; \
    tar -C /opt/zig --strip-components=1 -xJf /tmp/zig.tar.xz; \
    rm /tmp/zig.tar.xz; \
    ln -s /opt/zig/zig /usr/local/bin/zig; \
    zig version
RUN cargo install --locked "cargo-zigbuild@$CARGO_ZIGBUILD_VERSION" \
 && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl arm-unknown-linux-musleabihf

# -------- The binaries ---------------------------------------------------------
FROM toolchain AS build
WORKDIR /build
# Every member of the workspace, since Cargo reads them all. A new crate goes here too.
COPY Cargo.toml Cargo.lock ./
COPY koppel-bin ./koppel-bin
COPY koppel-config ./koppel-config
COPY koppel-contract ./koppel-contract
COPY koppel-usb ./koppel-usb
COPY koppel-web ./koppel-web
COPY --from=ui /build/ui/app/dist ./ui/app/dist
# The registry, Zig's cache and target/ are cache mounts, kept between builds. The binaries are copied out of
# target/ to /out/<TARGETARCH>/koppel, where the stages below find them: amd64, arm64, and arm for ARMv6.
RUN --mount=type=cache,id=koppel-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=koppel-zig-cache,target=/root/.cache \
    --mount=type=cache,id=koppel-rust-target,target=/build/target \
    set -eu; \
    for pair in amd64:x86_64-unknown-linux-musl arm64:aarch64-unknown-linux-musl arm:arm-unknown-linux-musleabihf; do \
      arch="${pair%%:*}"; target="${pair#*:}"; \
      cargo zigbuild --locked --release -p koppel --target "$target"; \
      mkdir -p "/out/$arch"; \
      cp "target/$target/release/koppel" "/out/$arch/koppel"; \
    done; \
    cargo pkgid --offline -p koppel | sed 's/.*[#@]//' > /out/version; \
    grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+' /out/version
# The volume's folders, copied into the image below, which has no shell to make them.
RUN mkdir -p /koppel/config /koppel/keys

FROM scratch AS binaries
COPY --from=build /out/ /

# -------- The .deb packages (not part of the image) ----------------------------------
# Built only on request, through installer/linux/build.ps1. nfpm wraps each binary with the files in
# installer/linux; the armv6 binary becomes the armhf package, which is what Raspberry Pi OS (32-bit) installs.
FROM --platform=$BUILDPLATFORM goreleaser/nfpm:v2.48.0@sha256:d57a1452e7a019896b355f7b1ec67262278f6c503631f652d4fce7aef9ac8df7 AS nfpm

FROM build AS deb
COPY --from=nfpm /usr/bin/nfpm /usr/local/bin/nfpm
COPY installer/linux /build/installer/linux
WORKDIR /build/installer/linux
RUN set -eu; \
    VERSION="$(cat /out/version)"; export VERSION; \
    mkdir -p /packages; \
    for pair in amd64:amd64 arm64:arm64 armhf:arm; do \
      rm -rf /stage; mkdir /stage; \
      cp "/out/${pair#*:}/koppel" /stage/koppel; \
      ARCH="${pair%%:*}" nfpm package --config nfpm.yaml --packager deb --target /packages/; \
    done

FROM scratch AS linux-packages
COPY --from=deb /packages/ /

# -------- The image ------------------------------------------------------------
# distroless/static: root certificates, time zone data, /etc/passwd with a nonroot user (65532), no shell
# and no libc. It is published for amd64 and arm64, and for arm only as arm/v7. It holds no programs, only
# files, so for arm/v6 the image takes the arm/v7 variant's files as they are. Its package list then says
# armhf, Debian's name for ARMv7.
FROM --platform=linux/arm/v7 gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329 AS distroless-arm-files

FROM gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329 AS base-amd64
FROM gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329 AS base-arm64
FROM scratch AS base-arm
COPY --from=distroless-arm-files / /
# What distroless sets in its image config, which a copy of its files does not carry.
ENV PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
WORKDIR /home/nonroot

FROM base-${TARGETARCH} AS image
ARG TARGETARCH
# On the PATH, so `docker compose exec koppel koppel --reset-password` finds it.
COPY --from=build /out/${TARGETARCH}/koppel /usr/local/bin/koppel
# One volume root for everything Koppel writes.
COPY --from=build --chown=65532:65532 /koppel /koppel
VOLUME /koppel
# The composes run it as root (user: "0"), which binding and attaching USB devices needs.
USER 65532:65532

# 5240: the UI and everything other Koppel machines do, USB traffic included, over HTTPS; discovery on UDP.
EXPOSE 5240 5240/udp

ENV Paths__Config=/koppel/config
ENV Paths__Keys=/koppel/keys

ENTRYPOINT ["/usr/local/bin/koppel"]
