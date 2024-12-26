mod apt;
mod cmd;
mod download;
mod lineinfile;
mod script;
mod template;
mod upload;

pub use apt::apt;
pub use cmd::cmd;
pub use download::download;
pub use lineinfile::lineinfile;
pub use script::script;
pub use template::template;
pub use upload::upload;

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
