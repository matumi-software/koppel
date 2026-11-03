# Koppel

USB devices shared over the network. Read `docs/architecture.md` first.

## Layout

The root stays clean: no code, no build files. Everything that builds lives under `src/`, one
folder per solution, each with its own `src/` and `tests/`.

| Folder | What |
|---|---|
| `docs/` | Architecture and decisions |
| `resources/design-system/` | The design system: brand book, `tokens.json`, component specs and previews, logos. The source of every colour, size and component style |
| `src/` | Shared build config: `Directory.Build.props`, `Directory.Packages.props`, `global.json`, `.editorconfig`, the pnpm workspace |
| `src/Koppel/` | The product: `Koppel.slnx`, the host (`src/Koppel`), the React app (`src/Koppel.UI`), its tests and `docker-compose.yml` |
| `src/Koppel.Core/` | The libraries: `Koppel.Core.slnx` with Configuration, Security, Auth and Usb, the UI library (`src/Koppel.Core.React.UI`, package `@koppel/react-ui`), their tests and `tools/` |

## Rules

- **The UI never assumes a change worked.** Send it over REST, show it as pending, and treat it as done only when the SignalR push brings the value back after the server wrote its config file. Pushes are applied in `HubPushes` and pending state lives in `PendingProvider`, above the pages. See "The confirm rule" in `docs/architecture.md`.
- **Design values come from `resources/design-system/`.** Do not hand-edit `Koppel.Core.React.UI/src/styles/tokens.css` or `components.css`; change the design system and run `pnpm design:sync`.
- **No new UI primitives in the app.** Add them to `@koppel/react-ui` and to the design system.
- **Every mutating fetch goes through `uiFetch`** (`api/http.ts`), which adds the `X-Koppel-UI` header the server requires. A route under `/api` on the UI port needs a session by default (`Surface.RequiresSignIn`); list it in `Surface.Anonymous` only when it must work signed out. A route for remote clients goes in `Surface.Listed` on the HTTPS listener. See "Ports and what they serve" in `docs/architecture.md`.
- **Reusable code goes in `Koppel.Core`**, product wiring in `Koppel`. Core never references the product.
- **MIT licence.** Do not copy code from GPL projects.
- C#: file-scoped namespaces, internal by default, `[LoggerMessage]` for logging. Tests are named `Method_does_thing`.

## Commands

Run from `src/`:

```sh
pnpm install                                         # once
pnpm design:sync                                     # after changing resources/design-system
dotnet build Koppel/Koppel.slnx
dotnet test Koppel/tests/Koppel.Tests/Koppel.Tests.csproj
dotnet test Koppel.Core/Koppel.Core.slnx                       # Core libraries (the sysfs part runs in Docker, below)
pnpm --filter @koppel/react-ui test                  # component tests
pnpm --filter koppel.ui test                           # app tests
pnpm --filter koppel.ui lint
pnpm --filter koppel.ui build
dotnet run --project Koppel/src/Koppel               # starts the Vite dev server through SpaProxy
docker compose -f Koppel/docker-compose.yml up --build   # https://localhost:5240
```

Packages restore from committed `packages.lock.json` files. After adding or changing a package, run `dotnet restore` on both solutions and commit the lock files; a locked build fails with NU1004 until you do. Restore without `-r` (every project restores for win-x64, linux-x64 and linux-arm64), then publish with `--no-restore -r <rid>`.

The sysfs tests create folders with `:` in their names, which Windows cannot, so part of them only
runs on Linux. Run the Core USB tests in a Linux container as well:

```sh
docker run --rm -v "$PWD:/repo:ro" mcr.microsoft.com/dotnet/sdk:10.0 bash -c \
  "cp -r /repo /work && find /work -type d \( -name bin -o -name obj -o -name node_modules \) -prune -exec rm -rf {} + && cd /work/Koppel.Core && dotnet test tests/Koppel.Core.Usb.Tests"
```
