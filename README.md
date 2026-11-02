# Koppel

Share USB devices across the network. Plug a device into one machine and use it on another.

Koppel speaks the standard USB/IP protocol and adds a web UI, authentication and encrypted transport
around it. It runs as a server, a client or both, on Linux (Docker) and Windows.

Status: early development. Linux servers and clients can share and attach devices over an
encrypted, authenticated link; this has not yet been tried on real USB hardware. Windows support
comes later. Work happens on `dev`; `main` holds releases.

## Run it

```sh
cd src/Koppel
docker compose up --build
```

Then open http://localhost:5240.

For development, see the commands in [CLAUDE.md](CLAUDE.md) and the design in
[docs/architecture.md](docs/architecture.md).

## License

MIT, see [LICENSE](LICENSE).
