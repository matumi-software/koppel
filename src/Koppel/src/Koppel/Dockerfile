# Multi-stage build for Koppel. Build context = src/ (see Koppel/docker-compose.yml).
#
# One image serves both roles; the role is a setting. USB needs a privileged container running as
# root and the host's usbip-host / vhci-hcd modules; see docker-compose.yml.

# -------- Stage 1: build ---------------------------------------------------
FROM mcr.microsoft.com/dotnet/sdk:10.0 AS build
ARG BUILD_CONFIGURATION=Release

# Toolchain: Node 24 (NodeSource) + pnpm 9.15.0, as in the Timebase images.
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates gnupg \
 && curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && npm install -g pnpm@9.15.0 \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Workspace and .NET props first, so dependency layers cache across source edits.
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml ./
COPY Koppel.Core/src/Koppel.Core.React.UI/package.json Koppel.Core/src/Koppel.Core.React.UI/
COPY Koppel/src/Koppel.UI/package.json Koppel/src/Koppel.UI/
RUN pnpm install --frozen-lockfile

COPY global.json Directory.Build.props Directory.Packages.props .editorconfig ./
COPY Koppel.Core ./Koppel.Core
COPY Koppel ./Koppel

# PublishRunWebpack builds the SPA and stages dist/ into wwwroot/. ReadyToRun compiles ahead of time:
# without it a low-power thin client spends seconds compiling on its first request (8 to 14 s measured
# on an AMD GX-212JC), long enough for a client's first token or pairing request to time out.
ARG TARGETARCH
RUN RID=$([ "$TARGETARCH" = "arm64" ] && echo linux-arm64 || echo linux-x64) \
 && dotnet publish Koppel/src/Koppel/Koppel.csproj -c $BUILD_CONFIGURATION -o /publish --nologo \
      -r $RID --self-contained false -p:PublishReadyToRun=true -p:SkipSpaInstall=true

# -------- Stage 2: runtime -------------------------------------------------
FROM mcr.microsoft.com/dotnet/aspnet:10.0 AS final

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

ENTRYPOINT ["dotnet", "Koppel.dll"]
