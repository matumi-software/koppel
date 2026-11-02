#!/usr/bin/env bash
# Docker HEALTHCHECK for the Koppel image: GET /healthz on the UI port, anonymous. The image has no curl, so bash
# opens the connection itself.
#
# It asks where the UI is configured to listen (Ports__HttpAddress and Ports__Http, the same settings the host
# reads), and then on 127.0.0.1, so a container bound to one LAN address is not unhealthy forever just because
# nothing listens on loopback. "Any address" (unset, 0.0.0.0 or ::) is reached through loopback.
port="${Ports__Http:-5240}"
address="${Ports__HttpAddress:-127.0.0.1}"
case "$address" in
    0.0.0.0 | :: | '[::]') address=127.0.0.1 ;;
esac

for host in "$address" 127.0.0.1; do
    exec 3<>"/dev/tcp/$host/$port" || continue
    printf 'GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n' >&3
    read -r status <&3
    [[ $status == *' 200 '* ]]
    exit
done
exit 1
