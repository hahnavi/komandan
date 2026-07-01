//! Conversion bridge between komandan-plugin-abi mirror types and komandan-core models.

use komandan_plugin_abi::{
    GlobalFlags, HostInfo, ModuleResult, ROption, RString, RValue, ReportStatus,
};
use mlua::{Lua, Value};
use secrecy::SecretString;

use crate::args::Flags;
use crate::models::{ConnectionType, Host, KomandoResult};
use crate::report::TaskStatus;
use crate::ssh::ElevationMethod;

/// Map a plugin `HostInfo` to a komandan-core `Host`.
///
/// Lenient: unparseable `connection_type` / `become_method` map to `None`
/// rather than erroring (plugins may send partial host data). `HostInfo`
/// fields absent from `Host` (`key_check`, `env`) default to `None`.
/// Secret fields (`password`, `private_key_pass`) are wrapped in `SecretString`.
#[must_use]
pub fn host_info_to_host(info: &HostInfo) -> Host {
    Host {
        name: match &info.name {
            ROption::RSome(s) => Some(s.to_string()),
            ROption::RNone => None,
        },
        address: info.address.to_string(),
        port: match info.port {
            ROption::RSome(p) => Some(p),
            ROption::RNone => None,
        },
        user: match &info.user {
            ROption::RSome(s) => Some(s.to_string()),
            ROption::RNone => None,
        },
        key_check: None,
        private_key_file: match &info.ssh_key_path {
            ROption::RSome(s) => Some(s.to_string()),
            ROption::RNone => None,
        },
        private_key_pass: match &info.private_key_pass {
            ROption::RSome(s) => Some(SecretString::new(s.to_string().into_boxed_str())),
            ROption::RNone => None,
        },
        password: match &info.password {
            ROption::RSome(s) => Some(SecretString::new(s.to_string().into_boxed_str())),
            ROption::RNone => None,
        },
        elevate: match info.elevate {
            ROption::RSome(e) => Some(e),
            ROption::RNone => None,
        },
        elevation_method: match &info.become_method {
            ROption::RSome(s) => s.parse::<ElevationMethod>().ok(),
            ROption::RNone => None,
        },
        as_user: match &info.become_user {
            ROption::RSome(s) => Some(s.to_string()),
            ROption::RNone => None,
        },
        env: None,
        connection: if info.connection_type.is_empty() {
            None
        } else {
            info.connection_type.parse::<ConnectionType>().ok()
        },
    }
}

/// Map a komandan-core `KomandoResult` to a plugin `ModuleResult`.
///
/// `success` is derived (`exit_code == 0`); `msg` is `RNone` (no source field
/// on `KomandoResult`). Used by the `komando` `CoreApi` bridge.
#[must_use]
pub fn komando_result_to_module_result(r: &KomandoResult) -> ModuleResult {
    ModuleResult {
        changed: r.changed,
        rc: r.exit_code,
        stdout: RString::from(r.stdout.clone()),
        stderr: RString::from(r.stderr.clone()),
        success: r.exit_code == 0,
        msg: ROption::RNone,
    }
}

/// Map a plugin `ReportStatus` to komandan-core's `TaskStatus`.
///
/// Returns `None` for `ReportStatus::Skipped` — komandan-core has no Skipped
/// variant, so the caller must drop the record.
#[must_use]
#[allow(clippy::missing_const_for_fn)] // kept non-const to match the module's fn posture.
pub fn report_status_to_task_status(s: ReportStatus) -> Option<TaskStatus> {
    match s {
        ReportStatus::Ok => Some(TaskStatus::OK),
        ReportStatus::Changed => Some(TaskStatus::Changed),
        ReportStatus::Failed => Some(TaskStatus::Failed),
        // `ReportStatus::Skipped` (and any future variant) has no komandan-core
        // `TaskStatus` equivalent — the caller must drop the record.
        _ => None,
    }
}

/// Map komandan-core CLI `Flags` to a plugin `GlobalFlags`.
///
/// `interactive` and `version` are deliberately omitted from the plugin mirror.
#[must_use]
#[allow(clippy::missing_const_for_fn)] // kept non-const to match the module's fn posture.
pub fn flags_to_global_flags(f: &Flags) -> GlobalFlags {
    GlobalFlags {
        verbose: f.verbose,
        dry_run: f.dry_run,
        no_report: f.no_report,
        unsafe_lua: f.unsafe_lua,
    }
}

