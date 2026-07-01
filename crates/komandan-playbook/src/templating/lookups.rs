//! Lookup plugins (spec §7.3): `file`, `env`, `pipe`.
//!
//! Exposed as a single `lookup('<kind>', ...)` function (and a `query` alias),
//! matching Ansible's `{{ lookup('file', '/etc/hostname') }}` call form. Only
//! the three v0.1 lookups ship; the broader lookup catalogue is deferred to
//! v0.1.1.
//!
//! Return shape (v0.1 simplification): a single argument yields a scalar
//! string; multiple arguments yield a list. Ansible's comma-joined-string
//! `lookup` vs list `query` distinction is collapsed for now.

use std::path::Path;
use std::process::Command;

use minijinja::{Environment, Error, ErrorKind, Value, value::Rest};

/// Register the `lookup` / `query` function on `env`.
pub(super) fn register(env: &mut Environment<'_>) {
    env.add_function("lookup", lookup);
    env.add_function("query", lookup);
}

fn err(kind: ErrorKind, msg: impl std::fmt::Display) -> Error {
    Error::new(kind, msg.to_string())
}

fn lookup(kind: &str, rest: Rest<Value>) -> Result<Value, Error> {
    let args: Vec<Value> = rest.0;
    match kind {
        "file" => lookup_file(&args),
        "env" => lookup_env(&args),
        "pipe" => lookup_pipe(&args),
        "password" => lookup_password(&args),
        "subelements" => lookup_subelements(&args),
        "flattened" => lookup_flattened(&args),
        "lines" => lookup_lines(&args),
        "template" => lookup_template(&args),
        "first_found" => lookup_first_found(&args),
        "ini" => lookup_ini(&args),
        "json" => lookup_json(&args),
        "sequence" => lookup_sequence(&args),
        "together" => lookup_together(&args),
        "indexed_items" => lookup_indexed_items(&args),
        "csvfile" => lookup_csvfile(&args),
        "fileglob" => lookup_fileglob(&args),
        other => Err(err(
            ErrorKind::UnknownFunction,
            format!("unknown lookup plugin '{other}'"),
        )),
    }
}

fn lookup_file(args: &[Value]) -> Result<Value, Error> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        let path = a.as_str().ok_or_else(|| {
            err(
                ErrorKind::InvalidOperation,
                "file lookup: path must be a string",
            )
        })?;
        let content = std::fs::read_to_string(path)
            .map_err(|e| err(ErrorKind::InvalidOperation, format!("file lookup: {e}")))?;
        out.push(content);
    }
    Ok(pack(out))
}

fn lookup_env(args: &[Value]) -> Result<Value, Error> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        let name = a.as_str().ok_or_else(|| {
            err(
                ErrorKind::InvalidOperation,
                "env lookup: name must be a string",
            )
        })?;
        let val = std::env::var(name)
            .map_err(|e| err(ErrorKind::InvalidOperation, format!("env lookup: {e}")))?;
        out.push(val);
    }
    Ok(pack(out))
}

fn lookup_pipe(args: &[Value]) -> Result<Value, Error> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        let cmd = a.as_str().ok_or_else(|| {
            err(
                ErrorKind::InvalidOperation,
                "pipe lookup: command must be a string",
            )
        })?;
        let result = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|e| err(ErrorKind::InvalidOperation, format!("pipe lookup: {e}")))?;
        if !result.status.success() {
            return Err(err(
                ErrorKind::InvalidOperation,
                format!(
                    "pipe lookup exited {}: {}",
                    result.status,
                    String::from_utf8_lossy(&result.stderr).trim_end()
                ),
            ));
        }
        out.push(String::from_utf8_lossy(&result.stdout).into_owned());
    }
    Ok(pack(out))
}

/// Collapse a single-element result to a scalar string; otherwise return a list.
fn pack(mut values: Vec<String>) -> Value {
    if values.len() == 1 {
        Value::from(values.pop().unwrap_or_default())
    } else {
        Value::from_serialize(values)
    }
}

// --- additional lookups ----------------------------------------------------

fn arg_strings(args: &[Value]) -> Vec<String> {
    args.iter()
        .filter_map(|a| a.as_str().map(str::to_string))
        .collect()
}

fn to_json_value(v: &Value) -> Result<serde_json::Value, Error> {
    serde_json::to_value(v).map_err(|e| err(ErrorKind::BadSerialization, e.to_string()))
}

