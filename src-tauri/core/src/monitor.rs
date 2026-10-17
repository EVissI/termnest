//! Server information window: resource usage and `authorized_keys`, collected with a
//! fixed read-only script over a connection kept open while the window is shown.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::Handle;
use russh::keys::{HashAlg, PublicKey};
use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::ssh::{self, Auth, Client, SshContext};
use crate::target::Target;

/// Fed to `sh -s` through stdin, so the user's login shell and quoting never matter.
/// Only reads: /proc, df, ps, /etc/passwd and readable authorized_keys files.
const SCRIPT: &str = r#"
echo @@host; hostname 2>/dev/null || cat /proc/sys/kernel/hostname
echo @@os; (. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME")
echo @@kernel; uname -srm
echo @@cpu; grep -m1 'model name' /proc/cpuinfo | cut -d: -f2-; grep -c '^processor' /proc/cpuinfo
echo @@load; cat /proc/loadavg
echo @@sample1; cat /proc/uptime; head -n1 /proc/stat; cat /proc/net/dev
echo @@proc1; cat /proc/[0-9]*/stat 2>/dev/null; echo
sleep 0.5 2>/dev/null || sleep 1
echo @@sample2; cat /proc/uptime; head -n1 /proc/stat; cat /proc/net/dev
echo @@proc2; cat /proc/[0-9]*/stat 2>/dev/null; echo
echo @@mem; cat /proc/meminfo
echo @@df; df -P -k 2>/dev/null
echo @@ps; ps -eo pid=,user=,pmem= 2>/dev/null
echo @@user; id -un
echo @@keys
seen=" "
while IFS=: read -r u _ _ _ _ h _; do
  f="$h/.ssh/authorized_keys"
  case "$seen" in *" $f "*) continue ;; esac
  seen="$seen$f "
  if [ -r "$f" ]; then echo "@@file $u $f"; cat "$f"; echo; fi
