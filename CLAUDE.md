# Koppel

USB devices shared over the network. Read `docs/architecture.md` first.

## Layout

| Folder | What |
|---|---|
| `docs/` | Architecture and decisions |
| `resources/design-system/` | The design system: brand book, `tokens.json`, component specs and previews, logos. The source of every colour, size and component style |
| `src/Koppel/` | ASP.NET Core host: REST, SignalR, config store. Serves the built UI |
| `src/Koppel.UI/` | The React app (Vite) |
| `src/Koppel.React.UI/` | The UI component library (`@koppel/react-ui`), built on react-aria-components |
| `tests/` | .NET tests. UI library tests sit next to the components (`*.test.tsx`) |
| `tools/` | Repo scripts |

## Rules

- **The UI never assumes a change worked.** Send it over REST, show it as pending, and treat it as done only when the SignalR push brings the value back after the server wrote its config file. See "The confirm rule" in `docs/architecture.md`.
- **Design values come from `resources/design-system/`.** Do not hand-edit `src/Koppel.React.UI/src/styles/tokens.css` or `components.css`; change the design system and run `pnpm design:sync`.
- **No new UI primitives in the app.** Add them to `@koppel/react-ui` and to the design system.
- **MIT licence.** Do not copy code from GPL projects.
- C#: file-scoped namespaces, internal by default, `[LoggerMessage]` for logging. Tests are named `Method_does_thing`.

## Commands

```sh
pnpm install                                  # repo root, once
pnpm design:sync                              # after changing resources/design-system
dotnet build Koppel.slnx
dotnet test tests/Koppel.Tests/Koppel.Tests.csproj
pnpm --filter @koppel/react-ui test           # component tests
pnpm --filter koppel.ui lint
pnpm --filter koppel.ui build
dotnet run --project src/Koppel               # starts the Vite dev server through SpaProxy
docker compose up --build                     # http://localhost:5240
```
