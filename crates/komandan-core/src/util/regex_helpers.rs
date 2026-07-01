use mlua::Lua;

pub fn regex_is_match(
    _: &Lua,
    (text, pattern): (mlua::String, mlua::String),
) -> mlua::Result<bool> {
    let re = regex::Regex::new(&pattern.to_str()?)
        .map_err(|e| mlua::Error::RuntimeError(format!("Invalid regex pattern: {e}")))?;
    Ok(re.is_match(&text.to_str()?))
}
