//! Shell-based facts collection (Phase 4).
//!
//! [`collect`] runs a series of POSIX-ish commands on the target host and
//! parses the output into an Ansible-compatible facts map. Each command is
//! best-effort: a failure silently omits that fact.

use indexmap::IndexMap;

use crate::executors::Connection;

/// Collect system facts from the target host via shell commands.
///
/// Runs a series of POSIX-ish commands (`uname`, `hostname`,
/// `cat /etc/os-release`, `cat /proc/meminfo`, `nproc`, `ip`/hardware) and
/// parses their output into an Ansible-compatible facts map. Each command is
/// best-effort: a failure (non-zero exit, empty output, or parse error)
/// silently omits that fact — the collector never errors out.
///
/// Populated keys (when available):
/// - `ansible_system`      → e.g. `"Linux"`
/// - `ansible_kernel`      → e.g. `"6.1.0-1234"`
/// - `ansible_machine` / `ansible_architecture` → e.g. `"x86_64"`
/// - `ansible_hostname`    → short host name
/// - `ansible_fqdn`        → fully-qualified host name (omitted if unknown)
/// - `ansible_distribution`/`ansible_distribution_version`/`ansible_os_family`
/// - `ansible_memtotal_mb` → integer megabytes
/// - `ansible_processor_vcpus` → integer vCPU count
/// - `ansible_interfaces`  → JSON array of interface names
/// - `ansible_default_ipv4`→ `{ "interface": ..., "address": ... }`
///
/// # Errors
///
/// Never returns an error: every command failure is silently skipped.
///
/// # Panics
///
/// Never.
#[must_use]
pub fn collect(conn: &Connection<'_>) -> IndexMap<String, serde_json::Value> {
    let mut facts = IndexMap::new();

    if let Some(sys) = run_trim(conn, "uname -s") {
        facts.insert("ansible_system".to_string(), serde_json::Value::from(sys));
    }
    if let Some(kernel) = run_trim(conn, "uname -r") {
        facts.insert(
            "ansible_kernel".to_string(),
            serde_json::Value::from(kernel),
        );
    }
    if let Some(machine) = run_trim(conn, "uname -m") {
        facts.insert(
            "ansible_machine".to_string(),
            serde_json::Value::from(machine.clone()),
        );
        facts.insert(
            "ansible_architecture".to_string(),
            serde_json::Value::from(machine),
        );
    }

    if let Some(host) = run_trim(conn, "hostname").or_else(|| run_trim(conn, "uname -n")) {
        facts.insert(
            "ansible_hostname".to_string(),
            serde_json::Value::from(host),
        );
    }
    if let Some(fqdn) = run_trim(conn, "hostname -f") {
        facts.insert("ansible_fqdn".to_string(), serde_json::Value::from(fqdn));
    }

    if let Some(os_release) = run_trim(conn, "cat /etc/os-release") {
        if let Some(dist) = parse_os_release_field(&os_release, "NAME") {
            facts.insert(
                "ansible_distribution".to_string(),
                serde_json::Value::from(dist.clone()),
            );
            if let Some(family) = os_family(&dist) {
                facts.insert(
                    "ansible_os_family".to_string(),
                    serde_json::Value::from(family),
                );
            }
        }
        if let Some(version) = parse_os_release_field(&os_release, "VERSION_ID") {
            facts.insert(
                "ansible_distribution_version".to_string(),
                serde_json::Value::from(version),
            );
        }
    }

    if let Some(meminfo) = run_trim(conn, "cat /proc/meminfo")
        && let Some(mb) = parse_memtotal_mb(&meminfo)
    {
        facts.insert(
            "ansible_memtotal_mb".to_string(),
            serde_json::Value::from(mb),
        );
    }

    if let Some(nproc_out) = run_trim(conn, "nproc")
        && let Ok(vcpus) = nproc_out.parse::<u64>()
    {
        facts.insert(
            "ansible_processor_vcpus".to_string(),
            serde_json::Value::from(vcpus),
        );
    }

    if let Some(net_list) = run_trim(conn, "ls /sys/class/net") {
        let ifaces: Vec<serde_json::Value> = net_list
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(serde_json::Value::from)
            .collect();
        if !ifaces.is_empty() {
            facts.insert(
                "ansible_interfaces".to_string(),
                serde_json::Value::Array(ifaces),
            );
        }
    }

    if let Some(default_ipv4) = collect_default_ipv4(conn) {
        facts.insert(
            "ansible_default_ipv4".to_string(),
            serde_json::Value::Object(default_ipv4),
        );
    }

    facts
}

