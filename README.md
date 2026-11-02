# Koppel

Share USB devices across the network. Plug a device into one machine and use it on another.

Koppel speaks the standard USB/IP protocol and adds a web UI, authentication and encrypted transport
around it. It runs as a server, a client or both, on Linux (Docker) and Windows.

Status: early development. The control plane (host, UI, live settings) is in place; the USB layer
and authentication are next. Work happens on `dev`; `main` holds releases.

## Run it

```sh
docker compose up --build
```

Then open http://localhost:5240.

For development, see the commands in [CLAUDE.md](CLAUDE.md) and the design in
[docs/architecture.md](docs/architecture.md).

## License

MIT, see [LICENSE](LICENSE).
