# Hyperion

My personal SSH, postgres, network, and file server. It is also my home gateway (router and
firewall) and runs the UniFi controller.

- `src/` mirrors `/` on the server, and `./install` symlinks each file into place.
- `webroot/` is served by nginx straight from the clone.
- `test/` runs the router config in a container.
- `old_freebsd_port_configs/` holds configs from the FreeBSD days.

## Installation

```sh
git clone https://github.com/gregschmit/hyperion.git /srv/hyperion
/srv/hyperion/install
```

Clone to `/srv/hyperion`, since nginx serves `webroot/` from there. Keep the clone on the root
filesystem and out of `/home` and `/root`: services read their configs through the links early in
boot, and nftables and networkd can't see `/home` or `/root`.

Besides linking the files in `src/`, `./install` installs the packages in `pkglist` that are
missing, and UniFi OS Server (the latest release, from Ubiquiti, checked against its SHA-256; it
updates itself after that). nginx comes from nginx.org (see `src/etc/apt`), for its ACME module:
nginx gets and renews a Let's Encrypt certificate for each HTTPS site by itself, using the site's
`server_name`. The names must point at this server, and port 80 must reach it. Until a site has
its certificate, its HTTPS connections fail and `/var/log/nginx/error.log` says why; the other
sites work. The certificates live in `/var/cache/nginx/acme-letsencrypt`.

On its first run, `./install` hands networking from ifupdown (Debian's default) to
systemd-networkd and turns on the firewall. That, like new NIC names in the `.link` files, takes
effect at the next boot, so the script asks to reboot. For a new server, see the first-time setup
below.

Re-run `./install` (as root) after each pull. It links files one by one, so files that packages
put in the same directories stay out of the repo, and it removes links to files that are gone from
`src/`. To link a whole directory instead, add it to `LINK_DIRS` in the script. Package upgrades
leave linked configs alone and put the package's version beside the link (e.g.
`/etc/nftables.conf.dpkg-new`); check it for useful changes, then delete it.

Then apply the changes:

| Changed              | Apply with                                                              |
| -------------------- | ----------------------------------------------------------------------- |
| `nftables.conf`      | `nft -c -f /etc/nftables.conf && systemctl reload-or-restart nftables`  |
| `*.network`          | `networkctl reload`                                                     |
| `*.link`             | reboot                                                                  |
| `dnsmasq.d/lan.conf` | `dnsmasq --test --conf-dir=/etc/dnsmasq.d && systemctl restart dnsmasq` |
| `systemd/system/*`   | `systemctl daemon-reload`                                               |
| `nginx/*`            | `nginx -t && systemctl reload nginx`                                    |
| `postgresql/*`       | `systemctl reload postgresql` (restart it for `listen_addresses`)       |
| `redis/*`            | `systemctl restart redis-server`                                        |

## Router

```
Metronet (IPv4) --- wan4 --+
                           +-- hyperion --- lan --- 172.31.0.0/24, 2603:300a:2161:ca00::/64
Comcast (IPv6) ---- wan6 --+
```

- `wan4` has the static `209.173.223.178/29` (gateway `.177`), and the LAN shares it through NAT.
- `wan6` takes its route from router advertisements, and asks for the delegated prefix with
  DHCPv6-PD, so Comcast routes it here: `2603:300a:2161:ca00::/56` and `cb00::/56`.
- `lan` is `172.31.0.1/24` and `2603:300a:2161:ca00::1/64`, a fixed /64 of that prefix with no
  NAT. dnsmasq leases `.100`-`.254`, serves DNS (with DHCP clients named under `home.arpa`), and
  sends the router advertisements. IPv6 hosts use SLAAC or a static address; there is no DHCPv6.
  Keep `.2`-`.99` for static lab hosts and fixed DHCP hosts (the rXg has `.23`), and give hosts
  that need a fixed public IPv6 address `::2`-`::9` (see `20-lan.network`); dnsmasq doesn't check
  those for clashes.
- The firewall lets the LAN reach anything. From the internet, only SSH, HTTP, HTTPS, and ping
  reach the router, plus Postgres and Redis for trusted clients: the `trusted_clients` sets in
  `nftables.conf`, for now my own address blocks (`209.173.223.176/29` and
  `2603:300a:2161:ca00::/55`). Nothing reaches the LAN unless it answers a connection from the
  LAN. The router only routes while the firewall is loaded.
- Postgres and Redis listen on all addresses and leave the choice of clients to the firewall.
  Network clients need TLS with Debian's self-signed certificate: `sslmode=require` for Postgres,
  and `rediss://` without verification (`redis-cli --tls --insecure`) for Redis. Postgres also
  needs a password; Redis has none.