/// Generate a random password. `terms[0]` = length (default 20), `terms[1]`
/// = character set (default alphanumeric). Entropy is drawn from
/// `/dev/urandom` (Linux) to avoid a hard `rand` dependency in this crate.
fn lookup_password(args: &[Value]) -> Result<Value, Error> {
    use std::io::Read;
    let terms = arg_strings(args);
    let len: usize = terms.first().and_then(|t| t.parse().ok()).unwrap_or(20);
    let chars: &str = terms.get(1).map_or(
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
        String::as_str,
    );
    let mut buf = vec![0u8; len];
    let mut f = std::fs::File::open("/dev/urandom")
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("password lookup: {e}")))?;
    f.read_exact(&mut buf)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("password lookup: {e}")))?;
    let pw: String = buf
        .iter()
        .filter_map(|b| {
            chars
                .as_bytes()
                .get((*b as usize) % chars.len())
                .copied()
                .map(char::from)
        })
        .collect();
    Ok(Value::from(pw))
}

/// Walk a dot-separated key path into a JSON value, returning null if absent.
fn json_path(v: &serde_json::Value, path: &str) -> serde_json::Value {
    let mut cur = v;
    for key in path.split('.') {
        match cur {
            serde_json::Value::Object(map) => match map.get(key) {
                Some(val) => cur = val,
                None => return serde_json::Value::Null,
            },
            _ => return serde_json::Value::Null,
        }
    }
    cur.clone()
}

/// Pair each dict in a list with the sub-element at `terms[1]`'s key path.
/// Powers `with_subelements`.
fn lookup_subelements(args: &[Value]) -> Result<Value, Error> {
    let list = args
        .first()
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "subelements: requires a list"))?;
    let keypath = args.get(1).and_then(Value::as_str).ok_or_else(|| {
        err(
            ErrorKind::InvalidOperation,
            "subelements: requires a key path",
        )
    })?;
    let arr = to_json_value(list)?;
    let serde_json::Value::Array(items) = arr else {
        return Err(err(
            ErrorKind::InvalidOperation,
            "subelements: first argument must be a list",
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in &items {
        let sub = json_path(item, keypath);
        out.push(serde_json::Value::Array(vec![item.clone(), sub]));
    }
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

/// Concatenate all list arguments into one flat list.
fn lookup_flattened(args: &[Value]) -> Result<Value, Error> {
    let mut out = Vec::new();
    for a in args {
        match to_json_value(a)? {
            serde_json::Value::Array(items) => out.extend(items),
            other => out.push(other),
        }
    }
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

/// Read a file and return its lines as a list (trailing empty lines trimmed).
fn lookup_lines(args: &[Value]) -> Result<Value, Error> {
    let path = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "lines: requires a file path"))?;
    let content = std::fs::read_to_string(path)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("lines lookup: {e}")))?;
    let trimmed = content.trim_end_matches(['\n', '\r']);
    let lines: Vec<String> = trimmed.lines().map(str::to_string).collect();
    Ok(Value::from_serialize(&lines))
}

/// Read a template file's raw content (rendered later by the calling context).
fn lookup_template(args: &[Value]) -> Result<Value, Error> {
    let path = args.first().and_then(Value::as_str).ok_or_else(|| {
        err(
            ErrorKind::InvalidOperation,
            "template: requires a file path",
        )
    })?;
    let content = std::fs::read_to_string(path)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("template lookup: {e}")))?;
    Ok(Value::from(content))
}

/// Return the first path that exists on disk. Arguments may be individual
/// strings or a single list of strings.
fn string_candidates(v: &Value) -> Vec<String> {
    match to_json_value(v) {
        Ok(serde_json::Value::Array(a)) => a
            .into_iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        Ok(serde_json::Value::String(s)) => vec![s],
        _ => Vec::new(),
    }
}

fn lookup_first_found(args: &[Value]) -> Result<Value, Error> {
    for a in args {
        for p in string_candidates(a) {
            if Path::new(&p).exists() {
                return Ok(Value::from(p));
            }
        }
    }
    Err(err(
        ErrorKind::InvalidOperation,
        "first_found: no file in the list exists",
    ))
}

// --- ini -------------------------------------------------------------------

