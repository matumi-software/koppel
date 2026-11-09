# Multi-stage build for Koppel. Build context = src/ (see Koppel/docker-compose.yml).
#
# One image serves both roles; the role is a setting. USB needs a privileged container running as
# root and the host's usbip-host / vhci-hcd modules; see docker-compose.yml.

# Base images are pinned to exact versions so a rebuild does not move under us. To bump one, pick the new tag
# on mcr.microsoft.com / Docker Hub and change it here; the SDK and the runtime must be the same .NET patch
# (`docker run --rm <sdk image> dotnet --list-runtimes` shows the runtime an SDK carries). Pinning by digest as
# well (tag@sha256:...) is possible: `docker buildx imagetools inspect <image>` prints it.
#
# Node comes from the official node image (copied in below) instead of a downloaded setup script, and is
# pinned with it. The node image is Debian and the SDK image is Ubuntu; Node needs only an older glibc than
# Ubuntu's, so the copied binary runs (the `node --version` below proves it on every build).
#
# Node and the SDK run on the build machine's own architecture ($BUILDPLATFORM) and cross-compile for the
# target, so an arm64 image builds on an x64 machine. .NET does not support running under QEMU emulation,
# which is what buildx would otherwise use for the build stage. Only the runtime stage is the target's.

# -------- Stage 0: Node ------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:24.22.2-bookworm-slim AS node

# -------- Stage 1: build ---------------------------------------------------
FROM --platform=$BUILDPLATFORM mcr.microsoft.com/dotnet/sdk:10.0.403-noble AS build
ARG BUILD_CONFIGURATION=Release

# Toolchain: Node 24 (from the node stage) + pnpm 9.15.0.
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && node --version \
 && npm install -g pnpm@9.15.0

WORKDIR /build

# Workspace and .NET props first, so dependency layers cache across source edits.
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY ui/react-ui/package.json ui/react-ui/
COPY ui/app/package.json ui/app/
RUN pnpm install --frozen-lockfile

COPY global.json Directory.Build.props Directory.Packages.props .editorconfig ./
COPY Koppel.Core ./Koppel.Core
COPY Koppel ./Koppel
COPY ui ./ui

# NuGet restores from the packages.lock.json files and fails if they have drifted (RestoreLockedMode). The
# lock files cover every platform the host ships on, so the restore takes no -r. PublishReadyToRun is on the
# restore too, so it fetches the ahead-of-time compiler pack the publish needs.
RUN dotnet restore Koppel/src/Koppel/Koppel.csproj -p:RestoreLockedMode=true -p:PublishReadyToRun=true --nologo

# -------- Stage 2: publish for the image ----------------------------------
FROM build AS publish
ARG BUILD_CONFIGURATION=Release

# PublishRunWebpack builds the SPA and stages dist/ into wwwroot/. ReadyToRun compiles ahead of time:
# without it a low-power thin client spends seconds compiling on its first request (8 to 14 s measured
# on an AMD GX-212JC), long enough for a client's first token or pairing request to time out.
# An architecture this does not know fails the build, rather than quietly building x64.
ARG TARGETARCH
RUN case "$TARGETARCH" in       amd64) RID=linux-x64 ;;       arm64) RID=linux-arm64 ;;       *) echo "Unsupported TARGETARCH '$TARGETARCH': build with BuildKit (docker buildx build or compose) for linux/amd64 or linux/arm64." >&2; exit 1 ;;     esac  && dotnet publish Koppel/src/Koppel/Koppel.csproj -c $BUILD_CONFIGURATION -o /publish --nologo --no-restore       -r $RID --self-contained false -p:PublishReadyToRun=true -p:SkipSpaInstall=true

# The volume's folders are made here and copied below, because the runtime stage has no RUN: a RUN there would
# execute the target's binaries and so need emulation on a machine of another architecture.
RUN mkdir -p /koppel/config /koppel/keys

# -------- Linux packages (not part of the image) -----------------------------
# Built only on request, through installer/linux/build.ps1 (--target linux-packages). Koppel is published
# self-contained, so the machine needs no .NET, for amd64 and arm64, and nfpm wraps each in a .deb with the
# files in installer/linux.
FROM --platform=$BUILDPLATFORM goreleaser/nfpm:v2.48.0 AS nfpm

FROM build AS deb
ARG BUILD_CONFIGURATION=Release
COPY --from=nfpm /usr/bin/nfpm /usr/local/bin/nfpm
WORKDIR /build/Koppel/installer/linux
RUN VERSION="$(dotnet msbuild /build/Koppel/src/Koppel/Koppel.csproj -getProperty:Version)" && export VERSION  && for ARCH in amd64 arm64; do       case "$ARCH" in amd64) RID=linux-x64 ;; arm64) RID=linux-arm64 ;; esac;       rm -rf /stage       && dotnet publish /build/Koppel/src/Koppel/Koppel.csproj -c $BUILD_CONFIGURATION -o /stage --nologo --no-restore            -r $RID --self-contained -p:PublishReadyToRun=true -p:SkipSpaInstall=true       && mkdir -p /packages       && ARCH=$ARCH nfpm package --config nfpm.yaml --packager deb --target /packages/       || exit 1;     done

FROM scratch AS linux-packages
COPY --from=deb /packages/ /

# -------- Stage 3: runtime -------------------------------------------------
FROM mcr.microsoft.com/dotnet/aspnet:10.0.14-noble AS final

WORKDIR /app
COPY --from=publish /publish ./

# One volume root for everything Koppel writes.
COPY --from=publish --chown=$APP_UID /koppel /koppel
VOLUME /koppel
USER $APP_UID

# 5240: the UI and everything other Koppel machines do, USB traffic included (pinned by clients), HTTPS with
# the server's own self-signed certificate; discovery on UDP. Ports__Network and Discovery__Port split them.
EXPOSE 5240 5240/udp

ENV Paths__Config=/koppel/config
ENV Paths__Keys=/koppel/keys

# The anonymous GET /healthz on the UI port, probed at the configured Ports__Address and Ports__Ui and then
# at 127.0.0.1 (see Web/HealthProbe.cs). It is Koppel itself because the port is HTTPS and the image has no curl
# or openssl. The timeout leaves room for the runtime to start on a slow thin client. Plain Docker only reports
# "unhealthy": it does not restart the container (restart: unless-stopped acts on exits, not on health). Swarm
# and Kubernetes act on it; with plain Docker, something has to watch it, for example an autoheal container, or
# `docker ps --filter health=unhealthy`.
HEALTHCHECK --interval=30s --timeout=10s --start-period=40s --retries=3 \
  CMD ["dotnet", "/app/Koppel.dll", "--healthcheck"]

ENTRYPOINT ["dotnet", "Koppel.dll"]
