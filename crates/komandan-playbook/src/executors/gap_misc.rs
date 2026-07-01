//! §6.3 gap executors: archive, cron, mount, reboot, uri.

use std::path::Path;

use komandan_plugin_abi::prelude::*;
use serde_json::Value;

use super::{Connection, ModuleError, ModuleExecutor, ModuleRegistry, TaskContext};

/// Register the misc gap executors on `reg`.
pub fn register_all(reg: &mut ModuleRegistry) {
    reg.register(Archive);
    reg.register(Cron);
    reg.register(Mount);
    reg.register(Reboot);
    reg.register(Uri);
}

/// Return `r` with a replaced `changed` flag (stdout/stderr/rc/success kept).
const fn with_changed(mut r: ModuleResult, changed: bool) -> ModuleResult {
    r.changed = changed;
    r
}

// ---- archive ------------------------------------------------------------

/// `archive` — pack a remote path into an archive file on the remote host.
struct Archive;

impl ModuleExecutor for Archive {
    fn name(&self) -> &'static str {
        "archive"
    }
    /// Execute the module.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Args`] if `path`/`dest` are missing or the format is
    /// unrecognized; [`ModuleError::Host`] on connection failure.
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("archive requires a 'path'"))?;
        let dest = args
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("archive requires a 'dest'"))?;
        let format = args.get("format").and_then(Value::as_str);
        let cmd = build_archive_cmd(path, dest, format).ok_or_else(|| {
            ModuleError::args(format!("archive: unrecognized archive format for '{dest}'"))
        })?;
        let prefix = super::shell_prefix(ctx);
        let result = conn.run_command(&format!("{prefix}{cmd}"))?;
        let changed = result.success && result.rc == 0;
        Ok(with_changed(result, changed))
    }
}

/// Build the shell command that creates an archive at `dest` from `path`.
/// Returns `None` if the format (explicit `format` or inferred from `dest`)
/// is unrecognized. `path` is split into parent (for tar `-C`) and base.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn build_archive_cmd(path: &str, dest: &str, format: Option<&str>) -> Option<String> {
    use std::fmt::Write as _;
    let f = format.map_or_else(|| dest.to_ascii_lowercase(), str::to_ascii_lowercase);
    let p = Path::new(path);
    let parent = p
        .parent()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(".");
    let base = p.file_name().and_then(|s| s.to_str()).unwrap_or(path);
    let mut s = String::new();
    if f == "tar.gz" || f.ends_with(".tar.gz") || f == "tgz" || f.ends_with(".tgz") {
        let _ = write!(s, "tar czf {dest} -C {parent} {base}");
    } else if f == "tar.bz2" || f.ends_with(".tar.bz2") || f == "tbz2" || f.ends_with(".tbz2") {
        let _ = write!(s, "tar cjf {dest} -C {parent} {base}");
    } else if f == "tar.xz" || f.ends_with(".tar.xz") || f == "txz" || f.ends_with(".txz") {
        let _ = write!(s, "tar cJf {dest} -C {parent} {base}");
    } else if f == "tar" || f.ends_with(".tar") {
        let _ = write!(s, "tar cf {dest} -C {parent} {base}");
    } else if f == "zip" || f.ends_with(".zip") {
        let _ = write!(s, "zip -r {dest} {path}");
    } else {
        return None;
    }
    Some(s)
}

// ---- cron ---------------------------------------------------------------

/// `cron` — manage a bounded crontab entry block on the remote host.
struct Cron;

impl ModuleExecutor for Cron {
    fn name(&self) -> &'static str {
        "cron"
    }
    /// Execute the module.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Args`] if `name`/`job` are missing; [`ModuleError::Host`]
    /// on connection failure.
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("cron requires a 'name'"))?;
        let job = args
            .get("job")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("cron requires a 'job'"))?;
        let minute = args.get("minute").and_then(Value::as_str).unwrap_or("*");
        let hour = args.get("hour").and_then(Value::as_str).unwrap_or("*");
        let day = args.get("day").and_then(Value::as_str).unwrap_or("*");
        let month = args.get("month").and_then(Value::as_str).unwrap_or("*");
        let weekday = args.get("weekday").and_then(Value::as_str).unwrap_or("*");
        let state = args
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("present");
        let user = args.get("user").and_then(Value::as_str);
        let cmd = build_cron_command(name, job, minute, hour, day, month, weekday, state, user);
        let prefix = super::shell_prefix(ctx);
        let result = conn.run_command(&format!("{prefix}{cmd}"))?;
        let changed = result.success && result.rc == 0;
        Ok(with_changed(result, changed))
    }
}