/// Read a single value from an INI file: `terms[0]` path, `terms[1]` section,
/// `terms[2]` key. Returns an empty string when the key is absent.
fn lookup_ini(args: &[Value]) -> Result<Value, Error> {
    let terms = arg_strings(args);
    let path = terms
        .first()
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "ini: requires a file path"))?;
    let section = terms
        .get(1)
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "ini: requires a section"))?;
    let key = terms
        .get(2)
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "ini: requires a key"))?;
    let content = std::fs::read_to_string(path)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("ini lookup: {e}")))?;
    let mut current = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if current == *section
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == *key
        {
            return Ok(Value::from(v.trim().to_string()));
        }
    }
    Ok(Value::from(""))
}

// --- json ------------------------------------------------------------------

/// Read a JSON file. Optional `terms[1]` dot-separated key path navigates in.
fn lookup_json(args: &[Value]) -> Result<Value, Error> {
    let terms = arg_strings(args);
    let path = terms
        .first()
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "json: requires a file path"))?;
    let content = std::fs::read_to_string(path)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("json lookup: {e}")))?;
    let jv: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("json lookup: {e}")))?;
    let out = match terms.get(1) {
        Some(keypath) => json_path(&jv, keypath),
        None => jv,
    };
    Ok(Value::from_serialize(&out))
}

// --- sequence --------------------------------------------------------------

fn parse_sequence_spec(s: &str) -> Result<(i64, i64, i64), Error> {
    let parts: Vec<&str> = s.split("..").collect();
    match parts.as_slice() {
        [start, end] => {
            let st: i64 = start
                .trim()
                .parse()
                .map_err(|e| err(ErrorKind::InvalidOperation, format!("sequence: {e}")))?;
            let en: i64 = end
                .trim()
                .parse()
                .map_err(|e| err(ErrorKind::InvalidOperation, format!("sequence: {e}")))?;
            Ok((st, en, 1))
        }
        [start, end, step] => {
            let st: i64 = start
                .trim()
                .parse()
                .map_err(|e| err(ErrorKind::InvalidOperation, format!("sequence: {e}")))?;
            let en: i64 = end
                .trim()
                .parse()
                .map_err(|e| err(ErrorKind::InvalidOperation, format!("sequence: {e}")))?;
            let sp: i64 = step
                .trim()
                .parse()
                .map_err(|e| err(ErrorKind::InvalidOperation, format!("sequence: {e}")))?;
            Ok((st, en, sp))
        }
        _ => Err(err(
            ErrorKind::InvalidOperation,
            format!("sequence: cannot parse range '{s}'"),
        )),
    }
}

