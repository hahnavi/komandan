//! Gap executors for remote text-file editing.
//!
//! [`BlockInFile`] inserts/updates/removes a marker-delimited multi-line block;
//! [`Replace`] substitutes a regexp across a whole file. Both read the remote
//! file via `cat`, manipulate in Rust, and write back via `write_file`.

use komandan_plugin_abi::prelude::*;
use serde_json::Value;

use super::{Connection, ModuleError, ModuleExecutor, ModuleRegistry, TaskContext};

/// Register the file-editing gap executors on `reg`.
pub fn register_all(reg: &mut ModuleRegistry) {
    reg.register(BlockInFile);
    reg.register(Replace);
}

/// Build a successful result carrying `stdout`.
fn ok_with_stdout(stdout: &str) -> ModuleResult {
    ModuleResult {
        changed: false,
        rc: 0,
        stdout: RString::from(stdout),
        stderr: RString::new(),
        success: true,
        msg: ROption::RNone,
    }
}

/// Default marker pair when the user does not supply a `marker:` template.
const DEFAULT_MARKER_BEGIN: &str = "# BEGIN ANSIBLE MANAGED BLOCK";
const DEFAULT_MARKER_END: &str = "# END ANSIBLE MANAGED BLOCK";
/// The placeholder substituted inside a user `marker:` template.
const MARK_PLACEHOLDER: &str = "{mark}";
/// Text substituted for `{mark}` to form the begin / end marker lines.
const MARK_BEGIN: &str = "BEGIN ANSIBLE MANAGED BLOCK";
const MARK_END: &str = "END ANSIBLE MANAGED BLOCK";

// ---- blockinfile --------------------------------------------------------

/// `blockinfile` — manage a marker-delimited text block in a remote file.
struct BlockInFile;

impl ModuleExecutor for BlockInFile {
    fn name(&self) -> &'static str {
        "blockinfile"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let path = args
            .get("path")
            .or_else(|| args.get("dest"))
            .or_else(|| args.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("blockinfile requires a 'path'"))?;
        let state = args
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("present");
        let block = match args.get("block").and_then(Value::as_str) {
            Some(b) => b,
            None if state == "absent" => "",
            None => return Err(ModuleError::args("blockinfile requires a 'block'")),
        };
        let create = args.get("create").and_then(Value::as_bool).unwrap_or(false);
        let insertafter = args.get("insertafter").and_then(Value::as_str);
        let (begin, end) = resolve_markers(args.get("marker").and_then(Value::as_str));

        let original = read_remote_file(conn, path, create)?;
        let new = apply_block(&original, &begin, &end, block, state, insertafter);
        match new {
            None => Ok(ok_with_stdout("blockinfile: no change")),
            Some(content) => {
                if !ctx.check_mode {
                    conn.write_file(path, content.as_bytes())?;
                }
                let mut msg = if ctx.check_mode {
                    String::from("blockinfile: block would be written")
                } else {
                    String::from("blockinfile: block written")
                };
                if ctx.diff_mode {
                    let diff = super::compute_file_diff(path, &original, &content);
                    if !diff.is_empty() {
                        msg.push('\n');
                        msg.push_str(&diff);
                    }
                }
                let mut r = ok_with_stdout(&msg);
                r.changed = true;
                Ok(r)
            }
        }
    }
}

/// Resolve the (begin, end) marker lines from an optional `marker:` template.
fn resolve_markers(template: Option<&str>) -> (String, String) {
    match template {
        Some(t) if t.contains(MARK_PLACEHOLDER) => (
            t.replace(MARK_PLACEHOLDER, MARK_BEGIN),
            t.replace(MARK_PLACEHOLDER, MARK_END),
        ),
        _ => (
            DEFAULT_MARKER_BEGIN.to_string(),
            DEFAULT_MARKER_END.to_string(),
        ),
    }
}

/// Apply a block edit to `original`. Returns `Some(new_content)` if it changed,
/// `None` if it would be unchanged.
fn apply_block(
    original: &str,
    begin: &str,
    end: &str,
    block: &str,
    state: &str,
    insertafter: Option<&str>,
) -> Option<String> {
    // Preserve a trailing newline if the original had one.
    let had_trailing_newline = original.ends_with('\n');
    let mut lines: Vec<String> = original.lines().map(str::to_string).collect();

    let begin_idx = lines.iter().position(|l| l == begin);
    let end_idx = lines.iter().position(|l| l == end);

    match (begin_idx, end_idx) {
        (Some(b), Some(e)) if b <= e => {
            if state == "absent" {
                lines = remove_range(lines, b, e);
            } else {
                let replacement = marked_block(begin, block, end);
                lines = replace_range(lines, b, e, replacement);
            }
        }
        _ => {
            if state == "absent" {
                return None;
            }
            let replacement = marked_block(begin, block, end);
            let pos = anchor_index(&lines, insertafter);
            lines = insert_at(lines, pos, replacement);
        }
    }

    let mut out = lines.join("\n");
    if had_trailing_newline {
        out.push('\n');
    }
    if out == original { None } else { Some(out) }
}

