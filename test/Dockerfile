# The router in a container, for test/run. The repo is mounted at /srv/hyperion at run time,
# so config changes need no rebuild.
FROM debian:trixie

# The router's packages (see pkglist; systemd includes systemd-networkd, and procps has the sysctl
# that the nftables drop-in runs), then the fake ISPs' servers and the tools that test/check uses.
COPY pkglist /tmp/pkglist
RUN apt-get update \
  && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    $(grep -v '^#' /tmp/pkglist) systemd systemd-sysv dbus udev procps \
    kea-dhcp6-server radvd busybox ndisc6 socat tcpdump bind9-dnsutils iproute2 iperf3 \
  && rm -rf /var/lib/apt/lists/*

# Start the router's services at boot. Kea and radvd only play the IPv6 ISP, in a namespace.
RUN systemctl enable systemd-networkd nftables dnsmasq \
  && systemctl disable kea-dhcp6-server radvd

# The container is privileged, so it can reach the host's consoles and kernel-wide settings. Keep
# logins off the consoles, and mask the packages' sysctl files, which set kernel.* values. Our
# own net.* values only apply in the container's network namespace.
RUN systemctl mask getty.target systemd-logind \
  && mkdir -p /etc/sysctl.d \
  && for f in /usr/lib/sysctl.d/*.conf; do ln -s /dev/null "/etc/sysctl.d/${f##*/}"; done

# Keep the journal in /run, so it starts empty on a container restart (which keeps the kernel's
# boot ID), and skip the kernel log, which holds the whole Docker VM's messages.
RUN mkdir -p /etc/systemd/journald.conf.d \
  && printf '[Journal]\nStorage=volatile\nReadKMsg=no\n' >/etc/systemd/journald.conf.d/test.conf

# Link the configs before systemd starts, like ./install on the real router. Make /sys read-only
# first, as systemd expects in a container: that keeps udev off, which would otherwise act on the
# host's devices. And unmount Docker's /etc/resolv.conf, which install links to the repo's.
ENTRYPOINT ["/bin/sh", "-c", \
  "mount -o remount,ro /sys && umount /etc/resolv.conf && /srv/hyperion/install && exec /sbin/init"]
STOPSIGNAL SIGRTMIN+3
