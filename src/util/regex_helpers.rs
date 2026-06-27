use mlua::Lua;

pub fn regex_is_match(
    _: &Lua,
    (text, pattern): (mlua::String, mlua::String),
) -> mlua::Result<bool> {
    let Ok(re) = regex::Regex::new(&pattern.to_str()?) else {
        return Ok(false);
    };
    Ok(re.is_match(&text.to_str()?))
}