/// Build the marker+content lines for one block (begin, block..., end).
fn marked_block(begin: &str, block: &str, end: &str) -> Vec<String> {
    let mut v = vec![begin.to_string()];
    v.extend(block.lines().map(str::to_string));
    v.push(end.to_string());
    v
}

/// Drop the inclusive range `start..=end`.
fn remove_range(mut lines: Vec<String>, start: usize, end: usize) -> Vec<String> {
    let tail = lines.split_off(end + 1);
    lines.truncate(start);
    lines.extend(tail);
    lines
}

/// Replace the inclusive range `start..=end` with `replacement`.
fn replace_range(
    mut lines: Vec<String>,
    start: usize,
    end: usize,
    replacement: Vec<String>,
) -> Vec<String> {
    let tail = lines.split_off(end + 1);
    lines.truncate(start);
    lines.extend(replacement);
    lines.extend(tail);
    lines
}

/// Insert `block` starting at index `at`.
fn insert_at(mut lines: Vec<String>, at: usize, block: Vec<String>) -> Vec<String> {
    let tail = lines.split_off(at.min(lines.len()));
    lines.extend(block);
    lines.extend(tail);
    lines
}

/// Index at which a new block should be inserted (after the anchor line, or at EOF).
fn anchor_index(lines: &[String], insertafter: Option<&str>) -> usize {
    match insertafter {
        None | Some("EOF") => lines.len(),
        Some(anchor) => lines
            .iter()
            .position(|l| l.contains(anchor))
            .map_or(lines.len(), |i| i + 1),
    }
}

// ---- replace ------------------------------------------------------------

/// `replace` — substitute a regexp across a whole remote file.
struct Replace;

impl ModuleExecutor for Replace {
    fn name(&self) -> &'static str {
        "replace"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let path = args
            .get("path")
            .or_else(|| args.get("dest"))
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("replace requires a 'path'"))?;
        let pattern = args
            .get("regexp")
            .or_else(|| args.get("regex"))
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("replace requires a 'regexp'"))?;
        let replace_str = args.get("replace").and_then(Value::as_str).unwrap_or("");
        // v0.1: `after`/`before` are ignored — the substitution spans the whole
        // file. `.` does not match newlines; `^`/`$` match string start/end
        // unless the user includes `(?m)`.

        let original = read_remote_file(conn, path, false)?;
        let re = regex::Regex::new(pattern)
            .map_err(|e| ModuleError::args(format!("invalid regexp: {e}")))?;
        let new_content = re.replace_all(&original, replace_str).into_owned();
        if new_content == original {
            return Ok(ok_with_stdout("replace: no change"));
        }
        if !ctx.check_mode {
            conn.write_file(path, new_content.as_bytes())?;
        }
        let mut msg = if ctx.check_mode {
            String::from("replace: file would be updated")
        } else {
            String::from("replace: file updated")
        };
        if ctx.diff_mode {
            let diff = super::compute_file_diff(path, &original, &new_content);
            if !diff.is_empty() {
                msg.push('\n');
                msg.push_str(&diff);
            }
        }
        let mut r = ok_with_stdout(&msg);
        r.changed = true;
        Ok(r)
    }
}

