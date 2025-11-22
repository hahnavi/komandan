use minijinja::Environment;
use mlua::{Error::RuntimeError, ExternalResult, Lua, Table, Value, chunk};
use rand::{Rng, distr::Alphanumeric};

pub fn template(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let Ok(src) = params.get::<String>("src") else {
        return Err(RuntimeError(String::from("'src' parameter is required")));
    };

    if params.get::<String>("dst").is_err() {
        return Err(RuntimeError(String::from("'dst' parameter is required")));
    }

    let vars = params.get::<Value>("vars")?;
    if !vars.is_nil() && !vars.is_table() {
        return Err(RuntimeError(String::from(
            "'vars' parameter must be a table",
        )));
    }

    if !std::path::Path::new(&src).exists() {
        return Err(RuntimeError(String::from("Source template does not exist")));
    }

    let src_content = std::fs::read_to_string(&src)
        .map_err(|e| RuntimeError(format!("Failed to read template file: {e}")))?;

    let mut env = Environment::new();
    env.add_template("template", &src_content)
        .map_err(|e| RuntimeError(format!("Failed to add template: {e}")))?;

    let rendered = env
        .get_template("template")
        .map_err(|e| RuntimeError(format!("Failed to get template: {e}")))?
        .render(minijinja::Value::from_serialize(vars))
        .map_err(|e| RuntimeError(format!("Failed to render template: {e}")))?;

    let random_file_name: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();

    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "template" })

            module.params = $params
            module.rendered = $rendered
            module.random_file_name = $random_file_name

            module.run = function(self)
                local tmpdir = self.ssh:get_tmpdir()
                local tmpfile = tmpdir .. "/." .. self.random_file_name
                self.ssh:write_remote_file(tmpfile, self.rendered)
                self.ssh:cmd("mv " .. tmpfile .. " " .. self.params.dst)
                self.ssh:set_changed(true)
            end

            return module
        })
        .set_name("template")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}

// Tests
#[cfg(test)]
mod tests {
    use crate::create_lua;

    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_template_src_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        let result = template(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: 'src' parameter is required");
        }
        Ok(())
    }

    #[test]
    fn test_template_dst_required() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("src", "example.src")?;
        let result = template(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: 'dst' parameter is required");
        }
        Ok(())
    }

    #[test]
    fn test_template_vars_must_be_table() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("src", "example.src")?;
        params.set("dst", "example.dst")?;
        params.set("vars", "not a table")?;
        let result = template(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(
                e.to_string(),
                "runtime error: 'vars' parameter must be a table"
            );
        }
        Ok(())
    }

    #[test]
    fn test_template_src_file_exists() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("src", "non_existent_file.src")?;
        params.set("dst", "example.dst")?;
        let result = template(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(
                e.to_string(),
                "runtime error: Source template does not exist"
            );
        }
        Ok(())
    }

    #[test]
    fn test_template_success() -> mlua::Result<()> {
        let mut temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        writeln!(temp_file, "{{{{ name }}}} is {{{{ age }}}} years old")
            .map_err(mlua::Error::external)?;
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set(
            "src",
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
        )?;
        params.set("dst", "/remote/file")?;
        let vars = lua.create_table()?;
        vars.set("name", "John")?;
        vars.set("age", 30)?;
        params.set("vars", vars)?;
        let result = template(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }
}
