# The Rust Koppel for Linux: static binaries for amd64, arm64 and armv6, the .deb packages, and the image.
# Build context = src/. Docker is all it needs.
#
#   Binaries, to <dir>/<arch>/koppel:
#     docker buildx build -f installer/docker/Dockerfile --target binaries --output type=local,dest=<dir> .
#   The .deb files (installer/linux/build.ps1 runs this):
#     docker buildx build -f installer/docker/Dockerfile --target linux-packages --output type=local,dest=<dir> .
#   The image (the last stage, so the default target):
#     docker buildx build -f installer/docker/Dockerfile --platform linux/amd64,linux/arm64,linux/arm/v6 -t <name> .
#
# Every image is pinned by tag and digest, so a rebuild does not move under us, and every tool by version,
# Zig and PowerShell also by their checksums. To bump an image, `docker buildx imagetools inspect <image>:<tag>` prints the new
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
# The third-party notices: cargo-about as the Windows installer's build uses it, and PowerShell to run the same
# script, src/installer/notices/new-notices.ps1. PowerShell's checksums are on each release's assets in
# https://github.com/PowerShell/PowerShell/releases.
ARG CARGO_ABOUT_VERSION=0.9.3
ARG PWSH_VERSION=7.6.8
ARG PWSH_SHA256_X64=59112e38ad83350e4f6dc0195953c66b5926792f300ca2e7691b8242e7a16e7f
ARG PWSH_SHA256_ARM64=3179e07fd98f026d39c730ebcb7626d071aac3351b171dda13f9f38d99efcef0

# -------- The UI ---------------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:24.22.2-bookworm-slim@sha256:01d566e8d94d8c619591d78652babfccd3c6992a3dffa9ed5ab39e11cf6c70e9 AS ui
ARG PNPM_VERSION
RUN npm install -g pnpm@$PNPM_VERSION
WORKDIR /build/ui
# The workspace's package files first, so the install is cached across source edits.
COPY ui/package.json ui/pnpm-workspace.yaml ui/pnpm-lock.yaml ./
COPY ui/react-ui/package.json react-ui/
COPY ui/app/package.json app/
RUN pnpm install --frozen-lockfile
COPY ui ./
RUN pnpm --filter koppel.ui build
# Later stages (notices) work from /build, where installer/notices lands.
WORKDIR /build

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
COPY koppel ./koppel
COPY koppel-config ./koppel-config
COPY koppel-usb ./koppel-usb
COPY koppel-web ./koppel-web
COPY koppel-windows ./koppel-windows
COPY --from=ui /build/ui/app/dist ./ui/app/dist
# The commit `koppel --version` names. The build has no .git, so the caller passes it (build.ps1, deploy.sh);
# without it the version says `unknown`.
ARG KOPPEL_GIT_COMMIT
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

# -------- The third-party notices, for the .deb files and the image ---------------------------
# The same script and licence policy as the Windows installer (src/installer/notices). cargo-about lists the
# crates of the three Linux targets as JSON where cargo and the crates' sources are; the script then runs under
# PowerShell where pnpm and the UI's packages are, and writes /notices/THIRD-PARTY-NOTICES.txt.
FROM toolchain AS cargo-about
ARG CARGO_ABOUT_VERSION
RUN cargo install --locked --features cli "cargo-about@$CARGO_ABOUT_VERSION"

FROM build AS crate-licences
COPY --from=cargo-about /usr/local/cargo/bin/cargo-about /usr/local/cargo/bin/cargo-about
COPY installer/notices/about.toml ./installer/notices/about.toml
RUN --mount=type=cache,id=koppel-cargo-registry,target=/usr/local/cargo/registry \
    cargo about generate --format json -c installer/notices/about.toml -m koppel/Cargo.toml \
      --target x86_64-unknown-linux-musl --target aarch64-unknown-linux-musl --target arm-unknown-linux-musleabihf \
      -o /crates.json

