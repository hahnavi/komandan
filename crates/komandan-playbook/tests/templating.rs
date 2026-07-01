//! Integration snapshot tests for the templating pipeline.
//!
//! Exercises the public `komandan_playbook::templating` + `vars` API as a unit:
//! layered precedence, magic vars, native-type rendering, the `omit` sentinel,
//! a couple of filters, and the `set_fact`/`register` write-back plumbing.

use komandan_playbook::templating::{MagicVars, render_template, render_value};
use komandan_playbook::vars::{LayerKind, VarLayer, Vars};

fn play_vars(pairs: &[(&str, serde_json::Value)]) -> Vars {
    let mut layer = VarLayer::new(LayerKind::PlayVars);
    for (k, v) in pairs {
        layer.insert((*k).to_string(), v.clone());
    }
    let mut vars = Vars::new();
    vars.push(layer);
    vars
}

#[test]
fn precedence_play_overrides_magic() -> anyhow::Result<()> {
    // Stack order = precedence (last pushed wins). Magic (low) first, then play.
    let mut vars = Vars::new();
    vars.push(
        MagicVars {
            inventory_hostname: "web1".into(),
            ansible_host: Some("127.0.0.1".into()),
            play_hosts: vec!["web1".into()],
            playbook_dir: None,
            role_path: None,
        }
        .to_layer(),
    );
    let mut play = VarLayer::new(LayerKind::PlayVars);
    play.insert("ansible_host", serde_json::json!("10.0.0.5"));
    vars.push(play);
    // play vars (higher precedence) wins over magic.
    assert_eq!(
        render_template("{{ ansible_host }}", &vars)?,
        serde_json::json!("10.0.0.5")
    );
    // but inventory_hostname still resolves from magic.
    assert_eq!(
        render_template("{{ inventory_hostname }}", &vars)?,
        serde_json::json!("web1")
    );
    Ok(())
}

#[test]
fn render_value_realistic_task_args() -> anyhow::Result<()> {
    let mut vars = play_vars(&[
        ("pkg", serde_json::json!("nginx")),
        ("ports", serde_json::json!([80, 443])),
        ("enabled", serde_json::json!(true)),
    ]);
    vars.push(
        MagicVars {
            inventory_hostname: "web1".into(),
            ansible_host: None,
            play_hosts: vec!["web1".into(), "web2".into()],
            playbook_dir: Some("/srv".into()),
            role_path: None,
        }
        .to_layer(),
    );
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        "name: install {{ pkg }}\n\
         apt:\n\
         \x20 name: '{{ pkg }}'\n\
         \x20 state: present\n\
         notify: '{{ ports }}'\n\
         when: '{{ enabled }}'\n\
         host_tag: '{{ inventory_hostname }}'\n",
    )?;
    let out = render_value(&yaml, &vars)?;
    // scalar string interpolation
    assert_eq!(
        out["name"],
        serde_yaml::Value::String("install nginx".into())
    );
    assert_eq!(
        out["apt"]["name"],
        serde_yaml::Value::String("nginx".into())
    );
    // native-type: the list flows through unstringified
    assert!(matches!(out["notify"], serde_yaml::Value::Sequence(_)));
    // native-type: boolean
    assert_eq!(out["when"], serde_yaml::Value::Bool(true));
    // magic var available in task args
    assert_eq!(out["host_tag"], serde_yaml::Value::String("web1".into()));
    Ok(())
}

#[test]
fn filters_round_trip_in_template() -> anyhow::Result<()> {
    let vars = play_vars(&[("s", serde_json::json!("hello world"))]);
    assert_eq!(
        render_template("{{ s | upper | replace(' ', '_') }}", &vars)?,
        serde_json::json!("HELLO_WORLD")
    );
    assert_eq!(
        render_template("{{ s | b64encode }}", &vars)?,
        serde_json::json!("aGVsbG8gd29ybGQ=")
    );
    Ok(())
}

#[test]
fn set_fact_then_read() -> anyhow::Result<()> {
    let mut vars = play_vars(&[]);
    vars.set_fact("answer", serde_json::json!(42));
    // set_fact should be visible to subsequent templates.
    assert_eq!(
        render_template("{{ answer }}", &vars)?,
        serde_json::json!(42)
    );
    vars.register("job", serde_json::json!({"changed": true, "failed": false}));
    assert_eq!(
        render_template("{{ job.changed }}", &vars)?,
        serde_json::json!(true)
    );
    Ok(())
}

#[test]
fn omit_strips_key_in_rendered_tree() -> anyhow::Result<()> {
    let mut vars = play_vars(&[("keep", serde_json::json!(true))]);
    let mut magic = VarLayer::new(LayerKind::Magic);
    magic.insert("omit", komandan_playbook::templating::magic::omit_value());
    vars.push(magic);
    let yaml: serde_yaml::Value =
        serde_yaml::from_str("present: '{{ keep }}'\nabsent: '{{ omit }}'\n")?;
    let out = render_value(&yaml, &vars)?;
    assert!(out.get("present").is_some());
    assert!(out.get("absent").is_none());
    Ok(())
}
