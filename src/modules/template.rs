use minijinja::Environment;
use mlua::{chunk, Error::RuntimeError, ExternalResult, Lua, Table, Value};
use rand::{distributions::Alphanumeric, Rng};

pub fn template(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let src = match params.get::<String>("src") {
        Ok(s) => s,
        Err(_) => return Err(RuntimeError(String::from("'src' parameter is required"))),
    };

    let dst = match params.get::<String>("dst") {
        Ok(s) => s,
        Err(_) => return Err(RuntimeError(String::from("'dst' parameter is required"))),
    };

    let vars = params.get::<Value>("vars")?;
    if !vars.is_nil() && !vars.is_table() {
        return Err(RuntimeError(String::from(
            "'vars' parameter must be a table",
        )));
    };

    if !std::path::Path::new(&src).exists() {
        return Err(RuntimeError(String::from("Source template does not exist")));
    }

    let src_content = std::fs::read_to_string(&src).unwrap();

    let mut env = Environment::new();
    env.add_template("template", &src_content).unwrap();

    let rendered = env
        .get_template("template")
        .unwrap()
        .render(minijinja::Value::from_serialize(vars))
        .unwrap();

    let random_file_name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();

    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "template" })

            function module:run()
                local tmpdir = module.ssh:get_tmpdir()
                local tmpfile = tmpdir .. "/." .. $random_file_name
                module.ssh:write_remote_file(tmpfile, $rendered)
                module.ssh:cmd("mv " .. tmpfile .. " " .. $dst)
            end

            return module
        })
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
