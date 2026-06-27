use mlua::Table;

pub fn host_display(host: &Table) -> String {
    let address = host.get::<String>("address").unwrap_or_default();

    match host.get::<String>("name") {
        Ok(name) => format!("{name} ({address})"),
        Err(_) => address,
    }
}

pub fn task_display(task: &Table) -> String {
    let module = task.get::<Table>(1).unwrap_or_else(|_| task.clone());
    task.get::<String>("name")
        .unwrap_or_else(|_| module.get::<String>("name").unwrap_or_default())
}
