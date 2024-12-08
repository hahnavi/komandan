use mlua::{chunk, Error::RuntimeError, Lua, Table};
use rand::{distributions::Alphanumeric, Rng};

pub fn script(lua: &Lua, params: Table) -> mlua::Result<Table> {
    if params.get::<String>("script").is_err() {
        return Err(RuntimeError(String::from("script parameter is required")));
    }
    let script = params.get::<String>("script")?;
    let interpreter = params
        .get::<String>("interpreter")
        .unwrap_or_else(|_| "sh".to_string())
        .to_string();

    let random_file_name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();
    let module = lua
        .load(chunk! {
            local module = komandan.KomandanModule:new({ name = "script" })

            function module:run()
                local homedir = module.ssh:cmd("echo $HOME").stdout:gsub("\n", "")
                local tmpdir = homedir .. "/.komandan/tmp"
                module.ssh:cmd("mkdir -p " .. tmpdir)
                module.ssh:write_remote_file(tmpdir .. "/." .. $random_file_name, $script)
                module.ssh:cmd($interpreter .. " " .. tmpdir .. "/." .. $random_file_name)
            end

            function module:cleanup()
            local homedir = module.ssh:cmd("echo $HOME").stdout:gsub("\n", "")
            local tmpdir = homedir .. "/.komandan/tmp"
                module.ssh:cmd("rm " .. tmpdir .. "/." .. $random_file_name)
            end

            return module
        })
        .eval::<Table>()?;

    Ok(module)
}