/// Build the shell snippet that strips any existing `# {name}`..`# end {name}`
/// block from the crontab and (for `present`) appends a fresh block. `user`
/// adds ` -u {user}` to both the read and write `crontab` calls.
#[allow(clippy::too_many_arguments)] // seven schedule fields + name/job/state/user
fn build_cron_command(
    name: &str,
    job: &str,
    minute: &str,
    hour: &str,
    day: &str,
    month: &str,
    weekday: &str,
    state: &str,
    user: Option<&str>,
) -> String {
    use std::fmt::Write as _;
    let u = user.map_or(String::new(), |user| format!(" -u {user}"));
    let mut s = String::new();
    let _ = writeln!(
        s,
        "CURT=$(crontab{u} -l 2>/dev/null | awk '/^# {name}$/{{f=1;next}} /^# end {name}$/{{f=0;next}} !f')"
    );
    if state == "absent" {
        let _ = writeln!(s, "printf '%s\\n' \"$CURT\" | crontab{u} -");
    } else {
        let _ = writeln!(
            s,
            "printf '%s\\n%s\\n%s\\n%s\\n' \"$CURT\" \"# {name}\" \"{minute} {hour} {day} {month} {weekday} {job}\" \"# end {name}\" | crontab{u} -"
        );
    }
    s
}

// ---- mount --------------------------------------------------------------

/// `mount` — manage a mount (and, in later phases, an `/etc/fstab` entry) on
/// the remote host.
struct Mount;

impl ModuleExecutor for Mount {
    fn name(&self) -> &'static str {
        "mount"
    }
    /// Execute the module.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Args`] if `path` is missing, `src`/`fstype` are missing
    /// for `mounted`, or the `state` is unsupported; [`ModuleError::Host`] on
    /// connection failure.
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("mount requires a 'path'"))?;
        let state = args
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("mounted");
        let prefix = super::shell_prefix(ctx);
        match state {
            "mounted" => {
                let src = args
                    .get("src")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ModuleError::args("mount 'mounted' requires a 'src'"))?;
                let fstype = args
                    .get("fstype")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ModuleError::args("mount 'mounted' requires an 'fstype'"))?;
                let opts = args
                    .get("opts")
                    .and_then(Value::as_str)
                    .unwrap_or("defaults");
                let cmd = format!("mount -t {fstype} -o {opts} {src} {path}");
                let result = conn.run_command(&format!("{prefix}{cmd}"))?;
                let changed = result.success && result.rc == 0;
                Ok(with_changed(result, changed))
            }
            "unmounted" => {
                let result = conn.run_command(&format!("{prefix}umount {path}"))?;
                let changed = result.success && result.rc == 0;
                Ok(with_changed(result, changed))
            }
            "remounted" => {
                let result = conn.run_command(&format!("{prefix}mount -o remount {path}"))?;
                let changed = result.success && result.rc == 0;
                Ok(with_changed(result, changed))
            }
            "present" | "absent" => Ok(ModuleResult {
                changed: false,
                rc: 0,
                stdout: RString::new(),
                stderr: RString::new(),
                success: true,
                msg: ROption::RSome(RStr::from(
                    "mount: /etc/fstab management is not yet implemented",
                )),
            }),
            other => Err(ModuleError::args(format!(
                "mount: unsupported state '{other}'"
            ))),
        }
    }
}

// ---- reboot -------------------------------------------------------------

/// `reboot` — reboot the remote host via `shutdown -r now`.
struct Reboot;

impl ModuleExecutor for Reboot {
    fn name(&self) -> &'static str {
        "reboot"
    }
    /// Execute the module.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Host`] on connection failure (the connection will often
    /// drop mid-shutdown; v0.1 does not reconnect-poll).
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let pre = args
            .get("pre_reboot_delay")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if pre > 0 {
            std::thread::sleep(std::time::Duration::from_secs(pre));
        }
        // TODO(phase2): reconnect-poll honoring `reboot_timeout`; v0.1 fires
        // the shutdown and returns.
        let cmd = args.get("msg").and_then(Value::as_str).map_or_else(
            || String::from("shutdown -r now"),
            |m| format!("shutdown -r now \"{m}\""),
        );
        let prefix = super::shell_prefix(ctx);
        let result = conn.run_command(&format!("{prefix}{cmd}"))?;
        let success = result.success;
        Ok(with_changed(result, success))
    }
}

