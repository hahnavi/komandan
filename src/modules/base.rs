use mlua::{Table, chunk};

pub fn base_module(lua: &mlua::Lua) -> mlua::Result<Table> {
    lua.load(chunk! {
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
}