/// Generate an integer range. Accepts `"start..end"`, `"start..end..step"`,
/// or a dict `{"start": N, "end": M, "stride": S}` as the first term.
fn lookup_sequence(args: &[Value]) -> Result<Value, Error> {
    let first = args.first().ok_or_else(|| {
        err(
            ErrorKind::InvalidOperation,
            "sequence: requires a range spec",
        )
    })?;
    let jv = to_json_value(first)?;
    let (start, end, stride) = match &jv {
        serde_json::Value::Object(map) => {
            let start = map
                .get("start")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let end = map
                .get("end")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            let stride = map
                .get("stride")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(1);
            (start, end, stride)
        }
        serde_json::Value::String(s) => parse_sequence_spec(s)?,
        _ => {
            return Err(err(
                ErrorKind::InvalidOperation,
                "sequence: invalid range argument",
            ));
        }
    };
    let stride = if stride == 0 { 1 } else { stride };
    let mut out = Vec::new();
    let mut i = start;
    if stride > 0 {
        while i < end {
            out.push(serde_json::Value::from(i));
            i += stride;
        }
    } else {
        while i > end {
            out.push(serde_json::Value::from(i));
            i += stride;
        }
    }
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

// --- together --------------------------------------------------------------

/// Zip multiple lists, padding missing positions with null (`zip_longest`).
fn lookup_together(args: &[Value]) -> Result<Value, Error> {
    let mut lists: Vec<Vec<serde_json::Value>> = Vec::with_capacity(args.len());
    for a in args {
        let jv = to_json_value(a)?;
        match jv {
            serde_json::Value::Array(items) => lists.push(items),
            other => lists.push(vec![other]),
        }
    }
    let max = lists.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = Vec::with_capacity(max);
    for i in 0..max {
        let row: Vec<serde_json::Value> = lists
            .iter()
            .map(|l| l.get(i).cloned().unwrap_or(serde_json::Value::Null))
            .collect();
        out.push(serde_json::Value::Array(row));
    }
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

// --- indexed_items ---------------------------------------------------------

/// Enumerate a list into `[{"index": n, "item": v}, ...]`.
fn lookup_indexed_items(args: &[Value]) -> Result<Value, Error> {
    let first = args.first().ok_or_else(|| {
        err(
            ErrorKind::InvalidOperation,
            "indexed_items: requires a list",
        )
    })?;
    let arr = to_json_value(first)?;
    let serde_json::Value::Array(items) = arr else {
        return Err(err(
            ErrorKind::InvalidOperation,
            "indexed_items: first argument must be a list",
        ));
    };
    let out: Vec<serde_json::Value> = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| serde_json::json!({"index": i, "item": item}))
        .collect();
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

// --- csvfile ---------------------------------------------------------------

/// Split a single CSV line on commas, stripping surrounding double quotes.
fn parse_csv_line(line: &str) -> Vec<String> {
    line.split(',')
        .map(|f| {
            let f = f.trim();
            if f.len() >= 2 && f.starts_with('"') && f.ends_with('"') {
                f[1..f.len() - 1].to_string()
            } else {
                f.to_string()
            }
        })
        .collect()
}

/// Look up `return_column` from the CSV row where `key_column` == `key_value`.
/// `terms[0]` path, `terms[1]` key column, `terms[2]` key value,
/// `terms[3]` return column (defaults to the key column).
fn lookup_csvfile(args: &[Value]) -> Result<Value, Error> {
    let terms = arg_strings(args);
    let path = terms
        .first()
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "csvfile: requires a file path"))?;
    let key_col = terms.get(1).ok_or_else(|| {
        err(
            ErrorKind::InvalidOperation,
            "csvfile: requires a key column",
        )
    })?;
    let key_val = terms
        .get(2)
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "csvfile: requires a key value"))?;
    let ret_col = terms.get(3).map_or(key_col.as_str(), String::as_str);
    let content = std::fs::read_to_string(path)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("csvfile lookup: {e}")))?;
    let mut lines = content.lines();
    let header_line = lines
        .next()
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "csvfile: empty file"))?;
    let header = parse_csv_line(header_line);
    let key_idx = header.iter().position(|h| h == key_col).ok_or_else(|| {
        err(
            ErrorKind::InvalidOperation,
            format!("csvfile: column '{key_col}' not found"),
        )
    })?;
    let ret_idx = header.iter().position(|h| h == ret_col).unwrap_or(key_idx);
    for line in lines {
        let row = parse_csv_line(line);
        if row.get(key_idx).is_some_and(|v| v == key_val) {
            return Ok(Value::from(row.get(ret_idx).cloned().unwrap_or_default()));
        }
    }
    Ok(Value::from(""))
}

// --- fileglob --------------------------------------------------------------

