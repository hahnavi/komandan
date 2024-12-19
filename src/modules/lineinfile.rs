use mlua::{chunk, Error::RuntimeError, ExternalResult, Lua, Table, Value};
use rand::{distributions::Alphanumeric, Rng};

pub fn lineinfile(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let path = match params.get::<String>("path") {
        Ok(path) => path,
        Err(_) => return Err(RuntimeError(String::from("'path' parameter is required"))),
    };

    let line = params.get::<Value>("line")?;
    let pattern = params.get::<Value>("pattern")?;

    let state = match params.get::<String>("state") {
        Ok(state) => match state.as_str() {
            "present" => state,
            "absent" => state,
            _ => {
                return Err(RuntimeError(String::from(
                    "'state' parameter must be 'present' or 'absent'",
                )))
            }
        },
        Err(_) => String::from("present"),
    };

    if line.is_nil() && pattern.is_nil() {
        return Err(RuntimeError(String::from(
            "'line' or 'pattern' parameter is required",
        )));
    }

    let insert_after = params.get::<Value>("insert_after")?;
    let insert_before = params.get::<Value>("insert_before")?;

    let create = params.get::<bool>("create").unwrap_or(false);
    let backup = params.get::<bool>("backup").unwrap_or(false);

    let random_file_name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();

    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "lineinfile" })

            function module:run()
                local tmpdir = module.ssh:get_tmpdir()
                module.remote_script = tmpdir .. "/." .. $random_file_name 
                module.ssh:write_remote_file(module.remote_script, $LINEINFILE_SCRIPT)
                module.ssh:chmod(module.remote_script, "+x")

                local cmd = module.remote_script .. " --path \"" .. $path .. "\" --create " .. tostring($create) .. " --backup " .. tostring($backup) .. " --state " .. $state
                if $line ~= nil then
                    cmd = cmd .. " --line \"" .. $line .. "\""
                end

                if $pattern ~= nil then
                    cmd = cmd .. " --pattern \"" .. $pattern .. "\""
                end

                if $insert_after ~= nil then
                    cmd = cmd .. " --insert_after \"" .. $insert_after .. "\""
                end

                if $insert_before ~= nil then
                    cmd = cmd .. " --insert_before \"" .. $insert_before .. "\""
                end

                module.ssh:cmd(cmd)
            end

            function module:cleanup()
                module.ssh:cmd("rm " .. module.remote_script)
            end

            return module
        })
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}

const LINEINFILE_SCRIPT: &str = r#"#!/bin/sh
# Default values
STATE="present"
CREATE="false"
BACKUP="false"

# Parse command-line arguments
while [ $# -gt 0 ]; do
  case "$1" in
    --path)
      FILE_PATH="$2"
      shift 2
      ;;
    --pattern)
      REGEXP="$2"
      shift 2
      ;;
    --line)
      LINE="$2"
      shift 2
      ;;
    --state)
      STATE="$2"
      shift 2
      ;;
    --create)
      CREATE="$2"
      shift 2
      ;;
    --insert_after)
      INSERTAFTER="$2"
      shift 2
      ;;
    --insert_before)
      INSERTBEFORE="$2"
      shift 2
      ;;
    --backup)
      BACKUP="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1"
      ;;
  esac
done

# Validate required arguments
if [ -z "$FILE_PATH" ]; then
  echo "Error: 'path' is required"
fi

# Check if the file exists
if [ ! -f "$FILE_PATH" -a "$CREATE" = "false" ]; then
  echo "Error: File '$FILE_PATH' does not exist and 'create' is set to 'false'"
  exit 1
elif [ ! -f "$FILE_PATH" -a "$CREATE" = "true" ]; then
  touch "$FILE_PATH"
fi

# Create a backup if requested
if [ "$BACKUP" = "true" ]; then
  BACKUP_FILE="$FILE_PATH.$(date +%Y%m%d%H%M%S).bak"
  cp "$FILE_PATH" "$BACKUP_FILE"
  echo "Backup created: $BACKUP_FILE"
fi