FROM toolchain AS pwsh
ARG PWSH_VERSION
ARG PWSH_SHA256_X64
ARG PWSH_SHA256_ARM64
RUN set -eu; \
    case "$(uname -m)" in \
      x86_64) arch=x64; sum="$PWSH_SHA256_X64" ;; \
      aarch64) arch=arm64; sum="$PWSH_SHA256_ARM64" ;; \
      *) echo "There is no PowerShell checksum here for a $(uname -m) build machine; build on x86_64 or aarch64." >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/PowerShell/PowerShell/releases/download/v$PWSH_VERSION/powershell-$PWSH_VERSION-linux-$arch.tar.gz" -o /tmp/pwsh.tar.gz; \
    echo "$sum  /tmp/pwsh.tar.gz" | sha256sum -c -; \
    mkdir /opt/pwsh; \
    tar -C /opt/pwsh -xzf /tmp/pwsh.tar.gz; \
    chmod +x /opt/pwsh/pwsh

FROM ui AS notices
COPY --from=pwsh /opt/pwsh /opt/pwsh
COPY installer/notices ./installer/notices
COPY --from=crate-licences /crates.json /tmp/crates.json
# No ICU in this image, and the script needs none. The build context is src/, without the repo's LICENSE, so
# Koppel's licence text comes from Debian's copy, which is the same GPL-3.0 text.
ENV DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1 POWERSHELL_TELEMETRY_OPTOUT=1
RUN mkdir /notices \
 && /opt/pwsh/pwsh -NoLogo -NoProfile -NonInteractive -File installer/notices/new-notices.ps1 \
      -AboutJson /tmp/crates.json -License /usr/share/common-licenses/GPL-3 -Output /notices/THIRD-PARTY-NOTICES.txt

# -------- The .deb packages (not part of the image) ----------------------------------
# Built only on request, through installer/linux/build.ps1. nfpm wraps each binary with the files in
# installer/linux; the armv6 binary becomes the armhf package, which is what Raspberry Pi OS (32-bit) installs.
FROM --platform=$BUILDPLATFORM goreleaser/nfpm:v2.48.0@sha256:d57a1452e7a019896b355f7b1ec67262278f6c503631f652d4fce7aef9ac8df7 AS nfpm

FROM build AS deb
COPY --from=nfpm /usr/bin/nfpm /usr/local/bin/nfpm
COPY --from=notices /notices /notices
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
# On the PATH, so `docker compose exec koppel koppel reset-password` finds it.
COPY --from=build /out/${TARGETARCH}/koppel /usr/local/bin/koppel
COPY --from=notices /notices/THIRD-PARTY-NOTICES.txt /usr/share/doc/koppel/THIRD-PARTY-NOTICES.txt
# One volume root for everything Koppel writes.
COPY --from=build --chown=65532:65532 /koppel /koppel
VOLUME /koppel
# The composes run it as root (user: "0"), which binding and attaching USB devices needs.
USER 65532:65532

# 5240: the UI and everything other Koppel machines do, USB traffic included, over HTTPS; discovery on UDP.
EXPOSE 5240 5240/udp

ENV KOPPEL_CONFIG_DIR=/koppel/config
ENV KOPPEL_KEYS_DIR=/koppel/keys

# GET /healthz on the UI port, asked at KOPPEL_LISTEN_ADDRESS and KOPPEL_UI_PORT and then at 127.0.0.1 (koppel/src/
# healthcheck.rs), as 0.1.x's image did. It is Koppel itself because the image has no shell or curl. Plain
# Docker only reports "unhealthy": it does not restart the container (restart: unless-stopped acts on exits, not
# on health). Swarm and Kubernetes act on it; with plain Docker, something has to watch it, for example an
# autoheal container, or `docker ps --filter health=unhealthy`.
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
  CMD ["/usr/local/bin/koppel", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/koppel"]