/// Build the `ansible_default_ipv4` object: resolve the egress interface for
/// `8.8.8.8`, then read its primary IPv4 address. Returns `None` if any step
/// fails.
#[must_use]
fn collect_default_ipv4(
    conn: &Connection<'_>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let route_out = run_trim(conn, "ip -4 route get 8.8.8.8 2>/dev/null")?;
    let iface = parse_route_iface(&route_out)?;
    let addr_out = run_trim(conn, &format!("ip -4 addr show {iface}"))?;
    let address = parse_inet_addr(&addr_out)?;
    let mut obj = serde_json::Map::new();
    obj.insert("interface".to_string(), serde_json::Value::from(iface));
    obj.insert("address".to_string(), serde_json::Value::from(address));
    Some(obj)
}

/// Run `cmd` on the host and return its trimmed stdout.
///
/// Returns `None` on host error, non-zero exit (`success == false`), or empty
/// output.
#[must_use]
fn run_trim(conn: &Connection<'_>, cmd: &str) -> Option<String> {
    let result = conn.run_command(cmd).ok()?;
    if !result.success {
        return None;
    }
    let s = result.stdout.as_str().trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Parse an `/etc/os-release` field (`NAME`, `VERSION_ID`, ...): the first
/// `KEY=value` line, with surrounding quotes stripped from `value`.
#[must_use]
fn parse_os_release_field(text: &str, key: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        let cleaned = value.trim().trim_matches(|c| c == '"' || c == '\'');
        if !cleaned.is_empty() {
            return Some(cleaned.to_string());
        }
    }
    None
}

/// Map a distribution name (`NAME=` value) to an Ansible `os_family`.
/// Returns `None` for unrecognized distributions (the fact is then omitted).
#[must_use]
fn os_family(distribution: &str) -> Option<&'static str> {
    let family = match distribution.trim() {
        "Debian" | "Ubuntu" | "Linux Mint" | "Raspbian" => "Debian",
        "Rocky Linux"
        | "AlmaLinux"
        | "Fedora"
        | "CentOS"
        | "Red Hat Enterprise Linux"
        | "Amazon Linux" => "RedHat",
        "Arch Linux" => "Archlinux",
        "Alpine" => "Alpine",
        "openSUSE" | "SUSE" => "Suse",
        _ => return None,
    };
    Some(family)
}

/// Parse `/proc/meminfo`'s `MemTotal:` line into whole megabytes (kB ÷ 1024).
#[must_use]
fn parse_memtotal_mb(text: &str) -> Option<u64> {
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb / 1024);
        }
    }
    None
}

/// Extract the egress interface from `ip -4 route get` output by locating the
/// `dev <iface>` token pair.
#[must_use]
fn parse_route_iface(text: &str) -> Option<String> {
    let mut tokens = text.split_whitespace();
    for tok in tokens.by_ref() {
        if tok == "dev" {
            return tokens.next().filter(|s| !s.is_empty()).map(str::to_string);
        }
    }
    None
}