done < /etc/passwd
"#;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Disk {
    pub mount: String,
    pub filesystem: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Process {
    pub pid: u32,
    pub user: String,
    /// Share of one core over the sampling interval, like `top`.
    pub cpu: f64,
    pub mem: f64,
    pub command: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AuthorizedKey {
    pub key_type: String,
    /// `SHA256:...`, empty if the line could not be parsed.
    pub fingerprint: String,
    pub comment: String,
    /// Options before the key (`from=...`, `command=...`).
    pub options: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct KeyFile {
    pub user: String,
    pub path: String,
    pub keys: Vec<AuthorizedKey>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ServerInfo {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub uptime_secs: u64,
    pub load: [f64; 3],
    /// Busy share of all cores over half a second, 0-100.
    pub cpu_percent: Option<f64>,
    /// Bytes.
    pub mem_total: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
    pub disks: Vec<Disk>,
    /// Bytes per second over all interfaces except loopback.
    pub net_rx_rate: Option<f64>,
    pub net_tx_rate: Option<f64>,
    pub processes: Vec<Process>,
    /// The account termnest is logged in as.
    pub user: String,
    /// authorized_keys files this account can read (all of them for root).
    pub key_files: Vec<KeyFile>,
}

/// Splits the output into `@@name` sections (a `@@file` line starts a key file).
fn sections(out: &str) -> HashMap<&str, Vec<&str>> {
    let mut map: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut current = "";
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            if !name.starts_with("file ") {
                current = name.trim();
                map.entry(current).or_default();
                continue;
            }
        }
        map.entry(current).or_default().push(line);
    }
    map
}

struct Sample {
    uptime: f64,
    /// (busy, total) jiffies.
    cpu: Option<(u64, u64)>,
    /// (rx, tx) bytes.
    net: Option<(u64, u64)>,
}

fn parse_sample(lines: &[&str]) -> Sample {
    let uptime = lines
        .first()
        .and_then(|l| l.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let cpu = lines.iter().find(|l| l.starts_with("cpu ")).map(|l| {
        let v: Vec<u64> = l
            .split_whitespace()
            .skip(1)
            .filter_map(|x| x.parse().ok())
            .collect();
        // user nice system idle iowait irq softirq steal (guest is already in user).
        let total: u64 = v.iter().take(8).sum();
        let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
        (total.saturating_sub(idle), total)
    });
    let mut net = None;
    for l in lines {
        let Some((iface, rest)) = l.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo" || iface.contains(' ') {
            continue;
        }
        let v: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        if v.len() >= 9 {
            let (rx, tx) = net.unwrap_or((0, 0));
            net = Some((rx + v[0], tx + v[8]));
        }
    }
    Sample { uptime, cpu, net }
}

fn meminfo(lines: &[&str], key: &str) -> u64 {
    lines
        .iter()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

/// Real storage only: no pseudo file systems, snaps or runtime mounts.
fn parse_df(lines: &[&str]) -> Vec<Disk> {
    let mut disks: Vec<Disk> = Vec::new();
    for l in lines.iter().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 6 {
            continue;
        }
        let mount = f[5..].join(" ");
        let fs = f[0];
        let pseudo = matches!(
            fs,
            "tmpfs" | "devtmpfs" | "udev" | "none" | "shm" | "efivarfs"
        ) || fs.starts_with("overlay")
            || [
                "/snap/",
                "/run",
                "/dev",
                "/sys",
                "/proc",
                "/boot/efi",
                "/var/lib/docker/",
                "/var/lib/containers/",
                "/var/lib/kubelet/",
            ]
            .iter()
            .any(|p| mount.starts_with(p));
        let total = f[1].parse::<u64>().unwrap_or(0) * 1024;
        if (pseudo && mount != "/") || total == 0 || disks.iter().any(|d| d.mount == mount) {
            continue;
        }
        disks.push(Disk {
            mount,
            filesystem: fs.to_owned(),
            total,
            used: f[2].parse::<u64>().unwrap_or(0) * 1024,
            available: f[3].parse::<u64>().unwrap_or(0) * 1024,
        });
    }
    disks
}

/// pid -> (command, utime + stime) from `/proc/<pid>/stat` lines.
fn proc_times(lines: &[&str]) -> HashMap<u32, (String, u64)> {
    let mut map = HashMap::new();
    for l in lines {
        // The command is in parentheses and may itself contain spaces or parentheses.
        let (Some(open), Some(close)) = (l.find('('), l.rfind(')')) else {
            continue;
        };
        let Ok(pid) = l[..open].trim().parse::<u32>() else {
            continue;
        };
        let rest: Vec<&str> = l[close + 1..].split_whitespace().collect();
        // Fields after the command start with state (field 3): utime is 14, stime 15.
        let time = |i: usize| rest.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        map.insert(pid, (l[open + 1..close].to_owned(), time(11) + time(12)));
    }
    map
}

/// The five busiest processes between the two samples (`jiffies_per_core` is the
/// length of the interval in clock ticks).
fn top_processes(
    before: &[&str],
    after: &[&str],
    ps: &[&str],
    jiffies_per_core: f64,
) -> Vec<Process> {
    if jiffies_per_core <= 0.0 {
        return Vec::new();
    }
    let before = proc_times(before);
    let owners: HashMap<u32, (&str, f64)> = ps
        .iter()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((
                f.first()?.parse().ok()?,
                (
                    *f.get(1)?,
                    f.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                ),
            ))
        })
        .collect();
    let mut list: Vec<Process> = proc_times(after)
        .into_iter()
        .filter_map(|(pid, (command, t2))| {
            let (_, t1) = before.get(&pid)?;
            let delta = t2.checked_sub(*t1)?;
            let (user, mem) = owners.get(&pid).copied().unwrap_or(("", 0.0));
            Some(Process {
                pid,
                user: user.to_owned(),
                cpu: delta as f64 * 100.0 / jiffies_per_core,
                mem,
                command,
            })
        })
        .filter(|p| p.cpu > 0.0)
        .collect();
    list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(a.pid.cmp(&b.pid)));
    list.truncate(5);
    list
}

