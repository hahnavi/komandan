use mlua::{chunk, ExternalResult, Lua, Table};
use rand::{distributions::Alphanumeric, Rng};

pub fn lineinfile(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let random_file_name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();

    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            if params.path == nil then
                error("'path' parameter is required")
            end

            if params.line == nil and params.pattern == nil then
                error("'line' or 'pattern' parameter is required")
            end

            if params.state == nil then
                params.state = "present"
            end

            if params.state ~= "present" and params.state ~= "absent" then
                error("'state' parameter must be 'present' or 'absent'")
            end

            if params.create == nil then
                params.create = false
            end

            if params.backup == nil then
                params.backup = false
            end

            local module = $base_module:new({ name = "lineinfile" })

            module.params = $params
            module.random_file_name = $random_file_name
            module.lineinfile_script = $LINEINFILE_SCRIPT

            module.run = function(self)
                local tmpdir = self.ssh:get_tmpdir()
                self.remote_script = tmpdir .. "/." .. self.random_file_name 
                self.ssh:write_remote_file(self.remote_script, self.lineinfile_script)
                self.ssh:chmod(self.remote_script, "+x")

                local cmd = self.remote_script .. " --path \"" .. self.params.path .. "\" --create " .. tostring(self.params.create) .. " --backup " .. tostring(self.params.backup) .. " --state " .. self.params.state
                if self.params.line ~= nil then
                    cmd = cmd .. " --line \"" .. self.params.line .. "\""
                end

                if self.params.pattern ~= nil then
                    cmd = cmd .. " --pattern \"" .. self.params.pattern .. "\""
                end

                if self.params.insert_after ~= nil then
                    cmd = cmd .. " --insert_after \"" .. self.params.insert_after .. "\""
                end

                if self.params.insert_before ~= nil then
                    cmd = cmd .. " --insert_before \"" .. self.params.insert_before .. "\""
                end

                self.ssh:cmd(cmd)
            end

            module.cleanup = function(self)
                self.ssh:cmd("rm " .. self.remote_script)
            end

            return module
        })
        .set_name("lineinfile")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}

const LINEINFILE_SCRIPT: &str = r#"#!/bin/sh

# Initialize default values
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
      exit 1
      ;;
  esac
done

# Validate required arguments
if [ -z "$FILE_PATH" ]; then
  echo "Error: '--path' is required"
  exit 1
fi

# Create the file if it doesn't exist and --create is true
if [ ! -f "$FILE_PATH" ]; then
  if [ "$CREATE" = "true" ]; then
    touch "$FILE_PATH"
    echo "File created: $FILE_PATH"
  else
    echo "Error: File '$FILE_PATH' does not exist and '--create' is set to false"
    exit 1
  fi
fi

# Create a backup if requested
if [ "$BACKUP" = "true" ]; then
  BACKUP_FILE="$FILE_PATH.$(date +%Y%m%d%H%M%S).bak"
  cp "$FILE_PATH" "$BACKUP_FILE"
  echo "Backup created: $BACKUP_FILE"
fi

# Handle the 'present' state
if [ "$STATE" = "present" ]; then
  if [ -z "$LINE" ]; then
    echo "Error: '--line' is required for 'present' state"
    exit 1
  fi

  # Check if the line already exists
  if grep -Fxq "$LINE" "$FILE_PATH"; then
    echo "Line already exists, no changes made."
    exit 0
  fi

  # Handle pattern replacement
  if [ -n "$REGEXP" ]; then
    if grep -q "$REGEXP" "$FILE_PATH"; then
      sed -i.bak "/$REGEXP/c\$LINE" "$FILE_PATH"
      echo "Line replaced matching pattern: $REGEXP"
      exit 0
    fi
  fi

  # Handle line insertion
  if [ -n "$INSERTAFTER" ]; then
    if [ "$INSERTAFTER" = "EOF" ]; then
      echo "$LINE" >> "$FILE_PATH"
      echo "Line appended to the end of the file."
    else
      sed -i.bak "/$INSERTAFTER/a\$LINE" "$FILE_PATH"
      echo "Line inserted after pattern: $INSERTAFTER"
    fi
  elif [ -n "$INSERTBEFORE" ]; then
    if [ "$INSERTBEFORE" = "BOF" ]; then
      sed -i.bak "1i\$LINE" "$FILE_PATH"
      echo "Line inserted at the beginning of the file."
    else
      sed -i.bak "/$INSERTBEFORE/i\$LINE" "$FILE_PATH"
      echo "Line inserted before pattern: $INSERTBEFORE"
    fi
  else
    echo "$LINE" >> "$FILE_PATH"
    echo "Line appended to the file."
  fi
  exit 0
fi

# Handle the 'absent' state
if [ "$STATE" = "absent" ]; then
  if [ -z "$REGEXP" ] && [ -z "$LINE" ]; then
    echo "Error: '--pattern' or '--line' is required for 'absent' state"
    exit 1
  fi

  # Remove lines matching the exact line
  if [ -n "$LINE" ]; then
    sed -i.bak "/^$(echo "$LINE" | sed 's/[^^]/[&]/g; s/\^/\\^/g')$/d" "$FILE_PATH"
    echo "Removed line: $LINE"
  fi

  # Remove lines matching the regex
  if [ -n "$REGEXP" ]; then
    sed -i.bak "/$REGEXP/d" "$FILE_PATH"
    echo "Removed lines matching pattern: $REGEXP"
  fi
  exit 0
fi

# If no valid state is provided
echo "Error: Invalid state '$STATE'. Use 'present' or 'absent'."
exit 1
"#;

// Tests
#[cfg(test)]
mod tests {
    use crate::create_lua;

    use super::*;

    #[test]
    fn test_lineinfile_no_path() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("'path' parameter is required"));
    }

    #[test]
    fn test_lineinfile_invalid_state() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        params.set("line", "Hello, world!").unwrap();
        params.set("state", "--invalid-state--").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("'state' parameter must be 'present' or 'absent'"));
    }

    #[test]
    fn test_lineinfile_no_line_or_pattern() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("'line' or 'pattern' parameter is required"));
    }

    #[test]
    fn test_lineinfile_present() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        params.set("state", "present").unwrap();
        params.set("line", "Hello, world!").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_lineinfile_absent() {
        let lua = create_lua().unwrap();
        let params = lua.create_table().unwrap();
        params.set("path", "/tmp/test.txt").unwrap();
        params.set("state", "absent").unwrap();
        params.set("line", "Hello, world!").unwrap();
        let result = lineinfile(&lua, params);
        assert!(result.is_ok());
    }
}
