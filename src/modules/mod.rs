mod apt;
mod cmd;
mod download;
mod lineinfile;
mod script;
mod systemd_service;
mod template;
mod upload;

use mlua::{chunk, Table};

pub fn base_module(lua: &mlua::Lua) -> Table {
    return lua
        .load(chunk! {
                local KomandanModule = {}

        KomandanModule.new = function(self,data)
            local o = setmetatable({}, { __index = self })
            o.name = data.name
            return o
        end

        KomandanModule.run = function(self)
        end

        KomandanModule.cleanup = function(self)
        end

        return KomandanModule
            })
        .eval::<Table>()
        .unwrap();
}

pub fn collect_modules(lua: &mlua::Lua) -> Table {
    let modules = lua.create_table().unwrap();
    modules
        .set("apt", lua.create_function(apt::apt).unwrap())
        .unwrap();
    modules
        .set("cmd", lua.create_function(cmd::cmd).unwrap())
        .unwrap();
    modules
        .set("download", lua.create_function(download::download).unwrap())
        .unwrap();
    modules
        .set(
            "lineinfile",
            lua.create_function(lineinfile::lineinfile).unwrap(),
        )
        .unwrap();
    modules
        .set("script", lua.create_function(script::script).unwrap())
        .unwrap();
    modules
        .set(
            "systemd_service",
            lua.create_function(systemd_service::systemd_service)
                .unwrap(),
        )
        .unwrap();
    modules
        .set("template", lua.create_function(template::template).unwrap())
        .unwrap();
    modules
        .set("upload", lua.create_function(upload::upload).unwrap())
        .unwrap();
    return modules;
}