/// Read a remote file via `cat`. When `allow_missing` and the read fails
/// (non-zero exit), return an empty string (treated as a file to create).
fn read_remote_file(
    conn: &Connection<'_>,
    path: &str,
    allow_missing: bool,
) -> Result<String, ModuleError> {
    let res = conn.run_command(&format!("cat '{path}'"))?;
    if res.success {
        return Ok(res.stdout.to_string());
    }
    if allow_missing {
        Ok(String::new())
    } else {
        Err(ModuleError::Other(format!(
            "cannot read {path}: {}",
            res.stderr
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::{Connection, CoreApiRef};
    use crate::test_support::{MockCore, localhost_host, null_core};
    use serde_json::json;

    fn conn(core: &CoreApiRef) -> Connection<'_> {
        Connection::new(core, ConnectionHandle::INVALID, localhost_host())
    }
    fn noop_ctx() -> TaskContext {
        TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        )
    }
    fn check_ctx() -> TaskContext {
        let mut c = noop_ctx();
        c.check_mode = true;
        c
    }
    fn check_diff_ctx() -> TaskContext {
        let mut c = check_ctx();
        c.diff_mode = true;
        c
    }

    #[test]
    fn resolve_markers_default_and_template() {
        let (b, e) = resolve_markers(None);
        assert_eq!(b, DEFAULT_MARKER_BEGIN);
        assert_eq!(e, DEFAULT_MARKER_END);
        let (b, e) = resolve_markers(Some("<!-- {mark} -->"));
        assert_eq!(b, "<!-- BEGIN ANSIBLE MANAGED BLOCK -->");
        assert_eq!(e, "<!-- END ANSIBLE MANAGED BLOCK -->");
    }

    #[test]
    fn apply_block_inserts_into_empty() {
        let new = apply_block(
            "",
            DEFAULT_MARKER_BEGIN,
            DEFAULT_MARKER_END,
            "line1\nline2",
            "present",
            None,
        )
        .unwrap_or_default();
        assert!(new.contains("line1"), "{new}");
        assert!(new.contains(DEFAULT_MARKER_BEGIN), "{new}");
    }

    #[test]
    fn apply_block_replaces_existing() {
        let original = format!("a\n{DEFAULT_MARKER_BEGIN}\nold\n{DEFAULT_MARKER_END}\nb");
        let new = apply_block(
            &original,
            DEFAULT_MARKER_BEGIN,
            DEFAULT_MARKER_END,
            "new",
            "present",
            None,
        );
        assert!(new.is_some());
        let new = new.unwrap_or_default();
        assert!(new.contains("new"), "{new}");
        assert!(!new.contains("old"), "{new}");
    }

    #[test]
    fn apply_block_absent_removes() {
        let original = format!("a\n{DEFAULT_MARKER_BEGIN}\nx\n{DEFAULT_MARKER_END}\nb");
        let new = apply_block(
            &original,
            DEFAULT_MARKER_BEGIN,
            DEFAULT_MARKER_END,
            "",
            "absent",
            None,
        );
        assert!(new.is_some());
        assert!(
            !new.as_ref()
                .unwrap_or(&String::new())
                .contains(DEFAULT_MARKER_BEGIN)
        );
    }

    #[test]
    fn apply_block_unchanged_returns_none() {
        let original = format!("{DEFAULT_MARKER_BEGIN}\nx\n{DEFAULT_MARKER_END}\n");
        let new = apply_block(
            &original,
            DEFAULT_MARKER_BEGIN,
            DEFAULT_MARKER_END,
            "x",
            "present",
            None,
        );
        assert!(new.is_none());
    }

    #[test]
    fn apply_block_insertafter_places_block() {
        let new = apply_block(
            "header\ntarget\nfooter",
            DEFAULT_MARKER_BEGIN,
            DEFAULT_MARKER_END,
            "block",
            "present",
            Some("target"),
        )
        .unwrap_or_default();
        let lines: Vec<&str> = new.lines().collect();
        // Block sits immediately after the "target" line.
        let target_pos = lines
            .iter()
            .position(|l| *l == "target")
            .unwrap_or(usize::MAX);
        assert!(
            lines
                .get(target_pos + 1)
                .is_some_and(|l| *l == DEFAULT_MARKER_BEGIN)
        );
    }

    #[test]
    fn blockinfile_reads_and_writes_back() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("alpha\nbeta\n"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = BlockInFile
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "block": "gamma"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
    }

    #[test]
    fn replace_substitutes_via_regex() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("foo bar foo"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Replace
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "regexp": "foo", "replace": "baz"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
    }

    #[test]
    fn replace_no_change_reports_ok() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("nothing here"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Replace
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "regexp": "foo", "replace": "baz"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!r.changed);
    }

    #[test]
    fn replace_missing_path_errors() {
        assert!(
            Replace
                .run(&conn(&null_core()), &json!({}), &noop_ctx())
                .is_err()
        );
    }

    #[test]
    fn blockinfile_check_mode_reports_would_change() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("alpha\nbeta\n"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = BlockInFile
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "block": "gamma"}),
                &check_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(
            r.stdout.as_str().contains("would be written"),
            "expected 'would be written' in: {}",
            r.stdout
        );
    }

    #[test]
    fn blockinfile_check_diff_mode_shows_diff_without_writing() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("alpha\nbeta\n"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = BlockInFile
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "block": "gamma"}),
                &check_diff_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(
            r.stdout.as_str().contains("would be written"),
            "expected 'would be written' in: {}",
            r.stdout
        );
        assert!(
            r.stdout.as_str().contains("--- /tmp/x:before"),
            "expected diff header in: {}",
            r.stdout
        );
        assert!(
            r.stdout.as_str().contains("+++ /tmp/x:after"),
            "expected diff header in: {}",
            r.stdout
        );
    }

    #[test]
    fn replace_check_mode_reports_would_change() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("foo bar foo"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Replace
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "regexp": "foo", "replace": "baz"}),
                &check_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(
            r.stdout.as_str().contains("would be updated"),
            "expected 'would be updated' in: {}",
            r.stdout
        );
    }

    #[test]
    fn replace_check_diff_mode_shows_diff_without_writing() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("foo bar foo"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Replace
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "regexp": "foo", "replace": "baz"}),
                &check_diff_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(
            r.stdout.as_str().contains("would be updated"),
            "expected 'would be updated' in: {}",
            r.stdout
        );
        assert!(
            r.stdout.as_str().contains("--- /tmp/x:before"),
            "expected diff header in: {}",
            r.stdout
        );
    }
}