const KEY_TYPES: [&str; 4] = ["ssh-", "ecdsa-sha2-", "sk-ssh-", "sk-ecdsa-"];

/// One `authorized_keys` line; `None` for blank lines and comments.
pub fn parse_authorized_key(line: &str) -> Option<AuthorizedKey> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    // The key type is the first token that looks like one; anything before it is options.
    let mut offset = 0;
    let mut start = None;
    for token in line.split_ascii_whitespace() {
        let at = offset + line[offset..].find(token).unwrap_or(0);
        offset = at + token.len();
        if KEY_TYPES.iter().any(|p| token.starts_with(p)) {
            start = Some(at);
            break;
        }
    }
    let Some(start) = start else {
        return Some(AuthorizedKey {
            comment: line.chars().take(80).collect(),
            ..Default::default()
        });
    };
    let options = line[..start].trim().to_owned();
    let key = &line[start..];
    let mut parts = key.split_ascii_whitespace();
    let key_type = parts.next().unwrap_or_default().to_owned();
    let comment = parts.skip(1).collect::<Vec<_>>().join(" ");
    let fingerprint = PublicKey::from_openssh(key)
        .map(|k| k.fingerprint(HashAlg::Sha256).to_string())
        .unwrap_or_default();
    Some(AuthorizedKey {
        key_type,
        fingerprint,
        comment,
        options,
    })
}

/// The `keys` section: `@@file user path` headers followed by the file contents.
fn parse_keys(lines: &[&str]) -> Vec<KeyFile> {
    let mut files: Vec<KeyFile> = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("@@file ") {
            let (user, path) = rest.split_once(' ').unwrap_or((rest, ""));
            files.push(KeyFile {
                user: user.to_owned(),
                path: path.to_owned(),
                keys: Vec::new(),
            });
        } else if let Some(file) = files.last_mut() {
            if let Some(key) = parse_authorized_key(line) {
                file.keys.push(key);
            }
        }
    }
    files
}

pub fn parse(out: &str) -> ServerInfo {
    let s = sections(out);
    let get = |name: &str| s.get(name).cloned().unwrap_or_default();
    let first = |name: &str| {
        get(name)
            .iter()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .to_owned()
    };

    let cpu = get("cpu");
    let load: Vec<f64> = first("load")
        .split_whitespace()
        .take(3)
        .filter_map(|v| v.parse().ok())
        .collect();
    let a = parse_sample(&get("sample1"));
    let b = parse_sample(&get("sample2"));
    let cpu_cores: u32 = cpu.get(1).and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    let (cpu_percent, jiffies) = match (a.cpu, b.cpu) {
        (Some((busy1, total1)), Some((busy2, total2))) if total2 > total1 => (
            Some((busy2.saturating_sub(busy1)) as f64 * 100.0 / (total2 - total1) as f64),
            (total2 - total1) as f64,
        ),
        _ => (None, 0.0),
    };
    let dt = b.uptime - a.uptime;
    let rate = |x: u64, y: u64| (dt > 0.0).then(|| y.saturating_sub(x) as f64 / dt);
    let (net_rx_rate, net_tx_rate) = match (a.net, b.net) {
        (Some((rx1, tx1)), Some((rx2, tx2))) => (rate(rx1, rx2), rate(tx1, tx2)),
        _ => (None, None),
    };
    let mem = get("mem");

    ServerInfo {
        hostname: first("host"),
        os: first("os"),
        kernel: first("kernel"),
        cpu_model: cpu.first().map(|l| l.trim().to_owned()).unwrap_or_default(),
        cpu_cores,
        uptime_secs: b.uptime as u64,
        load: [
            load.first().copied().unwrap_or(0.0),
            load.get(1).copied().unwrap_or(0.0),
            load.get(2).copied().unwrap_or(0.0),
        ],
        cpu_percent,
        mem_total: meminfo(&mem, "MemTotal"),
        mem_available: meminfo(&mem, "MemAvailable"),
        swap_total: meminfo(&mem, "SwapTotal"),
        swap_free: meminfo(&mem, "SwapFree"),
        disks: parse_df(&get("df")),
        net_rx_rate,
        net_tx_rate,
        processes: top_processes(
            &get("proc1"),
            &get("proc2"),
            &get("ps"),
            jiffies / f64::from(cpu_cores.max(1)),
        ),
        user: first("user"),
        key_files: parse_keys(&get("keys")),
    }
}

