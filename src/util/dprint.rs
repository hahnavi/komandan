use mlua::{Lua, Value, chunk};

pub fn dprint(lua: &Lua, value: Value) -> mlua::Result<()> {
    if crate::args::global_flags().verbose {
        lua.load(chunk! {
            print($value)
        })
        .exec()?;
    }
    Ok(())
}
