use mlua::{ExternalResult, Lua, Table, chunk};

pub fn lineinfile(lua: &Lua, params: Table) -> mlua::Result<Table> {
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
            module.lineinfile_script = $LINEINFILE_SCRIPT

            module.run_lineinfile_script = function(self)
                local args = " --path \"" .. self.params.path .. "\" --create " .. tostring(self.params.create) .. " --backup " .. tostring(self.params.backup) .. " --state " .. self.params.state
                if self.params.line ~= nil then
                    args = args .. " --line \"" .. self.params.line .. "\""
                end

                if self.params.pattern ~= nil then
                    args = args .. " --pattern \"" .. self.params.pattern .. "\""
                end

                if self.params.insert_after ~= nil then
                    args = args .. " --insert_after \"" .. self.params.insert_after .. "\""
                end

                if self.params.insert_before ~= nil then
                    args = args .. " --insert_before \"" .. self.params.insert_before .. "\""
                end

                if self.params.dry_run then
                    args = args .. " --dry-run"
                end

                -- Execute script inline using heredoc
                local cmd = "sh -s --" .. args .. " <<'LINEINFILE_EOF'\n" .. self.lineinfile_script .. "\nLINEINFILE_EOF"
                return self.ssh:cmd(cmd)
            end

            module.dry_run = function(self)
                self.params.dry_run = true
                local result = self:run_lineinfile_script()
                if result.stdout ~= "OK" then
                    self.ssh:set_changed(true)
                end
            end

            module.run = function(self)
                local result = self:run_lineinfile_script()
                if result.stdout ~= "OK" then
                    self.ssh:set_changed(true)
                end
            end

            module.cleanup = function(self)
                -- No cleanup needed
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
DRYRUN="false"

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
    --dry-run)
      DRYRUN="true"
      shift 1
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
    if [ "$DRYRUN" = "true" ]; then
      echo "[DRY-RUN] File would be created: $FILE_PATH"
    else
      touch "$FILE_PATH"
      echo "Changed"
    fi
  else
    echo "Error: File '$FILE_PATH' does not exist and '--create' is set to false"
    exit 1
  fi
fi

# Create a backup if requested
if [ "$BACKUP" = "true" ]; then
  BACKUP_FILE="$FILE_PATH.$(date +%Y%m%d%H%M%S).bak"
  if [ "$DRYRUN" = "true" ]; then
    echo "[DRY-RUN] Backup would be created: $BACKUP_FILE"
  else
    cp "$FILE_PATH" "$BACKUP_FILE"
    echo "Changed"
  fi
fi

# Handle the 'present' state
if [ "$STATE" = "present" ]; then
  if [ -z "$LINE" ]; then
    echo "Error: '--line' is required for 'present' state"
    exit 1
  fi

  # Check if the line already exists
  if grep -Fxq "$LINE" "$FILE_PATH"; then
    echo "OK" # Unchanged
    exit 0
  fi

  # Handle pattern replacement
  if [ -n "$REGEXP" ]; then
    if grep -q "$REGEXP" "$FILE_PATH"; then
      if [ "$DRYRUN" = "true" ]; then
        echo "[DRY-RUN] Line matching '$REGEXP' would be replaced with: $LINE"
      else
        sed -i "/$REGEXP/c\\$LINE" "$FILE_PATH"
        echo "Changed"
      fi
      exit 0
    fi
  fi

  # Handle line insertion
  if [ -n "$INSERTAFTER" ]; then
    if [ "$DRYRUN" = "true" ]; then
      echo "[DRY-RUN] Line '$LINE' would be inserted after pattern: $INSERTAFTER"
    else
      if [ "$INSERTAFTER" = "EOF" ]; then
        echo "$LINE" >> "$FILE_PATH"
        echo "Changed"
      else
        sed -i "/$INSERTAFTER/a\\$LINE" "$FILE_PATH"
        echo "Changed"
      fi
    fi
  elif [ -n "$INSERTBEFORE" ]; then
    if [ "$DRYRUN" = "true" ]; then
      echo "[DRY-RUN] Line '$LINE' would be inserted before pattern: $INSERTBEFORE"
    else
      if [ "$INSERTBEFORE" = "BOF" ]; then
        sed -i "1i\\$LINE" "$FILE_PATH"
        echo "Changed"
      else
        sed -i "/$INSERTBEFORE/i\\$LINE" "$FILE_PATH"
        echo "Changed"
      fi
    fi
  else
    if [ "$DRYRUN" = "true" ]; then
      echo "[DRY-RUN] Line '$LINE' would be appended to the file."
    else
      echo "$LINE" >> "$FILE_PATH"
      echo "Changed"
    fi
  fi
  exit 0
fi

# Handle 'absent' state if implemented in the future
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
    fn test_lineinfile_no_path() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("'path' parameter is required"));
        }
        Ok(())
    }

    #[test]
    fn test_lineinfile_invalid_state() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("path", "/tmp/test.txt")?;
        params.set("line", "Hello, world!")?;
        params.set("state", "--invalid-state--")?;
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("'state' parameter must be 'present' or 'absent'")
            );
        }
        Ok(())
    }

    #[test]
    fn test_lineinfile_no_line_or_pattern() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("path", "/tmp/test.txt")?;
        let result = lineinfile(&lua, params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("'line' or 'pattern' parameter is required")
            );
        }
        Ok(())
    }

    #[test]
    fn test_lineinfile_present() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("path", "/tmp/test.txt")?;
        params.set("state", "present")?;
        params.set("line", "Hello, world!")?;
        let result = lineinfile(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_lineinfile_absent() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("path", "/tmp/test.txt")?;
        params.set("state", "absent")?;
        params.set("line", "Hello, world!")?;
        let result = lineinfile(&lua, params);
        assert!(result.is_ok());
        Ok(())
    }
}