/// Connections of open server information windows.
#[derive(Default)]
pub struct MonitorManager {
    runs: Mutex<HashMap<u64, Arc<Handle<Client>>>>,
    next: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorOpened {
    pub id: u64,
    /// Fingerprint of the key used for this connection (marked among authorized keys).
    pub key_fingerprint: Option<String>,
}

impl MonitorManager {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Handle<Client>>>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub async fn open(
        &self,
        target: &Target,
        key_path: String,
        ctx: &SshContext,
    ) -> CoreResult<MonitorOpened> {
        let path = PathBuf::from(&key_path);
        let handle = ssh::connect(target, Auth::Key(&path), ctx).await?;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.lock().insert(id, Arc::new(handle));
        Ok(MonitorOpened {
            id,
            key_fingerprint: crate::keys::validate(&path).ok().map(|k| k.fingerprint),
        })
    }

    pub async fn sample(&self, id: u64) -> CoreResult<ServerInfo> {
        let handle = self
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Окно мониторинга закрыто"))?;
        let out = ssh::exec(&handle, "sh -s", Some(SCRIPT.as_bytes())).await?;
        if !out.stdout.contains("@@host") {
            return Err(CoreError::new(
                ErrorKind::Remote,
                format!(
                    "Сервер не выполнил скрипт сбора данных: {}",
                    out.stderr.trim()
                ),
            ));
        }
        Ok(parse(&out.stdout))
    }