/// Convert a plugin [`RValue`] into an `mlua` [`Value`] on the given Lua VM.
///
/// Recursively handles `List` (→ Lua sequence table, keys `1..=n`) and `Map`
/// (→ Lua table with string keys). `Bytes` become Lua strings (binary-safe).
/// Used by the `komando` `CoreApi` bridge to build module params from a
/// plugin's flat `RHashMap<RStr, RValue>` arg map. Unknown `#[non_exhaustive]`
/// variants map to `nil`.
///
/// # Errors
/// Propagates `mlua` allocation errors (table/string creation).
pub fn rvalue_to_lua(lua: &Lua, v: &RValue) -> mlua::Result<Value> {
    Ok(match v {
        RValue::Bool(b) => Value::Boolean(*b),
        RValue::Int(i) => Value::Integer(*i),
        RValue::Float(f) => Value::Number(*f),
        RValue::Str(s) => Value::String(lua.create_string(s.as_str())?),
        RValue::Bytes(b) => Value::String(lua.create_string(b.as_slice())?),
        RValue::List(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.set(i + 1, rvalue_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
        RValue::Map(map) => {
            let t = lua.create_table()?;
            for entry in map {
                t.set(entry.0.as_str(), rvalue_to_lua(lua, entry.1)?)?;
            }
            Value::Table(t)
        }
        // Null and any future #[non_exhaustive] variant map to nil.
        _ => Value::Nil,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use komandan_plugin_abi::{RHashMap, RStr, RValue, RVec};
    use secrecy::ExposeSecret;

    use crate::create_lua;

    fn sample_host_info() -> HostInfo {
        HostInfo {
            name: ROption::RSome(RStr::from("web-01")),
            address: RStr::from("10.0.0.5"),
            port: ROption::RSome(2222),
            user: ROption::RSome(RStr::from("deploy")),
            ssh_key_path: ROption::RSome(RStr::from("/home/deploy/.ssh/id_ed25519")),
            private_key_pass: ROption::RSome(RStr::from("hunter2")),
            password: ROption::RSome(RStr::from("correct horse battery staple")),
            become_method: ROption::RSome(RStr::from("sudo")),
            become_user: ROption::RSome(RStr::from("root")),
            elevate: ROption::RSome(true),
            connection_type: RStr::from("ssh"),
        }
    }

    #[test]
    fn host_info_round_trips_full() {
        let info = sample_host_info();
        let h = host_info_to_host(&info);
        assert_eq!(h.name.as_deref(), Some("web-01"));
        assert_eq!(h.address, "10.0.0.5");
        assert_eq!(h.port, Some(2222));
        assert_eq!(h.user.as_deref(), Some("deploy"));
        assert_eq!(
            h.private_key_file.as_deref(),
            Some("/home/deploy/.ssh/id_ed25519")
        );
        assert_eq!(h.elevate, Some(true));
        assert_eq!(h.elevation_method, Some(ElevationMethod::Sudo));
        assert_eq!(h.as_user.as_deref(), Some("root"));
        assert_eq!(h.connection, Some(ConnectionType::SSH));
        assert!(h.key_check.is_none());
        assert!(h.env.is_none());
    }

    #[test]
    fn host_info_secret_fields_round_trip_identical_bytes() {
        let info = sample_host_info();
        let h = host_info_to_host(&info);
        assert_eq!(
            h.password.as_ref().map(|s| s.expose_secret().to_string()),
            Some("correct horse battery staple".to_string())
        );
        assert_eq!(
            h.private_key_pass
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            Some("hunter2".to_string())
        );
        // Negative: confirm a single changed bit would be caught.
        assert_ne!(
            h.password.as_ref().map(|s| s.expose_secret().to_string()),
            Some("correct horse battery stapl".to_string())
        );
    }

    #[test]
    fn host_info_lenient_empty_connection_and_bogus_become() {
        let mut info = sample_host_info();
        info.connection_type = RStr::from("");
        info.become_method = ROption::RSome(RStr::from("definitely-bogus"));
        let h = host_info_to_host(&info);
        assert!(
            h.connection.is_none(),
            "empty connection_type must map to None"
        );
        assert!(
            h.elevation_method.is_none(),
            "unparseable become_method must map to None, not error"
        );
    }

    #[test]
    fn host_info_rnone_fields_yield_none() {
        let info = HostInfo {
            name: ROption::RNone,
            address: RStr::from("127.0.0.1"),
            port: ROption::RNone,
            user: ROption::RNone,
            ssh_key_path: ROption::RNone,
            private_key_pass: ROption::RNone,
            password: ROption::RNone,
            become_method: ROption::RNone,
            become_user: ROption::RNone,
            elevate: ROption::RNone,
            connection_type: RStr::from(""),
        };
        let h = host_info_to_host(&info);
        assert!(h.name.is_none());
        assert_eq!(h.address, "127.0.0.1");
        assert!(h.port.is_none());
        assert!(h.user.is_none());
        assert!(h.private_key_file.is_none());
        assert!(h.private_key_pass.is_none());
        assert!(h.password.is_none());
        assert!(h.elevate.is_none());
        assert!(h.elevation_method.is_none());
        assert!(h.as_user.is_none());
        assert!(h.connection.is_none());
    }

    #[test]
    fn komando_result_to_module_result_success_and_fields() {
        let r = KomandoResult {
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            exit_code: 0,
            changed: true,
        };
        let m = komando_result_to_module_result(&r);
        assert!(m.success, "exit_code==0 implies success");
        assert_eq!(m.rc, 0);
        assert!(m.changed);
        assert_eq!(m.stdout.as_str(), "out");
        assert_eq!(m.stderr.as_str(), "err");
        assert!(matches!(m.msg, ROption::RNone));
    }

    #[test]
    fn komando_result_to_module_result_failure_rc() {
        let r = KomandoResult {
            stdout: String::new(),
            stderr: "boom".to_string(),
            exit_code: 7,
            changed: false,
        };
        let m = komando_result_to_module_result(&r);
        assert!(!m.success, "non-zero rc implies not success");
        assert_eq!(m.rc, 7);
        assert!(!m.changed);
    }

    #[test]
    fn report_status_maps_to_task_status() {
        assert_eq!(
            report_status_to_task_status(ReportStatus::Ok),
            Some(TaskStatus::OK)
        );
        assert_eq!(
            report_status_to_task_status(ReportStatus::Changed),
            Some(TaskStatus::Changed)
        );
        assert_eq!(
            report_status_to_task_status(ReportStatus::Failed),
            Some(TaskStatus::Failed)
        );
        assert_eq!(
            report_status_to_task_status(ReportStatus::Skipped),
            None,
            "Skipped has no TaskStatus variant; caller must drop it"
        );
    }

    #[test]
    fn flags_to_global_flags_copies_four_fields() {
        let f = Flags {
            dry_run: true,
            no_report: false,
            interactive: true,
            verbose: true,
            unsafe_lua: false,
            version: true,
        };
        let g = flags_to_global_flags(&f);
        assert!(g.verbose);
        assert!(g.dry_run);
        assert!(!g.no_report);
        assert!(!g.unsafe_lua);
    }

    #[test]
    fn rvalue_to_lua_scalars() -> anyhow::Result<()> {
        let lua = create_lua()?;
        assert!(matches!(rvalue_to_lua(&lua, &RValue::Null)?, Value::Nil));
        assert!(matches!(
            rvalue_to_lua(&lua, &RValue::Bool(true))?,
            Value::Boolean(true)
        ));
        assert!(matches!(
            rvalue_to_lua(&lua, &RValue::Int(42))?,
            Value::Integer(42)
        ));
        match rvalue_to_lua(&lua, &RValue::Float(1.5))? {
            Value::Number(n) => assert!((n - 1.5_f64).abs() < f64::EPSILON),
            other => panic!("expected Number, got {other:?}"),
        }
        match rvalue_to_lua(&lua, &RValue::str_literal("hi"))? {
            Value::String(s) => assert_eq!(s.to_str()?, "hi"),
            other => panic!("expected String, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn rvalue_to_lua_bytes() -> anyhow::Result<()> {
        let lua = create_lua()?;
        let bytes = RValue::Bytes(RVec::from(vec![0_u8, 1, 2, 3]));
        match rvalue_to_lua(&lua, &bytes)? {
            Value::String(s) => assert_eq!(s.as_bytes(), &[0, 1, 2, 3]),
            other => panic!("expected String (bytes), got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn rvalue_to_lua_list_and_map() -> anyhow::Result<()> {
        let lua = create_lua()?;
        let list = RValue::List(RVec::from(vec![
            RValue::Int(1),
            RValue::str_literal("two"),
            RValue::Bool(false),
        ]));
        let t = match rvalue_to_lua(&lua, &list)? {
            Value::Table(t) => t,
            other => panic!("expected Table, got {other:?}"),
        };
        assert_eq!(t.len()?, 3, "list must be a 1-indexed sequence");
        assert_eq!(t.get::<i64>(1)?, 1);
        assert_eq!(t.get::<String>(2)?, "two");
        assert!(!t.get::<bool>(3)?);

        let mut map = RHashMap::new();
        map.insert(RStr::from("n"), RValue::Int(7));
        map.insert(
            RStr::from("nested"),
            RValue::List(RVec::from(vec![RValue::Int(10)])),
        );
        let mt = match rvalue_to_lua(&lua, &RValue::Map(map))? {
            Value::Table(t) => t,
            other => panic!("expected Table, got {other:?}"),
        };
        assert_eq!(mt.get::<i64>("n")?, 7);
        let nested = mt.get::<mlua::Table>("nested")?;
        assert_eq!(nested.get::<i64>(1)?, 10);
        Ok(())
    }
}
