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

After linking, `./install` installs the packages in `pkglist` that are missing, and UniFi OS
Server (the latest release, from Ubiquiti, checked against its SHA-256; it updates itself after
that). nginx comes from nginx.org (see `src/etc/apt`), for its ACME module: nginx gets and renews
a Let's Encrypt certificate for each HTTPS site by itself, using the site's `server_name`. The
names must point at this server, and port 80 must reach it. Until a site has its certificate, its
HTTPS connections fail and `/var/log/nginx/error.log` says why; the other sites work. The
certificates live in `/var/cache/nginx/acme-letsencrypt`.

`./install` also puts the router configs in place, and they take effect at the next boot. On a
box that isn't the router yet, follow the first-time setup below instead.

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
| `postgresql/*`       | `./install` (reloads postgres, and restarts it if a setting needs that) |
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
  Network clients need a password and TLS with Debian's self-signed certificate: `sslmode=require`
  for Postgres, and `rediss://` without verification (`redis-cli --tls --insecure`) for Redis.
- UniFi OS Server (the UniFi controller) runs in rootless podman as the `uosserver` user; manage it
  with the `uosserver` command. Its ports are open to the LAN only. LAN clients reach its web UI at
  `https://unifi.schmit.net` through nginx (or at `https://172.31.0.1:11443`), and UniFi devices
  find it by the name `unifi` to adopt. The certificate needs `unifi.schmit.net` in public DNS
  (e.g. a CNAME to `schmit.net`), though nginx serves the name to the LAN only.

The configs live in `src/etc`: `systemd/network/` (interface names and addresses), `nftables.conf`
(firewall and NAT), `systemd/system/nftables.service.d/` (forwarding), and `dnsmasq.d/lan.conf`
(DHCP, DNS, and router advertisements).

### First-Time Setup

1. Put each NIC's MAC address (see `ip -br link` on the server) in
   `src/etc/systemd/network/10-*.link`, and commit and push it.
2. At the server's console (the network drops when it switches over), pull and run `./install`
   (move an old clone from `/srv/schmit.net` to `/srv/hyperion` first). apt replaces Debian's nginx
   with nginx.org's. Then check the names the NICs will get. Expect `wan4`, `wan6`, and `lan`:

   ```sh
   for nic in /sys/class/net/*; do
     udevadm test-builtin net_setup_link "$nic" 2>/dev/null | grep ID_NET_NAME=
   done
   ```

3. Unplug `lan` if it's on the old router's LAN. Then move networking from ifupdown to
   systemd-networkd, turn on the firewall, and reboot (if NetworkManager is installed,
   `apt purge network-manager` first):

   ```sh
   mv /etc/network/interfaces /etc/network/interfaces.save
   systemctl disable networking
   systemctl enable systemd-networkd nftables
   reboot
   ```

4. Point the router's own DNS at dnsmasq, with Cloudflare as a fallback. (Before the reboot,
   ifupdown's DHCP client would have overwritten this.)

   ```sh
   printf 'nameserver %s\n' 127.0.0.1 1.1.1.1 2606:4700:4700::1111 > /etc/resolv.conf
   ```

5. Take the old router off the LAN, or turn off its DHCP server and router advertisements if it
   stays as a switch or access point. Move the ISP cables to `wan4` and `wan6`, plug in `lan`, and
   restart the LAN switch, so clients get new addresses now instead of in a few hours.
6. Check that:
   - `networkctl` shows all three links configured. If an uplink gets no address or no route,
     power-cycle its modem.
   - `journalctl -u systemd-networkd | grep "delegated prefix"` shows `2603:300a:2161:ca00::/56`.
     Comcast may tie the prefix to the old router's DHCP identity (its DUID) and delegate another
     one. If it does, change the prefix in `20-lan.network` (and the IPv6 example in
     `nftables.conf`), or give `wan6` the old router's DUID (`DUIDRawData=` in `[DHCPv6]`).
   - `nft list ruleset` and `journalctl -u dnsmasq` look right, and leases show up in
     `/var/lib/misc/dnsmasq.leases`.
   - schmit.net points at `209.173.223.178` (`dig +short schmit.net`), and each site has a
     certificate in `/var/cache/nginx/acme-letsencrypt`.
7. Once it all works, remove the old tools:
   `apt purge ifupdown dhcpcd-base isc-dhcp-client certbot python3-certbot-nginx`.

To go back, run this at the console (after step 7, first `apt install ifupdown` while the network
still works). Until you try again, don't run `./install`: it puts the `.link` files back, and the
next boot renames the NICs.

```sh
systemctl disable systemd-networkd nftables
systemctl enable networking
rm /etc/systemd/network/10-*.link
mv /etc/network/interfaces.save /etc/network/interfaces
reboot
```

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
- The postgres and redis passwords are in `/root/postgres_pw.txt` and `/root/redis_pw.txt`.
  `./install` sets each one when its file is missing or no longer works, so to rotate one, delete
  its file and run `./install`.

## OpenRGB Config on Physical Servers:

```sh
root@big-red:~# cat /etc/systemd/system/openrgb.service
[Service]
Type=oneshot
ExecStart=/usr/bin/openrgb -c ff0000 -m Direct

[Install]
WantedBy=multi-user.target
```