/// Extract the primary IPv4 address from `ip -4 addr show` output by locating
/// the `inet <a.b.c.d>/<mask>` token pair and stripping the mask.
#[must_use]
fn parse_inet_addr(text: &str) -> Option<String> {
    let mut tokens = text.split_whitespace();
    while let Some(tok) = tokens.next() {
        if tok == "inet" {
            let cidr = tokens.next()?;
            let addr = cidr.split_once('/').map_or(cidr, |(a, _)| a);
            if !addr.is_empty() {
                return Some(addr.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_os_release_fields() {
        let text = "\
NAME=\"Ubuntu\"
VERSION=\"22.04.4 LTS (Jammy Jellyfish)\"
ID=ubuntu
VERSION_ID=\"22.04\"
";
        assert_eq!(
            parse_os_release_field(text, "NAME").as_deref(),
            Some("Ubuntu")
        );
        assert_eq!(
            parse_os_release_field(text, "VERSION_ID").as_deref(),
            Some("22.04")
        );
        assert!(parse_os_release_field(text, "MISSING").is_none());
    }

    #[test]
    fn os_release_field_does_not_match_prefix_collisions() {
        // `VERSION` must not match the `VERSION_ID` line and vice versa.
        let text = "VERSION_ID=\"22.04\"\nVERSION=\"22.04 LTS\"\n";
        assert_eq!(
            parse_os_release_field(text, "VERSION").as_deref(),
            Some("22.04 LTS")
        );
        assert_eq!(
            parse_os_release_field(text, "VERSION_ID").as_deref(),
            Some("22.04")
        );
    }

    #[test]
    fn os_family_maps_known_distributions() {
        assert_eq!(os_family("Ubuntu"), Some("Debian"));
        assert_eq!(os_family("Debian"), Some("Debian"));
        assert_eq!(os_family("Linux Mint"), Some("Debian"));
        assert_eq!(os_family("Raspbian"), Some("Debian"));
        assert_eq!(os_family("Fedora"), Some("RedHat"));
        assert_eq!(os_family("CentOS"), Some("RedHat"));
        assert_eq!(os_family("Rocky Linux"), Some("RedHat"));
        assert_eq!(os_family("AlmaLinux"), Some("RedHat"));
        assert_eq!(os_family("Red Hat Enterprise Linux"), Some("RedHat"));
        assert_eq!(os_family("Amazon Linux"), Some("RedHat"));
        assert_eq!(os_family("Arch Linux"), Some("Archlinux"));
        assert_eq!(os_family("Alpine"), Some("Alpine"));
        assert_eq!(os_family("openSUSE"), Some("Suse"));
        assert_eq!(os_family("SUSE"), Some("Suse"));
        assert_eq!(os_family("GhostOS"), None);
    }

    #[test]
    fn parses_memtotal_mb() {
        let text = "MemTotal:        8388608 kB\nMemFree:          123456 kB\n";
        assert_eq!(parse_memtotal_mb(text), Some(8192));
        assert!(parse_memtotal_mb("nothing useful here").is_none());
    }

    #[test]
    fn parses_route_interface() {
        let text = "8.8.8.8 via 10.0.0.1 dev eth0 src 10.0.0.5 uid 0\n    cache";
        assert_eq!(parse_route_iface(text).as_deref(), Some("eth0"));
        let text2 = "8.8.8.8 dev wlan0 src 192.168.1.5";
        assert_eq!(parse_route_iface(text2).as_deref(), Some("wlan0"));
        assert!(parse_route_iface("nothing useful here").is_none());
    }

    #[test]
    fn parses_inet_address_and_strips_mask() {
        let text = "\
2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP
    link/ether aa:bb:cc:dd:ee:ff brd ff:ff:ff:ff:ff:ff
    inet 10.0.0.5/24 brd 10.0.0.255 scope global eth0
       valid_lft forever preferred_lft forever
    inet6 fe80::a8bb:ccff:fedd:eeff/64 scope link
";
        assert_eq!(parse_inet_addr(text).as_deref(), Some("10.0.0.5"));
        assert!(parse_inet_addr("nothing useful here").is_none());
    }
}