// ---- uri ----------------------------------------------------------------

/// `uri` — issue an HTTP request from the target host via `curl`.
struct Uri;

impl ModuleExecutor for Uri {
    fn name(&self) -> &'static str {
        "uri"
    }
    /// Execute the module.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Args`] if `url` is missing; [`ModuleError::Host`] on
    /// connection failure.
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("uri requires a 'url'"))?;
        let method = args.get("method").and_then(Value::as_str).unwrap_or("GET");
        let body = args.get("body").and_then(Value::as_str);
        let headers = args.get("headers").and_then(Value::as_object);
        let status_code = args
            .get("status_code")
            .and_then(Value::as_i64)
            .unwrap_or(200);
        let return_content = args
            .get("return_content")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let hdr_list: Vec<(String, String)> = headers
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let cmd = build_curl_command(url, method, body, &hdr_list);
        let prefix = super::shell_prefix(ctx);
        let result = conn.run_command(&format!("{prefix}{cmd}"))?;

        if !result.success || result.rc != 0 {
            return Ok(ModuleResult {
                changed: false,
                rc: result.rc,
                stdout: RString::new(),
                stderr: result.stderr,
                success: false,
                msg: ROption::RSome(RStr::from("uri: curl request failed")),
            });
        }

        let (body_out, status) = parse_uri_status(result.stdout.as_str());
        let success = status.map(i64::from) == Some(status_code);
        let changed = success && method != "GET";
        let stdout = if return_content {
            body_out
        } else {
            String::new()
        };
        Ok(ModuleResult {
            changed,
            rc: 0,
            stdout: RString::from(stdout),
            stderr: result.stderr,
            success,
            msg: ROption::RSome(crate::leak::rstr(&format!("HTTP {}", status.unwrap_or(-1)))),
        })
    }
}

/// Build the `curl` invocation. The response body is written to stdout
/// followed by a newline and the HTTP status code (`-w '\n%{http_code}'`).
fn build_curl_command(
    url: &str,
    method: &str,
    body: Option<&str>,
    headers: &[(String, String)],
) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("curl -s -o - -w '\\n%{http_code}'");
    let _ = write!(s, " -X {method}");
    for (k, v) in headers {
        let _ = write!(s, " -H \"{k}: {v}\"");
    }
    if let Some(b) = body {
        let _ = write!(s, " --data \"{b}\"");
    }
    let _ = write!(s, " {url}");
    s
}

