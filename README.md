# Hyperion

My personal SSH, postgres, network, and file server. It is also my home gateway (router and
firewall) and runs the UniFi controller.

## Installation

On a fresh Debian 13 install with a `gns` user, connect each NIC to its network by the MAC address
in its `.link` file: `wan4` to Metronet, `wan6` to Comcast, and `lan` to the LAN switch, which must
carry the guest networks (VLANs 20 and 21) and the old rXg's network (VLAN 10), tagged. (On
Proxmox, put each network device on the right bridge, and give `lan`'s no VLAN tag.)

`gns` needs sudo. The Debian installer sets that up if you leave the root password empty; if you
set one, run `su -c 'apt-get install -y sudo && usermod -aG sudo gns'`, and log in again. Then run
this as `gns`:

```sh
sudo apt-get update && sudo apt-get install -y git &&
  sudo install -d -o gns -g gns /srv/hyperion &&
  git clone https://github.com/gregschmit/hyperion.git /srv/hyperion &&
  sudo /srv/hyperion/install
```

Keep the clone at `/srv/hyperion`: nginx serves `webroot/` from there, and services can't read
configs linked from `/home` or `/root`. `install` links each file in `src/` to the same path under
`/`, installs packages and UniFi OS Server, moves networking to systemd-networkd, and offers to
reboot. If it warns that no NIC has a MAC address from a `.link` file, fix the file and run it
again.

Before you reboot (decline `install`'s offer until then), take the old router off the LAN, and
point `schmit.net` and the subdomains that nginx serves at `209.173.223.178` in public DNS. After
the reboot, restart the LAN switch, so clients get new addresses now. In UniFi, add the guest
networks (VLAN only, with IDs 20 and 21), each with a WiFi network that has client device isolation
on. Then check that:

- `networkctl` shows every link configured. Power-cycle the modem of an uplink that gets no
  address.
- `ip -6 route show type unreachable proto dhcp` shows the delegated prefix,
  `2603:300a:2161:ca00::/56`. A smaller one (e.g. a /59) means the Comcast gateway routes IPv6
  itself instead of bridging it. Comcast may also tie the prefix to the old router's DHCP identity
  (its DUID) and delegate another. Then replace the prefix everywhere
  (`grep -r 2603:300a:2161 src`), or give `wan6` the old router's DUID (`DUIDRawData=` in
  `[DHCPv6]`).
- LAN clients and guests get addresses, and reach the internet over IPv6 (e.g.
  `curl -6 -I https://ipv6.google.com`). Guests don't reach the LAN.
- nginx got its certificates (see `/var/log/nginx/error.log`).

## Updating

As `gns`, in `/srv/hyperion`: `git pull && sudo ./install`. `install` doesn't reload or restart
anything, so apply changes with each service's own command. Package upgrades leave linked configs
alone and put the package's version beside the link (e.g. `/etc/nftables.conf.dpkg-new`); check it
for useful changes, then delete it.

To push from the server, add `gns`'s SSH key to GitHub, and run
`git remote set-url origin git@github.com:gregschmit/hyperion.git`.

## Testing

`test/run` boots the router config in a Debian container with fake ISPs, LAN clients, and guests,
and checks it. It needs Docker. `KEEP=1 test/run` leaves the container running, and
`REPO=<path> test/run` checks another copy of the repo.

Docker Desktop's kernel can't run `guest-shaper`, so `test/run` skips the bandwidth checks there.
To run them, use a Debian VM, e.g. with Lima:

```sh
brew install lima
limactl start --name=hyperion-test template:debian-13
limactl shell hyperion-test sudo apt-get install -y docker.io
limactl shell hyperion-test sudo test/run
```

## Security

After a fresh install or a dist upgrade, check that:

- `/etc/ssh/sshd_config` has `PasswordAuthentication no`.
- `/etc/nftables.conf` links into the repo, and `nftables.service` is enabled and active.

The Postgres password is in `/root/postgres_pw.txt`. `install` sets it when the file is missing or
the password no longer works, so to rotate it, delete the file and run `install`.

## OpenRGB Config on Physical Servers:

```sh
root@big-red:~# cat /etc/systemd/system/openrgb.service
[Service]
Type=oneshot
ExecStart=/usr/bin/openrgb -c ff0000 -m Direct

[Install]
WantedBy=multi-user.target
```