- UniFi OS Server (the UniFi controller) runs in rootless podman as the `uosserver` user; manage it
  with the `uosserver` command. Its ports are open to the LAN only. LAN clients reach its web UI at
  `https://unifi.schmit.net` through nginx (or at `https://172.31.0.1:11443`), and UniFi devices
  find it by the name `unifi` to adopt. The certificate needs `unifi.schmit.net` in public DNS
  (e.g. a CNAME to `schmit.net`), though nginx serves the name to the LAN only.

The configs live in `src/etc`: `systemd/network/` (interface names and addresses), `nftables.conf`
(firewall and NAT), `systemd/system/nftables.service.d/` (forwarding), and `dnsmasq.d/lan.conf`
(DHCP, DNS, and router advertisements).

### First-Time Setup

On a fresh Debian install:

1. Connect each NIC to its network by the MAC address in its `.link` file in
   `src/etc/systemd/network/` (on Proxmox, put each network device on the right bridge): `wan4` to
   Metronet, `wan6` to Comcast, and `lan` to the LAN switch.
2. Clone the repo and run `./install` (see above). If it warns that no NIC has a MAC address from
   a `.link` file, fix the file and re-run it.
3. Take the old router off the LAN, or turn off its DHCP server and router advertisements if it
   stays as a switch or access point.
4. In public DNS, point `schmit.net` at `209.173.223.178`, and `www`, `hyperion`, `files`, and
   `unifi` at `schmit.net` (e.g. with CNAMEs). nginx needs these names for its certificates.
5. Reboot (`./install` offers to, or run `reboot`). The server comes back at `209.173.223.178`
   and, from the LAN, `172.31.0.1`. Then restart the LAN switch, so clients get new addresses now
   instead of in a few hours.
6. Check that:
   - `networkctl` shows all three links configured. If an uplink gets no address or no route,
     power-cycle its modem.
   - `journalctl -u systemd-networkd | grep "delegated prefix"` shows `2603:300a:2161:ca00::/56`.
     Comcast may tie the prefix to the old router's DHCP identity (its DUID) and delegate another
     one. If it does, change the prefix in `20-lan.network` (and the IPv6 example in
     `nftables.conf`), or give `wan6` the old router's DUID (`DUIDRawData=` in `[DHCPv6]`).
   - `nft list ruleset` and `journalctl -u dnsmasq` look right, and leases show up in
     `/var/lib/misc/dnsmasq.leases`.
   - `dig +short schmit.net` shows `209.173.223.178`, and each site has a certificate in
     `/var/cache/nginx/acme-letsencrypt`.

### Common Changes

- Use one uplink for both IPv4 and IPv6: see the top of `20-wan4.network`.
- Give a host a fixed address or boot options: add a `dhcp-host` line to `lan.conf`, like the rXg.
- Name a static lab host: add a `host-record` line to `lan.conf` (dnsmasq skips `/etc/hosts`).
- Open a port on the router: add it to `tcp dport { 22, 80, 443 }` in `nftables.conf`.
- Forward a port to the LAN, or open an IPv6 server: see the commented rules in `nftables.conf`.
- Open the UniFi web UI to the internet: remove the `allow` and `deny` lines in
  `nginx/conf.d/unifi.schmit.net.conf`.

## Testing

`test/run` boots the router config in a Debian container, with fake ISPs and LAN clients in
network namespaces, and checks DHCP, SLAAC, DNS, NAT, and the firewall. It needs Docker.
`KEEP=1 test/run` leaves the container running to look around in, and `REPO=<path> test/run`
checks another copy of the repo.

## Security

Always check the following after fresh install or dist upgrade:
- Ensure `PasswordAuthentication no` is set in `/etc/ssh/sshd_config`.
- Ensure `/etc/nftables.conf` links to `src/etc/nftables.conf`, and `nftables.service` is enabled
  and active.
- The postgres password is in `/root/postgres_pw.txt`. `./install` sets it when the file is
  missing or the password no longer works, so to rotate it, delete the file and run `./install`.
- Redis has no password, so only the LAN and the `trusted_clients` sets in `nftables.conf` may
  reach it.

## OpenRGB Config on Physical Servers:

```sh
root@big-red:~# cat /etc/systemd/system/openrgb.service
[Service]
Type=oneshot
ExecStart=/usr/bin/openrgb -c ff0000 -m Direct

[Install]
WantedBy=multi-user.target
```