/// Split `curl` stdout into `(body, status)`. The last newline-delimited line
/// is parsed as the HTTP status code; everything before it is the body. A
/// trailing newline (if any) is trimmed first.
fn parse_uri_status(stdout: &str) -> (String, Option<i32>) {
    let trimmed = stdout.strip_suffix('\n').unwrap_or(stdout);
    trimmed.rfind('\n').map_or_else(
        || (String::new(), trimmed.parse::<i32>().ok()),
        |i| {
            let status_str = &trimmed[i + 1..];
            (trimmed[..i].to_string(), status_str.parse::<i32>().ok())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::{Connection, CoreApiRef};
    use crate::test_support::{MockCore, localhost_host, null_core};
    use serde_json::json;

    fn conn(core: &CoreApiRef) -> Connection<'_> {
        Connection::new(core, ConnectionHandle::INVALID, localhost_host())
    }

    fn noop_ctx() -> TaskContext {
        TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        )
    }

    // ---- archive ----

    #[test]
    fn archive_tar_gz_uses_parent_and_base() {
        let core = null_core();
        let r = Archive
            .run(
                &conn(&core),
                &json!({"path": "/var/log/app", "dest": "/tmp/app.tar.gz"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(
            r.stdout
                .as_str()
                .contains("tar czf /tmp/app.tar.gz -C /var/log app"),
            "{}",
            r.stdout
        );
    }

    #[test]
    fn archive_explicit_format_overrides_dest_extension() {
        let cmd = build_archive_cmd("/srv/data", "/tmp/out.bin", Some("zip"));
        let s = cmd.unwrap_or_else(|| panic!("expected zip archive command"));
        assert!(s.contains("zip -r /tmp/out.bin /srv/data"), "{s}");
    }

    #[test]
    fn archive_unrecognized_format_errors() {
        assert!(
            Archive
                .run(
                    &conn(&null_core()),
                    &json!({"path": "/x", "dest": "/y.bin"}),
                    &noop_ctx()
                )
                .is_err()
        );
    }

    #[test]
    fn archive_missing_path_errors() {
        assert!(
            Archive
                .run(&conn(&null_core()), &json!({"dest": "/y.tgz"}), &noop_ctx())
                .is_err()
        );
    }

    // ---- cron ----

    #[test]
    fn cron_present_builds_block_with_markers() {
        let s = build_cron_command(
            "daily-backup",
            "/usr/local/bin/backup.sh",
            "0",
            "2",
            "*",
            "*",
            "*",
            "present",
            None,
        );
        assert!(s.contains("crontab -l"), "{s}");
        assert!(s.contains("# daily-backup"), "{s}");
        assert!(s.contains("# end daily-backup"), "{s}");
        assert!(s.contains("0 2 * * * /usr/local/bin/backup.sh"), "{s}");
        assert!(s.contains("crontab -"), "{s}");
    }

    #[test]
    fn cron_absent_strips_block() {
        let s = build_cron_command("cleanup", "true", "*", "*", "*", "*", "*", "absent", None);
        assert!(s.contains("awk"), "{s}");
        assert!(s.contains("crontab -"), "{s}");
        assert!(
            !s.contains("* * * * * true"),
            "absent must not append the job: {s}"
        );
    }

    #[test]
    fn cron_user_flag_applied_to_read_and_write() {
        let s = build_cron_command(
            "n",
            "j",
            "*",
            "*",
            "*",
            "*",
            "*",
            "present",
            Some("appuser"),
        );
        assert!(s.contains("crontab -u appuser -l"), "{s}");
        assert!(s.contains("crontab -u appuser -"), "{s}");
    }

    #[test]
    fn cron_missing_name_errors() {
        assert!(
            Cron.run(&conn(&null_core()), &json!({"job": "x"}), &noop_ctx())
                .is_err()
        );
    }

    #[test]
    fn cron_missing_job_errors() {
        assert!(
            Cron.run(&conn(&null_core()), &json!({"name": "x"}), &noop_ctx())
                .is_err()
        );
    }

    // ---- mount ----

    #[test]
    fn mount_mounted_runs_mount_command() {
        let core = null_core();
        let r = Mount
            .run(
                &conn(&core),
                &json!({
                    "path": "/mnt/data",
                    "src": "/dev/sda1",
                    "fstype": "ext4"
                }),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(
            r.stdout
                .as_str()
                .contains("mount -t ext4 -o defaults /dev/sda1 /mnt/data"),
            "{}",
            r.stdout
        );
    }

    #[test]
    fn mount_unmounted_runs_umount() {
        let core = null_core();
        let r = Mount
            .run(
                &conn(&core),
                &json!({"path": "/mnt/data", "state": "unmounted"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            r.stdout.as_str().contains("umount /mnt/data"),
            "{}",
            r.stdout
        );
    }

    #[test]
    fn mount_present_is_unchanged_with_msg() {
        let core = null_core();
        let r = Mount
            .run(
                &conn(&core),
                &json!({"path": "/mnt/x", "state": "present"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!r.changed);
        assert!(r.success);
        assert!(
            r.msg
                .as_ref()
                .map_or(false, |m| m.as_str().contains("fstab")),
            "{:?}",
            r.msg
        );
    }

    #[test]
    fn mount_mounted_requires_src_and_fstype() {
        assert!(
            Mount
                .run(
                    &conn(&null_core()),
                    &json!({"path": "/m", "fstype": "ext4"}),
                    &noop_ctx()
                )
                .is_err()
        );
        assert!(
            Mount
                .run(
                    &conn(&null_core()),
                    &json!({"path": "/m", "src": "/dev/x"}),
                    &noop_ctx()
                )
                .is_err()
        );
    }

    #[test]
    fn mount_missing_path_errors() {
        assert!(
            Mount
                .run(&conn(&null_core()), &json!({}), &noop_ctx())
                .is_err()
        );
    }

    // ---- reboot ----

    #[test]
    fn reboot_default_uses_shutdown_r_now() {
        let core = null_core();
        let r = Reboot
            .run(&conn(&core), &json!({}), &noop_ctx())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(r.stdout.as_str() == "shutdown -r now", "{}", r.stdout);
    }

    #[test]
    fn reboot_msg_quoted_in_command() {
        let core = null_core();
        let r = Reboot
            .run(&conn(&core), &json!({"msg": "kernel upgrade"}), &noop_ctx())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            r.stdout.as_str() == "shutdown -r now \"kernel upgrade\"",
            "{}",
            r.stdout
        );
    }

    // ---- uri ----

    #[test]
    fn build_curl_command_includes_method_headers_and_body() {
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Api-Key".to_string(), "secret".to_string()),
        ];
        let s = build_curl_command(
            "https://example.invalid/hook",
            "POST",
            Some("{\"a\":1}"),
            &headers,
        );
        assert!(s.contains("curl -s -o - -w '\\n%{http_code}'"), "{s}");
        assert!(s.contains(" -X POST"), "{s}");
        assert!(s.contains(" -H \"Content-Type: application/json\""), "{s}");
        assert!(s.contains(" -H \"X-Api-Key: secret\""), "{s}");
        assert!(s.contains(" --data "), "{s}");
        assert!(s.contains(r#"{"a":1}"#), "{s}");
        assert!(s.ends_with(" https://example.invalid/hook"), "{s}");
    }

    #[test]
    fn build_curl_command_omits_body_and_headers_when_absent() {
        let s = build_curl_command("https://x.invalid/", "GET", None, &[]);
        assert!(!s.contains(" -H "), "{s}");
        assert!(!s.contains(" --data "), "{s}");
        assert!(s.contains(" -X GET"), "{s}");
    }

    #[test]
    fn parse_uri_status_splits_body_and_code() {
        let (body, status) = parse_uri_status("hello\nworld\n200");
        assert_eq!(body, "hello\nworld");
        assert_eq!(status, Some(200));
    }

    #[test]
    fn parse_uri_status_handles_empty_body() {
        let (body, status) = parse_uri_status("\n404");
        assert_eq!(body, "");
        assert_eq!(status, Some(404));
    }

    #[test]
    fn parse_uri_status_trailing_newline_trimmed() {
        let (body, status) = parse_uri_status("ok\n201\n");
        assert_eq!(body, "ok");
        assert_eq!(status, Some(201));
    }

    #[test]
    fn uri_get_success_not_changed_no_body_returned() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("hello world\n200"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Uri
            .run(
                &conn(&core_ref),
                &json!({"url": "https://example.invalid/"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.success, "{:?}", r.msg);
        assert!(!r.changed, "GET must not be changed");
        assert!(r.stdout.as_str().is_empty(), "body must be stripped");
        assert!(
            r.msg.as_ref().map_or(false, |m| m.as_str() == "HTTP 200"),
            "{:?}",
            r.msg
        );
    }

    #[test]
    fn uri_post_with_return_content_is_changed_and_returns_body() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("{\"ok\":true}\n201"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Uri
            .run(
                &conn(&core_ref),
                &json!({
                    "url": "https://example.invalid/",
                    "method": "POST",
                    "status_code": 201,
                    "return_content": true
                }),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.success, "{:?}", r.msg);
        assert!(r.changed, "POST success must be changed");
        assert_eq!(r.stdout.as_str(), "{\"ok\":true}");
        assert!(
            r.msg.as_ref().map_or(false, |m| m.as_str() == "HTTP 201"),
            "{:?}",
            r.msg
        );
    }

    #[test]
    fn uri_unexpected_status_reports_failure() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("nope\n500"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Uri
            .run(
                &conn(&core_ref),
                &json!({"url": "https://example.invalid/"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!r.success);
        assert!(!r.changed);
    }

    #[test]
    fn uri_missing_url_errors() {
        assert!(
            Uri.run(&conn(&null_core()), &json!({}), &noop_ctx())
                .is_err()
        );
    }
}