# Handle present state
if [ "$STATE" = "present" ]; then
  # Handle insertion or replacement if 'line' is provided
  if [ -n "$LINE" ]; then
    # Check if line already exists
    if grep -q "$LINE" "$FILE_PATH"; then
      echo "Line already exists in the file"
      exit 0
    fi

    # Handle replacement
    if [ -n "$REGEXP" ]; then
      if grep -q "$REGEXP" "$FILE_PATH"; then
        # Use a temporary file for sed
        sed "s/$REGEXP/$LINE/" "$FILE_PATH" > "$FILE_PATH.tmp"
        mv "$FILE_PATH.tmp" "$FILE_PATH"
        echo "Line replaced in the file"
        exit 0
      fi
    fi

    # Handle insertion
    if [ -n "$INSERTAFTER" ]; then
      if [ "$INSERTAFTER" = "EOF" ]; then
        echo "$LINE" >> "$FILE_PATH"
        echo "Line inserted at end of file"
      else
        # Use a temporary file for sed
        sed "/$INSERTAFTER/a $LINE" "$FILE_PATH" > "$FILE_PATH.tmp"
        mv "$FILE_PATH.tmp" "$FILE_PATH"
        echo "Line inserted after '$INSERTAFTER'"
      fi
      exit 0
    elif [ -n "$INSERTBEFORE" ]; then
      if [ "$INSERTBEFORE" = "BOF" ]; then
        # Use a temporary file for sed
        sed "1s/^/$LINE\n/" "$FILE_PATH" > "$FILE_PATH.tmp"
        mv "$FILE_PATH.tmp" "$FILE_PATH"
        echo "Line inserted at beginning of file"
      else
        # Use a temporary file for sed
        sed "/$INSERTBEFORE/i $LINE" "$FILE_PATH" > "$FILE_PATH.tmp"
        mv "$FILE_PATH.tmp" "$FILE_PATH"
        echo "Line inserted before '$INSERTBEFORE'"
      fi
      exit 0
    else
      echo "$LINE" >> "$FILE_PATH"
      echo "Line appended to the file"
      exit 0
    fi
  # If 'line' is not provided, check for regexp match when state is present
  elif [ -n "$REGEXP" ]; then
    if ! grep -q "$REGEXP" "$FILE_PATH"; then
      echo "No lines match '$REGEXP' when expecting at least one match"
    fi
    exit 0
  else
    echo "Error: 'line' or 'pattern' is required when state is 'present'"
    exit 1
  fi
fi

# Handle absent state
if [ "$STATE" = "absent" ]; then
  if [ -z "$REGEXP" -a -z "$LINE" ]; then
    echo "Error: Either 'pattern' or 'line' is required when state is 'absent'"
    exit 1
  fi

  if [ -n "$REGEXP" ]; then
    # Use a temporary file for sed
    sed "/$REGEXP/d" "$FILE_PATH" > "$FILE_PATH.tmp"
    mv "$FILE_PATH.tmp" "$FILE_PATH"
    echo "Lines matching '$REGEXP' removed from the file"
  elif [ -n "$LINE" ]; then
    # Use a temporary file for sed
    sed "/$(echo "$LINE" | sed 's/[^^]/[&]/g; s/\^/\\^/g')/d" "$FILE_PATH" > "$FILE_PATH.tmp"
    mv "$FILE_PATH.tmp" "$FILE_PATH"
    echo "Lines matching '$LINE' removed from the file"
  fi
  exit 0
fi
"#;

// Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lineinfile_no_path() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "runtime error: 'path' parameter is required"
        );
    }

    #[test]
    fn test_lineinfile_invalid_state() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        params.set("state", "--invalid-state--").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "runtime error: 'state' parameter must be 'present' or 'absent'"
        );
    }

    #[test]
    fn test_lineinfile_no_line_or_pattern() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "runtime error: 'line' or 'pattern' parameter is required"
        );
    }

    #[test]
    fn test_lineinfile_present() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        params.set("state", "present").unwrap();
        params.set("line", "Hello, world!").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_lineinfile_absent() {
        let lua = Lua::new();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        params.set("state", "absent").unwrap();
        params.set("line", "Hello, world!").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_ok());
    }
}
