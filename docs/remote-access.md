# Remote access

The tablet runs Tailscale in userspace mode because the reMarkable kernel does
not expose `/dev/net/tun` or a WireGuard module. Access remains private to the
tailnet.

## Endpoints

Keep the device hostname outside the repository. Set it to either the tablet's
local address or its private Tailscale name:

```sh
export REMARQUE_HOST='<local IP or private MagicDNS name>'
ssh "root@$REMARQUE_HOST"
```

The Remarque screen is available at `http://$REMARQUE_HOST:7432` while the
application is running. Direct LAN access is unauthenticated; use it only on a
trusted network.

Tailscale Serve forwards tailnet TCP port 22 to a loopback-only Dropbear socket
on port 2222. It can also forward port 7432 to Remarque.

## Tablet paths

- Binaries and state: `/home/root/tailscale`
- Daemon unit: `/etc/systemd/system/tailscale-userspace.service`
- SSH socket unit: `/etc/systemd/system/dropbear-tailscale.socket`
- SSH service template: `/etc/systemd/system/dropbear-tailscale@.service`

Inspect the connection with:

```sh
/home/root/tailscale/tailscale \
  --socket=/home/root/tailscale/tailscaled.sock status
/home/root/tailscale/tailscale \
  --socket=/home/root/tailscale/tailscaled.sock serve status
```

Disable either forward with `tailscale serve --tcp=22 off` or
`tailscale serve --tcp=7432 off`, passing the same socket option.
