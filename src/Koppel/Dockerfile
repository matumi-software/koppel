# Multi-stage build for Koppel. Build context = repo root (see docker-compose.yml).
#
# One image serves both roles; the role is a setting. USB access (privileged mode,
# /dev/bus/usb, host kernel modules) is added in compose when the USB layer lands.

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
COPY src/Koppel.React.UI/package.json src/Koppel.React.UI/
COPY src/Koppel.UI/package.json src/Koppel.UI/
RUN pnpm install --frozen-lockfile

COPY global.json Directory.Build.props Directory.Packages.props .editorconfig ./
COPY src ./src

# PublishRunWebpack builds the SPA and stages dist/ into wwwroot/.
RUN dotnet publish src/Koppel/Koppel.csproj -c $BUILD_CONFIGURATION -o /publish --nologo -p:SkipSpaInstall=true

# -------- Stage 2: runtime -------------------------------------------------
FROM mcr.microsoft.com/dotnet/aspnet:10.0 AS final

WORKDIR /app
COPY --from=build /publish ./

# One volume root for everything Koppel writes.
RUN mkdir -p /koppel/config && chown -R $APP_UID /koppel
VOLUME /koppel
USER $APP_UID

EXPOSE 5240

ENV ASPNETCORE_URLS=http://+:5240
ENV Paths__Config=/koppel/config

ENTRYPOINT ["dotnet", "Koppel.dll"]