    pub async fn close(&self, id: u64) {
        let run = self.lock().remove(&id);
        if let Some(handle) = run {
            ssh::disconnect(&handle).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGFg5V2mEhSIuChwP6l0XCeU/wJuNqlvLxbAyJn8dtFQ";

    fn output() -> String {
        format!(
            "@@host\nweb-1\n@@os\nUbuntu 24.04.1 LTS\n@@kernel\nLinux 6.8.0-45-generic x86_64\n\
             @@cpu\n Intel(R) Xeon(R) CPU\n4\n@@load\n0.52 0.40 0.31 1/234 5678\n\
             @@sample1\n1000.00 3900.00\ncpu  100 0 100 700 100 0 0 0 0 0\n\
             Inter-|   Receive\n face |bytes\n    lo: 500 1 0 0 0 0 0 0 500 1 0 0 0 0 0 0\n  eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n\
             @@proc1\n1 (systemd) S 0 1 1 0 -1 0 0 0 0 0 100 50 0 0\n\
             1234 (php (fpm) worker) R 1 1 1 0 -1 0 0 0 0 0 1000 0 0 0\n77 (gone) S 1 1 1 0 -1 0 0 0 0 0 5 5 0 0\n\
             @@sample2\n1000.50 3901.00\ncpu  150 0 150 750 150 0 0 0 0 0\n\
             Inter-|   Receive\n face |bytes\n    lo: 900 1 0 0 0 0 0 0 900 1 0 0 0 0 0 0\n  eth0: 3000 10 0 0 0 0 0 0 2500 20 0 0 0 0 0 0\n\
             @@proc2\n1 (systemd) S 0 1 1 0 -1 0 0 0 0 0 100 50 0 0\n\
             1234 (php (fpm) worker) R 1 1 1 0 -1 0 0 0 0 0 1020 5 0 0\n9 (sh) S 1 1 1 0 -1 0 0 0 0 0 1 0 0 0\n\
             @@mem\nMemTotal:        2000 kB\nMemFree:  100 kB\nMemAvailable:     500 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n\
             @@df\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/sda1 1000 600 400 60% /\n\
             tmpfs 100 0 100 0% /run\n/dev/loop1 50 50 0 100% /snap/core/1\n/dev/sdb1 2000 0 2000 0% /mnt/my data\n\
             overlay 1000 600 400 60% /var/lib/docker/overlay2/abc/merged\n\
             @@ps\n  1234 www-data  3.1\n     1 root      0.1\n\
             @@user\nroot\n@@keys\n@@file root /root/.ssh/authorized_keys\n\
             {KEY} me@laptop\n# comment\n\nfrom=\"10.0.0.0/8\",no-pty {KEY} ci deploy\ngarbage line\n\
             @@file deploy /home/deploy/.ssh/authorized_keys\n{KEY}\n"
        )
    }

    #[test]
    fn parses_everything() {
        let info = parse(&output());
        assert_eq!(info.hostname, "web-1");
        assert_eq!(info.os, "Ubuntu 24.04.1 LTS");
        assert_eq!(info.cpu_model, "Intel(R) Xeon(R) CPU");
        assert_eq!(info.cpu_cores, 4);
        assert_eq!(info.uptime_secs, 1000);
        assert_eq!(info.load, [0.52, 0.40, 0.31]);
        // busy 200 -> 300 of total 1000 -> 1200.
        assert_eq!(info.cpu_percent, Some(50.0));
        assert_eq!(info.net_rx_rate, Some(4000.0));
        assert_eq!(info.net_tx_rate, Some(1000.0));
        assert_eq!(info.mem_total, 2000 * 1024);
        assert_eq!(info.mem_available, 500 * 1024);
        let mounts: Vec<_> = info.disks.iter().map(|d| d.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/mnt/my data"]);
        assert_eq!(info.disks[0].used, 600 * 1024);
        // 200 jiffies over 4 cores = 50 per core; php used 25 of them.
        assert_eq!(
            info.processes.len(),
            1,
            "idle and short-lived ones are skipped"
        );
        let p = &info.processes[0];
        assert_eq!((p.pid, p.command.as_str()), (1234, "php (fpm) worker"));
        assert_eq!((p.cpu, p.user.as_str(), p.mem), (50.0, "www-data", 3.1));
        assert_eq!(info.user, "root");

        assert_eq!(info.key_files.len(), 2);
        let root = &info.key_files[0];
        assert_eq!(root.user, "root");
        assert_eq!(root.keys.len(), 3);
        assert_eq!(root.keys[0].key_type, "ssh-ed25519");
        assert_eq!(root.keys[0].comment, "me@laptop");
        assert!(root.keys[0].fingerprint.starts_with("SHA256:"));
        assert_eq!(root.keys[1].options, "from=\"10.0.0.0/8\",no-pty");
        assert_eq!(root.keys[1].comment, "ci deploy");
        assert_eq!(root.keys[1].fingerprint, root.keys[0].fingerprint);
        assert_eq!(root.keys[2].fingerprint, "");
        assert_eq!(info.key_files[1].path, "/home/deploy/.ssh/authorized_keys");
        assert_eq!(info.key_files[1].keys[0].comment, "");
    }

    #[test]
    fn missing_sections_do_not_fail() {
        let info = parse("@@host\nbox\n@@ps\n@@keys\n");
        assert_eq!(info.hostname, "box");
        assert_eq!(info.cpu_percent, None);
        assert!(info.key_files.is_empty());
    }
}
