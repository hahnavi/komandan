mod cmd;
mod download;
mod script;
mod upload;

pub use cmd::cmd;
pub use download::download;
pub use script::script;
pub use upload::upload;

use mlua::{chunk, Table};

pub fn base_module(lua: &mlua::Lua) -> Table {
    return lua
        .load(chunk! {
                local KomandanModule = {}

        function KomandanModule:new(data)
            local o = setmetatable({}, { __index = self })
            o.name = data.name
            o.script = data.script
            return o
        end

        function KomandanModule:run()
        end

        function KomandanModule:cleanup()
        end

        return KomandanModule
            })
        .eval::<Table>()
        .unwrap();
}
