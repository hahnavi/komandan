//! Build an ABI [`HostInfo`] from an inventory host + its connection vars.
//!
//! Maps the standard `ansible_*` connection variables
//! (`ansible_connection`, `ansible_host`, `ansible_port`, `ansible_user`,
//! `ansible_ssh_private_key_file`, `ansible_become`, ...) onto the host's
//! connection fields. Anything unset falls back to komandan defaults
//! (`localhost` ⇒ local; otherwise ssh at the host's name).

use komandan_plugin_abi::prelude::*;

use crate::parser::Vars;

/// Build the [`HostInfo`] for `host_label`, consulting `vars` for `ansible_*`
/// connection settings.
#[must_use]
pub fn build_host_info(host_label: &str, vars: &Vars) -> HostInfo {
    let connection = str_var(vars, "ansible_connection").map_or_else(
        || {
            if host_label == "localhost" {
                "local".to_string()
            } else {
                "ssh".to_string()
            }
        },
        str::to_string,
    );
    let address = str_var(vars, "ansible_host").unwrap_or(host_label);
    HostInfo {
        name: rstr_some(host_label),
        address: crate::leak::rstr(address),
        port: str_var(vars, "ansible_port")
            .and_then(|p| p.parse::<u16>().ok())
            .into(),
        user: str_var(vars, "ansible_user").map(crate::leak::rstr).into(),
        ssh_key_path: str_var(vars, "ansible_ssh_private_key_file")
            .map(crate::leak::rstr)
            .into(),
        // ansible_ssh_pass / ansible_password — secret, see HostInfo docs.
        private_key_pass: ROption::RNone,
        password: str_var(vars, "ansible_password")
            .or_else(|| str_var(vars, "ansible_ssh_pass"))
            .map(crate::leak::rstr)
            .into(),
        become_method: str_var(vars, "ansible_become_method")
            .map(crate::leak::rstr)
            .into(),
        become_user: str_var(vars, "ansible_become_user")
            .map(crate::leak::rstr)
            .into(),
        elevate: str_var(vars, "ansible_become").and_then(parse_bool).into(),
        connection_type: crate::leak::rstr(&connection),
    }
}

fn str_var<'a>(vars: &'a Vars, key: &str) -> Option<&'a str> {
    vars.0.get(key).and_then(serde_yaml::Value::as_str)
}

/// Ansible's loose truthy strings for `ansible_become`.
fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "yes" | "true" | "True" | "1" => Some(true),
        "no" | "false" | "False" | "0" | "" => Some(false),
        _ => None,
    }
}

fn rstr_some(s: &str) -> ROption<RStr<'static>> {
    ROption::RSome(crate::leak::rstr(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        let mut m = IndexMap::new();
        for (k, v) in pairs {
            m.insert(
                (*k).to_string(),
                serde_yaml::Value::String((*v).to_string()),
            );
        }
        Vars(m)
    }

    #[test]
    fn localhost_defaults_to_local_connection() {
        let h = build_host_info("localhost", &Vars::default());
        assert_eq!(h.connection_type.as_str(), "local");
        assert_eq!(h.address.as_str(), "localhost");
    }

    #[test]
    fn remote_host_defaults_to_ssh() {
        let h = build_host_info("web-01", &vars(&[]));
        assert_eq!(h.connection_type.as_str(), "ssh");
        assert_eq!(h.address.as_str(), "web-01");
    }

    #[test]
    fn ansible_vars_override_defaults() {
        let h = build_host_info(
            "web-01",
            &vars(&[
                ("ansible_host", "10.0.0.5"),
                ("ansible_port", "2222"),
                ("ansible_user", "deploy"),
                ("ansible_become", "yes"),
                ("ansible_become_user", "root"),
            ]),
        );
        assert_eq!(h.address.as_str(), "10.0.0.5");
        assert_eq!(h.port, ROption::RSome(2222));
        assert_eq!(h.user.as_ref().unwrap().as_str(), "deploy");
        assert_eq!(h.elevate, ROption::RSome(true));
        assert_eq!(h.become_user.as_ref().unwrap().as_str(), "root");
    }
}
