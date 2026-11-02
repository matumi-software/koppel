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

# -------- Stage 0: Node ------------------------------------------------------
FROM node:24.22.2-bookworm-slim AS node

# -------- Stage 1: build ---------------------------------------------------
FROM mcr.microsoft.com/dotnet/sdk:10.0.403-noble AS build
ARG BUILD_CONFIGURATION=Release

# Toolchain: Node 24 (from the node stage) + pnpm 9.15.0, as in the Timebase images.
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && node --version \
 && npm install -g pnpm@9.15.0

WORKDIR /build

# Workspace and .NET props first, so dependency layers cache across source edits.
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY Koppel.Core/src/Koppel.Core.React.UI/package.json Koppel.Core/src/Koppel.Core.React.UI/
COPY Koppel/src/Koppel.UI/package.json Koppel/src/Koppel.UI/
RUN pnpm install --frozen-lockfile

COPY global.json Directory.Build.props Directory.Packages.props .editorconfig ./
COPY Koppel.Core ./Koppel.Core
COPY Koppel ./Koppel

# NuGet restores from the packages.lock.json files and fails if they have drifted (RestoreLockedMode). The
# lock files cover every platform the host ships on, so the restore takes no -r. PublishReadyToRun is on the
# restore too, so it fetches the ahead-of-time compiler pack the publish needs.
RUN dotnet restore Koppel/src/Koppel/Koppel.csproj -p:RestoreLockedMode=true -p:PublishReadyToRun=true --nologo

# PublishRunWebpack builds the SPA and stages dist/ into wwwroot/. ReadyToRun compiles ahead of time:
# without it a low-power thin client spends seconds compiling on its first request (8 to 14 s measured
# on an AMD GX-212JC), long enough for a client's first token or pairing request to time out.
# An architecture this does not know fails the build, rather than quietly building x64.
ARG TARGETARCH
RUN case "$TARGETARCH" in \
      amd64) RID=linux-x64 ;; \
      arm64) RID=linux-arm64 ;; \
      *) echo "Unsupported TARGETARCH '$TARGETARCH': build with BuildKit (docker buildx build or compose) for linux/amd64 or linux/arm64." >&2; exit 1 ;; \
    esac \
 && dotnet publish Koppel/src/Koppel/Koppel.csproj -c $BUILD_CONFIGURATION -o /publish --nologo --no-restore \
      -r $RID --self-contained false -p:PublishReadyToRun=true -p:SkipSpaInstall=true

# -------- Stage 2: runtime -------------------------------------------------
FROM mcr.microsoft.com/dotnet/aspnet:10.0.14-noble AS final

WORKDIR /app
COPY --from=build /publish ./

# One volume root for everything Koppel writes.
RUN mkdir -p /koppel/config /koppel/keys && chown -R $APP_UID /koppel
VOLUME /koppel
USER $APP_UID

# 5240: the UI (HTTP). 5241: token endpoint and remote API (HTTPS, self-signed, pinned by clients).
# 3240: USB traffic (TLS, token-authenticated).
EXPOSE 5240 5241 3240

ENV Paths__Config=/koppel/config
ENV Paths__Keys=/koppel/keys

# The anonymous GET /healthz on the UI port, probed at the configured Ports__HttpAddress and Ports__Http and then
# at 127.0.0.1 (see healthcheck.sh). Plain Docker only reports "unhealthy": it does not restart the container
# (restart: unless-stopped acts on exits, not on health). Swarm and Kubernetes act on it; with plain Docker,
# something has to watch it, for example an autoheal container, or `docker ps --filter health=unhealthy`.
COPY Koppel/src/Koppel/healthcheck.sh /app/healthcheck.sh
HEALTHCHECK --interval=30s --timeout=5s --start-period=40s --retries=3 \
  CMD ["bash", "/app/healthcheck.sh"]

ENTRYPOINT ["dotnet", "Koppel.dll"]