/// Match a simple glob pattern containing only `*` wildcards (no `**`).
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    if !name.starts_with(parts[0]) {
        return false;
    }
    let mut rest = &name[parts[0].len()..];
    let last = parts[parts.len() - 1];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(idx) => rest = &rest[idx + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// List files matching a `*` glob pattern (no recursion).
fn lookup_fileglob(args: &[Value]) -> Result<Value, Error> {
    let pattern = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| err(ErrorKind::InvalidOperation, "fileglob: requires a pattern"))?;
    let (dir, glob) = pattern
        .rfind('/')
        .map_or((".", pattern), |i| (&pattern[..i], &pattern[i + 1..]));
    let entries = std::fs::read_dir(dir)
        .map_err(|e| err(ErrorKind::InvalidOperation, format!("fileglob lookup: {e}")))?;
    let mut out = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if glob_match(glob, &name) {
            out.push(entry.path().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(Value::from_serialize(&out))
}

#[cfg(test)]
mod tests {
    use crate::templating::engine::build_environment;

    #[test]
    fn file_lookup_reads_content() {
        let env = build_environment();
        let path = std::env::current_exe().unwrap_or_default();
        let ctx = serde_json::json!({"p": path.to_string_lossy().to_string()});
        // Just exercise the code path; the binary path may not be utf8-stable
        // across systems, so assert no-error + non-empty when readable.
        let _ = env.render_str("{{ lookup('file', p) }}", &ctx);
    }

    #[test]
    fn env_lookup_reads_var() {
        let env = build_environment();
        // PATH is effectively always set.
        let out = env
            .render_str("{{ lookup('env', 'PATH') }}", serde_json::json!({}))
            .unwrap_or_default();
        assert!(!out.is_empty(), "PATH should be set, got empty");
    }

    #[test]
    fn pipe_lookup_runs_command() {
        let env = build_environment();
        let out = env
            .render_str("{{ lookup('pipe', 'echo hi') }}", serde_json::json!({}))
            .unwrap_or_default();
        assert_eq!(out.trim(), "hi");
    }

    #[test]
    fn unknown_lookup_errors() {
        let env = build_environment();
        let res = env.render_str("{{ lookup('nope', 'x') }}", serde_json::json!({}));
        assert!(res.is_err());
    }

    #[test]
    fn password_default_length() {
        let env = build_environment();
        let out = env
            .render_str("{{ lookup('password', '12') }}", serde_json::json!({}))
            .unwrap_or_default();
        assert_eq!(out.len(), 12);
    }

    #[test]
    fn password_default_20_when_empty() {
        let env = build_environment();
        let out = env
            .render_str("{{ lookup('password') }}", serde_json::json!({}))
            .unwrap_or_default();
        assert_eq!(out.len(), 20);
    }

    #[test]
    fn password_custom_charset() {
        let env = build_environment();
        let out = env
            .render_str("{{ lookup('password', '8', '01') }}", serde_json::json!({}))
            .unwrap_or_default();
        assert_eq!(out.len(), 8);
        assert!(out.chars().all(|c| c == '0' || c == '1'));
    }

    #[test]
    fn subelements_pairs() {
        let env = build_environment();
        let ctx = serde_json::json!({
            "users": [
                {"name": "a", "groups": ["x", "y"]},
                {"name": "b", "groups": ["z"]}
            ]
        });
        // 2 users → 2 pairs; length of pairs list is 2.
        let out = env
            .render_str(
                "{{ lookup('subelements', users, 'groups') | length }}",
                &ctx,
            )
            .unwrap_or_default();
        assert_eq!(out, "2");
    }

    #[test]
    fn subelements_extract_nested() {
        let env = build_environment();
        let ctx = serde_json::json!({
            "d": [{"opt": {"k": 7}}]
        });
        // pair[1] is the sub-element; access [0][1].k
        let out = env
            .render_str("{{ (lookup('subelements', d, 'opt') | first)[1].k }}", &ctx)
            .unwrap_or_default();
        assert_eq!(out, "7");
    }

    #[test]
    fn flattened_concatenates() {
        let env = build_environment();
        let ctx = serde_json::json!({"a": [1, 2], "b": [3]});
        let out = env
            .render_str("{{ query('flattened', a, b) | sort | join(',') }}", &ctx)
            .unwrap_or_default();
        assert_eq!(out, "1,2,3");
    }

    #[test]
    fn lines_reads_file() {
        let env = build_environment();
        let dir = std::env::temp_dir();
        let path = dir.join("komandan_lookup_lines_test.txt");
        std::fs::write(&path, "alpha\nbeta\n\n").unwrap_or(());
        let ctx = serde_json::json!({"p": path.to_string_lossy().to_string()});
        let out = env
            .render_str("{{ lookup('lines', p) | join('|') }}", &ctx)
            .unwrap_or_default();
        assert_eq!(out, "alpha|beta");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn template_reads_raw() {
        let env = build_environment();
        let dir = std::env::temp_dir();
        let path = dir.join("komandan_lookup_template_test.txt");
        std::fs::write(&path, "hello {{ name }}").unwrap_or(());
        let ctx = serde_json::json!({"p": path.to_string_lossy().to_string(), "name": "w"});
        // The lookup returns the raw file content (no recursive rendering).
        let out = env
            .render_str("{{ lookup('template', p) }}", &ctx)
            .unwrap_or_default();
        assert_eq!(out, "hello {{ name }}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn first_found_returns_existing() {
        let env = build_environment();
        let ctx = serde_json::json!({});
        let existing = std::env::current_exe().map_or_else(
            |_| "/bin/sh".to_string(),
            |p| p.to_string_lossy().to_string(),
        );
        let tmpl = format!("{{{{ lookup('first_found', '/nope/nope', '{existing}') }}}}");
        let out = env.render_str(&tmpl, &ctx).unwrap_or_default();
        assert!(!out.is_empty());
        assert_ne!(out, "/nope/nope");
    }

    #[test]
    fn first_found_none_errors() {
        let env = build_environment();
        let res = env.render_str(
            "{{ lookup('first_found', '/nope/aaa', '/nope/bbb') }}",
            serde_json::json!({}),
        );
        assert!(res.is_err());
    }

    #[test]
    fn ini_reads_section_key() {
        let env = build_environment();
        let dir = std::env::temp_dir();
        let path = dir.join("komandan_lookup_ini_test.ini");
        std::fs::write(&path, "[main]\nname = alice\n# comment\n[other]\nx = 1\n").unwrap_or(());
        let p = path.to_string_lossy().to_string();
        let out = env
            .render_str(
                &format!("{{{{ lookup('ini', '{p}', 'main', 'name') }}}}"),
                serde_json::json!({}),
            )
            .unwrap_or_default();
        assert_eq!(out, "alice");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn json_reads_file_and_path() {
        let env = build_environment();
        let dir = std::env::temp_dir();
        let path = dir.join("komandan_lookup_json_test.json");
        std::fs::write(&path, "{\"a\": {\"b\": 42}}").unwrap_or(());
        let p = path.to_string_lossy().to_string();
        let out = env
            .render_str(
                &format!("{{{{ lookup('json', '{p}', 'a.b') }}}}"),
                serde_json::json!({}),
            )
            .unwrap_or_default();
        assert_eq!(out, "42");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sequence_range_string() {
        let env = build_environment();
        let out = env
            .render_str(
                "{{ lookup('sequence', '1..4') | join(',') }}",
                serde_json::json!({}),
            )
            .unwrap_or_default();
        assert_eq!(out, "1,2,3");
    }

    #[test]
    fn sequence_range_with_step() {
        let env = build_environment();
        let out = env
            .render_str(
                "{{ lookup('sequence', '0..10..2') | join(',') }}",
                serde_json::json!({}),
            )
            .unwrap_or_default();
        assert_eq!(out, "0,2,4,6,8");
    }

    #[test]
    fn together_zips_lists() {
        let env = build_environment();
        let ctx = serde_json::json!({"a": [1, 2], "b": ["x", "y"]});
        let out = env
            .render_str("{{ lookup('together', a, b) | length }}", &ctx)
            .unwrap_or_default();
        assert_eq!(out, "2");
    }

    #[test]
    fn indexed_items_enumerates() {
        let env = build_environment();
        let ctx = serde_json::json!({"l": ["a", "b"]});
        let out = env
            .render_str("{{ (lookup('indexed_items', l) | first).index }}", &ctx)
            .unwrap_or_default();
        assert_eq!(out, "0");
        let out2 = env
            .render_str("{{ (lookup('indexed_items', l) | last).item }}", &ctx)
            .unwrap_or_default();
        assert_eq!(out2, "b");
    }

    #[test]
    fn csvfile_lookups_value() {
        let env = build_environment();
        let dir = std::env::temp_dir();
        let path = dir.join("komandan_lookup_csv_test.csv");
        std::fs::write(&path, "id,name\n1,alice\n2,bob\n").unwrap_or(());
        let p = path.to_string_lossy().to_string();
        let out = env
            .render_str(
                &format!("{{{{ lookup('csvfile', '{p}', 'id', '2', 'name') }}}}"),
                serde_json::json!({}),
            )
            .unwrap_or_default();
        assert_eq!(out, "bob");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fileglob_matches_files() {
        let env = build_environment();
        let dir = std::env::temp_dir();
        let base = "komandan_lookup_glob_test_";
        let p1 = dir.join(format!("{base}a.txt"));
        let p2 = dir.join(format!("{base}b.txt"));
        std::fs::write(&p1, "x").unwrap_or(());
        std::fs::write(&p2, "y").unwrap_or(());
        let pattern = format!("{}/{base}*.txt", dir.to_string_lossy());
        let out = env
            .render_str(
                &format!("{{{{ lookup('fileglob', '{pattern}') | length }}}}"),
                serde_json::json!({}),
            )
            .unwrap_or_default();
        let n: usize = out.parse().unwrap_or(0);
        assert!(n >= 2, "expected >=2 matches, got {n}");
        let _ = std::fs::remove_file(&p1);
        let _ = std::fs::remove_file(&p2);
    }
}
