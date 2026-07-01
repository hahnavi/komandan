//! Ansible gap filters (spec §7.3) not already provided by minijinja.
//!
//! minijinja ships `upper`, `lower`, `length`/`count`, `sort`, `map`, `select`,
//! `reject`, `selectattr`, `rejectattr`, `groupby`, `unique`, `zip`, `join`,
//! `split`, `first`, `last`, `min`, `max`, `reverse`, `replace`, `trim`,
//! `default`/`d`, `int`, `float`, `bool`, `string`, `list`, `sum`, `abs`,
//! `round`, `title`, `capitalize`, `tojson`, etc. as built-ins. This module
//! registers the **remaining** filters Ansible playbooks expect.
//!
//! Value-transforming filters round-trip through [`serde_json::Value`] (via
//! [`mj_to_json`]) rather than learning minijinja's value iteration API — a
//! deliberate simplicity/correctness trade-off for v0.1.

use std::path::Path;

use chrono::{DateTime, NaiveDateTime, Utc};
use md5::Md5;
use minijinja::{Environment, Error, ErrorKind, Value};
use sha2::{Digest, Sha256, Sha512};

use crate::vars::{json_to_yaml, yaml_to_json};

/// Register every gap filter on `env`.
pub(super) fn register(env: &mut Environment<'_>) {
    env.add_filter("mandatory", mandatory);
    env.add_filter("to_yaml", to_yaml);
    env.add_filter("from_yaml", from_yaml);
    env.add_filter("to_json", to_json);
    env.add_filter("from_json", from_json);
    env.add_filter("b64encode", b64encode);
    env.add_filter("b64decode", b64decode);
    env.add_filter("regex_replace", regex_replace);
    env.add_filter("regex_search", regex_search);
    env.add_filter("regex_findall", regex_findall);
    env.add_filter("regex_escape", regex_escape);
    env.add_filter("dict2items", dict2items);
    env.add_filter("items2dict", items2dict);
    env.add_filter("difference", difference);
    env.add_filter("union", seq_union);
    env.add_filter("intersect", intersect);
    env.add_filter("product", product);
    env.add_filter("combine", combine);
    env.add_filter("ternary", ternary);
    env.add_filter("quote", quote);
    env.add_filter("pquote", pquote);
    env.add_filter("password_hash", password_hash);
    env.add_filter("basename", basename);
    env.add_filter("dirname", dirname);
    env.add_filter("realpath", realpath);
    env.add_filter("relpath", relpath);
    env.add_filter("splitext", splitext);
    env.add_filter("flatten", flatten);
    env.add_filter("hash", hash_filter);
    env.add_filter("to_nice_json", to_nice_json);
    env.add_filter("to_nice_yaml", to_nice_yaml);
    env.add_filter("from_yaml_all", from_yaml_all);
    env.add_filter("path_join", path_join);
    env.add_filter("strftime", strftime);
    env.add_filter("to_datetime", to_datetime);
    env.add_filter("urlencode", urlencode);
    env.add_filter("urlsplit", urlsplit);
    env.add_filter("type_debug", type_debug);
    env.add_filter("human_readable", human_readable);
    env.add_filter("human_to_bytes", human_to_bytes);
    env.add_filter("center", center);
    env.add_filter("win_basename", win_basename);
    env.add_filter("win_dirname", win_dirname);
    env.add_filter("win_splitext", win_splitext);
    env.add_filter("regex_findall_ind", regex_findall_ind);
    env.add_filter("random", random);
    env.add_filter("ipv4", ipv4);
    env.add_filter("ipv6", ipv6);
    env.add_filter("ipwrap", ipwrap);
}

// --- helpers ---------------------------------------------------------------

fn mj_err(kind: ErrorKind, msg: impl std::fmt::Display) -> Error {
    Error::new(kind, msg.to_string())
}

pub(super) fn mj_to_json(v: &Value) -> Result<serde_json::Value, Error> {
    serde_json::to_value(v).map_err(|e| mj_err(ErrorKind::BadSerialization, e.to_string()))
}

fn seq_to_vec(v: &Value) -> Result<Vec<serde_json::Value>, Error> {
    match mj_to_json(v)? {
        serde_json::Value::Array(a) => Ok(a),
        _ => Err(mj_err(
            ErrorKind::InvalidOperation,
            "expected a sequence (list)",
        )),
    }
}

// --- presence / type coercion ---------------------------------------------

fn mandatory(v: &Value) -> Result<Value, Error> {
    if v.is_undefined() {
        return Err(mj_err(
            ErrorKind::UndefinedError,
            "mandatory variable was not provided",
        ));
    }
    Ok(v.clone())
}

fn ternary(v: &Value, true_val: Value, false_val: Value) -> Value {
    if v.is_true() { true_val } else { false_val }
}

// --- serialization ---------------------------------------------------------

fn to_yaml(v: &Value) -> Result<String, Error> {
    let jv = mj_to_json(v)?;
    let yaml_node = json_to_yaml(&jv);
    serde_yaml::to_string(&yaml_node)
        .map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))
}

fn from_yaml(s: &str) -> Result<Value, Error> {
    let yv: serde_yaml::Value =
        serde_yaml::from_str(s).map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))?;
    Ok(Value::from_serialize(yaml_to_json(&yv)))
}

fn to_json(v: &Value) -> Result<String, Error> {
    serde_json::to_string(v).map_err(|e| mj_err(ErrorKind::BadSerialization, e.to_string()))
}

fn from_json(s: &str) -> Result<Value, Error> {
    let jv: serde_json::Value =
        serde_json::from_str(s).map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))?;
    Ok(Value::from_serialize(&jv))
}

// --- base64 ----------------------------------------------------------------

fn b64encode(s: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

fn b64decode(s: &str) -> Result<String, Error> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))
}

// --- regex -----------------------------------------------------------------

fn build_regex(pattern: &str) -> Result<regex::Regex, Error> {
    regex::Regex::new(pattern).map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))
}

fn regex_replace(s: &str, pattern: &str, replace: &str) -> Result<String, Error> {
    Ok(build_regex(pattern)?.replace_all(s, replace).into_owned())
}

/// Returns the first match. If the pattern has one capture group, returns that
/// group; if multiple groups, returns them as a list; otherwise the whole
/// match. An empty string is returned when nothing matches (Ansible parity).
fn regex_search(s: &str, pattern: &str) -> Result<Value, Error> {
    let re = build_regex(pattern)?;
    let Some(caps) = re.captures(s) else {
        return Ok(Value::from(""));
    };
    let ngroups = caps.len() - 1; // index 0 is the whole match
    match ngroups {
        0 => Ok(Value::from(caps[0].to_string())),
        1 => Ok(Value::from(caps[1].to_string())),
        _ => {
            let groups: Vec<String> = (1..=ngroups)
                .map(|i| {
                    caps.get(i)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default()
                })
                .collect();
            Ok(Value::from_serialize(&groups))
        }
    }
}

fn regex_findall(s: &str, pattern: &str) -> Result<Value, Error> {
    let re = build_regex(pattern)?;
    let ngroups = re.captures_len() - 1;
    let out: Vec<String> = if ngroups >= 1 {
        re.captures_iter(s)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect()
    } else {
        re.find_iter(s).map(|m| m.as_str().to_string()).collect()
    };
    Ok(Value::from_serialize(&out))
}

fn regex_escape(s: &str) -> String {
    regex::escape(s)
}

// --- dict <-> items --------------------------------------------------------

fn dict2items(
    v: &Value,
    key_name: Option<String>,
    value_name: Option<String>,
) -> Result<Value, Error> {
    let serde_json::Value::Object(map) = mj_to_json(v)? else {
        return Err(mj_err(
            ErrorKind::InvalidOperation,
            "dict2items requires a mapping",
        ));
    };
    let kn = key_name.unwrap_or_else(|| "key".to_string());
    let vn = value_name.unwrap_or_else(|| "value".to_string());
    let out: Vec<serde_json::Value> = map
        .into_iter()
        .map(|(k, val)| {
            let mut o = serde_json::Map::new();
            o.insert(kn.clone(), serde_json::Value::String(k));
            o.insert(vn.clone(), val);
            serde_json::Value::Object(o)
        })
        .collect();
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

fn items2dict(items: &Value, key: Option<String>, value: Option<String>) -> Result<Value, Error> {
    let serde_json::Value::Array(arr) = mj_to_json(items)? else {
        return Err(mj_err(
            ErrorKind::InvalidOperation,
            "items2dict requires a list",
        ));
    };
    let kn = key.unwrap_or_else(|| "key".to_string());
    let vn = value.unwrap_or_else(|| "value".to_string());
    let mut out = serde_json::Map::new();
    for item in arr {
        if let serde_json::Value::Object(o) = item {
            let k = o
                .get(&kn)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    mj_err(ErrorKind::InvalidOperation, "items2dict: item missing key")
                })?;
            let v = o.get(&vn).cloned().unwrap_or(serde_json::Value::Null);
            out.insert(k.to_string(), v);
        }
    }
    Ok(Value::from_serialize(serde_json::Value::Object(out)))
}

// --- set ops ---------------------------------------------------------------

fn difference(a: &Value, b: &Value) -> Result<Value, Error> {
    let av = seq_to_vec(a)?;
    let bv = seq_to_vec(b)?;
    let out: Vec<serde_json::Value> = av.into_iter().filter(|x| !bv.contains(x)).collect();
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

fn intersect(a: &Value, b: &Value) -> Result<Value, Error> {
    let av = seq_to_vec(a)?;
    let bv = seq_to_vec(b)?;
    let mut seen = Vec::new();
    for x in av {
        if bv.contains(&x) && !seen.contains(&x) {
            seen.push(x);
        }
    }
    Ok(Value::from_serialize(serde_json::Value::Array(seen)))
}

fn seq_union(a: &Value, b: &Value) -> Result<Value, Error> {
    let mut av = seq_to_vec(a)?;
    let bv = seq_to_vec(b)?;
    for x in bv {
        if !av.contains(&x) {
            av.push(x);
        }
    }
    Ok(Value::from_serialize(serde_json::Value::Array(av)))
}

fn product(a: &Value, b: &Value) -> Result<Value, Error> {
    let av = seq_to_vec(a)?;
    let bv = seq_to_vec(b)?;
    let mut out = Vec::with_capacity(av.len() * bv.len());
    for x in &av {
        for y in &bv {
            out.push(serde_json::Value::Array(vec![x.clone(), y.clone()]));
        }
    }
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

// --- combine ---------------------------------------------------------------

fn combine(a: &Value, b: &Value, recursive: Option<bool>) -> Result<Value, Error> {
    let mut av = mj_to_json(a)?;
    let bv = mj_to_json(b)?;
    if recursive == Some(true) {
        av = deep_merge(av, bv);
    } else {
        match (&mut av, &bv) {
            (serde_json::Value::Object(am), serde_json::Value::Object(bo)) => {
                for (k, v) in bo {
                    am.insert(k.clone(), v.clone());
                }
            }
            _ => av = bv,
        }
    }
    Ok(Value::from_serialize(&av))
}

fn deep_merge(a: serde_json::Value, b: serde_json::Value) -> serde_json::Value {
    match (a, b) {
        (serde_json::Value::Object(mut am), serde_json::Value::Object(bo)) => {
            for (k, bv) in bo {
                if let Some(av) = am.remove(&k) {
                    am.insert(k, deep_merge(av, bv));
                } else {
                    am.insert(k, bv);
                }
            }
            serde_json::Value::Object(am)
        }
        (_, b) => b,
    }
}

// --- shell quoting ---------------------------------------------------------

fn quote_one(s: &str) -> String {
    // POSIX single-quote wrapping: embed every `'` as `'\''`.
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn quote(s: &str) -> String {
    quote_one(s)
}

fn pquote(items: &Value) -> Result<String, Error> {
    let v = seq_to_vec(items)?;
    let parts: Result<Vec<String>, Error> = v
        .iter()
        .map(|x| {
            x.as_str()
                .map(quote_one)
                .ok_or_else(|| mj_err(ErrorKind::InvalidOperation, "pquote: non-string item"))
        })
        .collect();
    Ok(parts?.join(" "))
}

// --- password hash ---------------------------------------------------------

/// `password_hash` — **bcrypt only** for v0.1. The Ansible scheme/salt
/// positional args are accepted but currently ignored (a fresh random salt is
/// generated at cost 12); idempotent hashing from a fixed salt is a later pass.
fn password_hash(s: &str, _scheme: Option<String>, _salt: Option<String>) -> Result<String, Error> {
    bcrypt::hash(s, 12).map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))
}

// --- path ------------------------------------------------------------------

fn basename(s: &str) -> String {
    Path::new(s)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string()
}

fn dirname(s: &str) -> String {
    Path::new(s)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("")
        .to_string()
}

fn realpath(s: &str) -> Result<String, Error> {
    std::fs::canonicalize(s)
        .map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))?
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| mj_err(ErrorKind::InvalidOperation, "realpath: non-utf8 path"))
}

fn relpath(s: &str, start: &str) -> String {
    // Component-wise relative path computation (no extra dep).
    let target: Vec<_> = Path::new(s).components().collect();
    let base: Vec<_> = Path::new(start).components().collect();
    let mut i = 0;
    while i < target.len() && i < base.len() && target[i] == base[i] {
        i += 1;
    }
    let mut out: Vec<String> = (0..base.len() - i).map(|_| "..".to_string()).collect();
    for c in &target[i..] {
        out.push(c.as_os_str().to_string_lossy().into_owned());
    }
    if out.is_empty() {
        out.push(".".to_string());
    }
    out.join(std::path::MAIN_SEPARATOR_STR)
}

fn splitext(s: &str) -> Value {
    let path = Path::new(s);
    let stem = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let ext = path
        .extension()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    Value::from_serialize(&[stem, ext])
}

// --- flatten ---------------------------------------------------------------

/// Flatten nested lists. `depth: None` flattens fully (legacy behavior);
/// `depth: Some(n)` flattens only `n` levels deep.
fn flatten(v: &Value, depth: Option<i64>) -> Result<Value, Error> {
    let jv = mj_to_json(v)?;
    let mut out = Vec::new();
    flatten_into_depth(&jv, depth, &mut out);
    Ok(Value::from_serialize(serde_json::Value::Array(out)))
}

fn flatten_into_depth(v: &serde_json::Value, depth: Option<i64>, out: &mut Vec<serde_json::Value>) {
    if let serde_json::Value::Array(a) = v {
        // Only recurse when no depth limit remains.
        if depth.is_none_or(|d| d > 0) {
            let next = depth.map(|d| d - 1);
            for x in a {
                flatten_into_depth(x, next, out);
            }
            return;
        }
    }
    out.push(v.clone());
}

// --- hash ------------------------------------------------------------------

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Cryptographic hash. `md5`/`sha256`/`sha512` supported; unknown algorithms
/// fall back to `sha256`. `sha1` is unsupported (no dep) and errors.
fn hash_filter(v: &str, algo: Option<&str>) -> Result<String, Error> {
    match algo.unwrap_or("sha256") {
        "md5" => Ok(hex_lower(Md5::digest(v.as_bytes()).as_slice())),
        "sha1" => Err(mj_err(ErrorKind::InvalidOperation, "sha1 not supported")),
        "sha512" => Ok(hex_lower(Sha512::digest(v.as_bytes()).as_slice())),
        // sha256 and any unknown algorithm default to sha256.
        _ => Ok(hex_lower(Sha256::digest(v.as_bytes()).as_slice())),
    }
}

// --- nice serialization ----------------------------------------------------

fn to_nice_json(v: &Value, _indent: Option<i64>) -> Result<String, Error> {
    // serde_json only emits 2-space pretty; the `indent` arg is accepted for
    // Ansible parity but has no effect beyond presence.
    serde_json::to_string_pretty(v).map_err(|e| mj_err(ErrorKind::BadSerialization, e.to_string()))
}

fn to_nice_yaml(v: &Value) -> Result<String, Error> {
    serde_yaml::to_string(v).map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))
}

/// Parse a multi-document YAML stream into a list of values.
fn from_yaml_all(v: &str) -> Result<Value, Error> {
    use serde::Deserialize;
    let mut docs = Vec::new();
    for d in serde_yaml::Deserializer::from_str(v) {
        let yv: serde_yaml::Value = serde_yaml::Value::deserialize(d)
            .map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))?;
        docs.push(yaml_to_json(&yv));
    }
    Ok(Value::from_serialize(serde_json::Value::Array(docs)))
}

// --- path join -------------------------------------------------------------

fn path_join(base: &str, sub: &str) -> String {
    let mut out = String::with_capacity(base.len() + sub.len() + 1);
    out.push_str(base.trim_end_matches('/'));
    out.push('/');
    out.push_str(sub.trim_start_matches('/'));
    out
}

// --- datetime --------------------------------------------------------------

fn parse_datetime(s: &str) -> Result<DateTime<Utc>, Error> {
    if let Ok(secs) = s.parse::<i64>() {
        return DateTime::<Utc>::from_timestamp(secs, 0)
            .ok_or_else(|| mj_err(ErrorKind::InvalidOperation, "invalid epoch seconds"));
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    for fmt in &["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(naive.and_utc());
        }
    }
    Err(mj_err(
        ErrorKind::InvalidOperation,
        format!("cannot parse datetime string '{s}'"),
    ))
}

/// Format a timestamp value using a `strftime` format string. Accepts epoch
/// integers (or numeric strings) and ISO 8601 / common naive datetime strings.
fn strftime(v: &Value, fmt: &str) -> Result<String, Error> {
    let dt = if let Some(secs) = v.as_i64() {
        DateTime::<Utc>::from_timestamp(secs, 0)
            .ok_or_else(|| mj_err(ErrorKind::InvalidOperation, "invalid epoch seconds"))?
    } else if let Some(s) = v.as_str() {
        parse_datetime(s)?
    } else {
        return Err(mj_err(
            ErrorKind::InvalidOperation,
            "strftime: expected a number or datetime string",
        ));
    };
    Ok(dt.format(fmt).to_string())
}

/// Parse a datetime string with an explicit format (default
/// `%Y-%m-%d %H:%M:%S`) and re-emit it as ISO 8601 (RFC 3339).
fn to_datetime(v: &str, fmt: Option<&str>) -> Result<String, Error> {
    let f = fmt.unwrap_or("%Y-%m-%d %H:%M:%S");
    let naive = NaiveDateTime::parse_from_str(v, f)
        .map_err(|e| mj_err(ErrorKind::InvalidOperation, e.to_string()))?;
    Ok(naive.and_utc().to_rfc3339())
}

// --- url -------------------------------------------------------------------

/// Percent-encode every non-unreserved byte (RFC 3986).
fn urlencode(v: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(v.len() * 3);
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Split a URL into `scheme`/`netloc`/`path`/`query`/`fragment`.
fn urlsplit(v: &str) -> Value {
    let (scheme, after_scheme) = match v.split_once("://") {
        Some((s, r)) => (s.to_string(), r),
        None => (String::new(), v),
    };
    let (before_fragment, fragment) = match after_scheme.split_once('#') {
        Some((b, f)) => (b, f.to_string()),
        None => (after_scheme, String::new()),
    };
    let (before_query, query) = match before_fragment.split_once('?') {
        Some((b, q)) => (b, q.to_string()),
        None => (before_fragment, String::new()),
    };
    let (netloc, path) = if scheme.is_empty() {
        (String::new(), before_query.to_string())
    } else if let Some((nl, p)) = before_query.split_once('/') {
        (nl.to_string(), format!("/{p}"))
    } else {
        (before_query.to_string(), String::new())
    };
    let map = serde_json::json!({
        "scheme": scheme,
        "netloc": netloc,
        "path": path,
        "query": query,
        "fragment": fragment,
    });
    Value::from_serialize(&map)
}

// --- type debug ------------------------------------------------------------

/// Return the [`ValueKind`] name as a human-readable string.
fn type_debug(v: &Value) -> String {
    format!("{}", v.kind())
}

// --- human readable byte sizes --------------------------------------------

/// Convert a byte count into a human-readable string (e.g. `1536` → `"1.5 KB"`).
fn human_readable(v: &Value) -> Result<String, Error> {
    let jv = mj_to_json(v)?;
    let mut bytes = jv.as_f64().ok_or_else(|| {
        mj_err(
            ErrorKind::InvalidOperation,
            "human_readable: expected a number",
        )
    })?;
    if bytes <= 0.0 {
        return Ok("0 B".to_string());
    }
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut idx = 0;
    while bytes >= 1024.0 && idx < units.len() - 1 {
        bytes /= 1024.0;
        idx += 1;
    }
    Ok(format!("{bytes:.1} {}", units[idx]))
}

/// Parse a human-readable size string (`"1.5 GB"`, `"500MB"`, `"1.5T"`) into bytes.
fn human_to_bytes(v: &str) -> Result<i64, Error> {
    let s = v.trim().to_lowercase();
    let mut num_end = 0;
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_digit() || c == '.' {
            num_end = i + 1;
        } else {
            break;
        }
    }
    if num_end == 0 {
        return Err(mj_err(
            ErrorKind::InvalidOperation,
            format!("human_to_bytes: cannot parse '{v}'"),
        ));
    }
    let unit = s[num_end..].trim();
    let mult: i64 = match unit {
        "" | "b" => 1,
        "k" | "kb" => 1024,
        "m" | "mb" => 1024_i64.pow(2),
        "g" | "gb" => 1024_i64.pow(3),
        "t" | "tb" => 1024_i64.pow(4),
        "p" | "pb" => 1024_i64.pow(5),
        other => {
            return Err(mj_err(
                ErrorKind::InvalidOperation,
                format!("human_to_bytes: unknown unit '{other}'"),
            ));
        }
    };
    let num_str = &s[..num_end];
    let (whole, frac) = match num_str.split_once('.') {
        Some((w, f)) => (w, f),
        None => (num_str, ""),
    };
    let mut bytes = whole
        .parse::<i64>()
        .map_err(|e| mj_err(ErrorKind::InvalidOperation, format!("human_to_bytes: {e}")))?
        .saturating_mul(mult);
    if !frac.is_empty() {
        let frac_num: i64 = frac
            .parse()
            .map_err(|e| mj_err(ErrorKind::InvalidOperation, format!("human_to_bytes: {e}")))?;
        let denom = 10_i64.pow(u32::try_from(frac.len()).unwrap_or(u32::MAX));
        bytes += frac_num.saturating_mul(mult) / denom;
    }
    Ok(bytes)
}

// --- string padding --------------------------------------------------------

/// Pad `v` with spaces on both sides to center it within `width` characters.
fn center(v: &str, width: i64) -> String {
    let len = v.chars().count();
    let width = usize::try_from(width).unwrap_or(0);
    if len >= width {
        return v.to_string();
    }
    let total = width - len;
    let left = total / 2;
    let right = total - left;
    format!("{}{}{}", " ".repeat(left), v, " ".repeat(right))
}

// --- windows-style path ----------------------------------------------------

/// Return the last component of a Windows-style path (splits on `\` or `/`).
fn win_basename(v: &str) -> String {
    v.rsplit(['\\', '/']).next().unwrap_or(v).to_string()
}

/// Return everything before the last `\` or `/` in a Windows-style path.
fn win_dirname(v: &str) -> String {
    v.rfind(['\\', '/'])
        .map_or_else(String::new, |i| v[..i].to_string())
}

/// Split a Windows-style path into `[stem, ext]` on the last `.` of the basename.
fn win_splitext(v: &str) -> Value {
    let base = v.rsplit(['\\', '/']).next().unwrap_or(v);
    let (stem, ext) = match base.rfind('.') {
        Some(0) | None => (base.to_string(), String::new()),
        Some(i) => (base[..i].to_string(), base[i..].to_string()),
    };
    Value::from(vec![stem, ext])
}

// --- regex match positions -------------------------------------------------

/// Return `[start, end]` byte offsets for each regex match as a list of pairs.
fn regex_findall_ind(v: &str, pattern: &str) -> Result<Value, Error> {
    let re = build_regex(pattern)?;
    let pairs: Vec<serde_json::Value> = re
        .captures_iter(v)
        .filter_map(|c| c.get(0).map(|m| serde_json::json!([m.start(), m.end()])))
        .collect();
    Ok(Value::from_serialize(serde_json::Value::Array(pairs)))
}

// --- random element --------------------------------------------------------

/// Pick a random element: list → one item, string → one char, int N → random `0..N`.
fn random(v: &Value) -> Result<Value, Error> {
    use rand::RngExt;
    let jv = mj_to_json(v)?;
    let mut rng = rand::rng();
    match jv {
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                return Ok(Value::UNDEFINED);
            }
            let idx = rng.random_range(0..arr.len());
            Ok(Value::from_serialize(&arr[idx]))
        }
        serde_json::Value::String(s) => {
            let chars: Vec<char> = s.chars().collect();
            if chars.is_empty() {
                return Ok(Value::UNDEFINED);
            }
            let idx = rng.random_range(0..chars.len());
            Ok(Value::from(chars[idx].to_string()))
        }
        serde_json::Value::Number(n) => {
            let bound = n.as_i64().unwrap_or(0);
            if bound <= 0 {
                return Ok(Value::from(0));
            }
            Ok(Value::from(rng.random_range(0..bound)))
        }
        _ => Err(mj_err(
            ErrorKind::InvalidOperation,
            "random: expected a list, string, or number",
        )),
    }
}

// --- ip validation / wrapping ----------------------------------------------

/// Validate an IPv4 address; return it on success, undefined otherwise.
fn ipv4(v: &Value) -> Value {
    use std::net::Ipv4Addr;
    use std::str::FromStr;
    match v.as_str() {
        Some(s) if Ipv4Addr::from_str(s).is_ok() => Value::from(s),
        _ => Value::UNDEFINED,
    }
}

/// Validate an IPv6 address; return it on success, undefined otherwise.
fn ipv6(v: &Value) -> Value {
    use std::net::Ipv6Addr;
    use std::str::FromStr;
    match v.as_str() {
        Some(s) if Ipv6Addr::from_str(s).is_ok() => Value::from(s),
        _ => Value::UNDEFINED,
    }
}

/// Wrap an IPv6 address in `[...]`; pass through anything else unchanged.
fn ipwrap(v: &str) -> String {
    use std::net::Ipv6Addr;
    use std::str::FromStr;
    if Ipv6Addr::from_str(v).is_ok() {
        format!("[{v}]")
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::templating::engine::build_environment;

    fn env() -> minijinja::Environment<'static> {
        build_environment()
    }

    fn render(tmpl: &str, ctx: &serde_json::Value) -> String {
        env()
            .render_str(tmpl, ctx)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn mandatory_undefined_errors() {
        let e = env().render_str("{{ x | mandatory }}", serde_json::json!({}));
        assert!(e.is_err());
    }

    #[test]
    fn to_yaml_and_back() {
        let ctx = serde_json::json!({"d": {"a": 1, "b": [2, 3]}});
        let out = render("{{ d | to_yaml | from_yaml }}", &ctx);
        // round-trip yields the same structure printed by minijinja
        assert!(out.contains("\"a\"") || out.contains("'a'"));
    }

    #[test]
    fn b64_roundtrip() {
        let ctx = serde_json::json!({"s": "hello"});
        assert_eq!(render("{{ s | b64encode | b64decode }}", &ctx), "hello");
    }

    #[test]
    fn regex_replace_basic() {
        let ctx = serde_json::json!({"s": "a1b2"});
        assert_eq!(
            render("{{ s | regex_replace('[0-9]', 'X') }}", &ctx),
            "aXbX"
        );
    }

    #[test]
    fn regex_search_group() {
        let ctx = serde_json::json!({"s": "user=42"});
        assert_eq!(
            render("{{ s | regex_search('user=(\\\\d+)') }}", &ctx),
            "42"
        );
    }

    #[test]
    fn regex_findall_all() {
        let ctx = serde_json::json!({"s": "a1 b2 c3"});
        let out = render("{{ s | regex_findall('\\\\w(\\d)') | join(',') }}", &ctx);
        assert_eq!(out, "1,2,3");
    }

    #[test]
    fn dict2items_default_keys() {
        let ctx = serde_json::json!({"d": {"x": 1}});
        let out = render("{{ d | dict2items }}", &ctx);
        assert!(out.contains("\"key\"") || out.contains("'key'"));
        assert!(out.contains("\"value\"") || out.contains("'value'"));
    }

    #[test]
    fn items2dict_roundtrip() {
        let ctx = serde_json::json!({"items": [{"key": "a", "value": 1}]});
        let out = render("{{ items | items2dict }}", &ctx);
        assert!(out.contains("\"a\"") || out.contains("'a'"));
    }

    #[test]
    fn difference_and_intersect() {
        let ctx = serde_json::json!({"a": [1,2,3], "b": [2,3,4]});
        assert_eq!(render("{{ a | difference(b) | join(',') }}", &ctx), "1");
        assert_eq!(
            render("{{ a | intersect(b) | sort | join(',') }}", &ctx),
            "2,3"
        );
        assert_eq!(
            render("{{ a | union(b) | sort | join(',') }}", &ctx),
            "1,2,3,4"
        );
    }

    #[test]
    fn combine_shallow() {
        let ctx = serde_json::json!({"a": {"x": 1}, "b": {"y": 2}});
        let out = render("{{ a | combine(b) }}", &ctx);
        assert!(out.contains("\"x\"") || out.contains("'x'"));
        assert!(out.contains("\"y\"") || out.contains("'y'"));
    }

    #[test]
    fn ternary_selects() {
        assert_eq!(
            render("{{ true | ternary('yes', 'no') }}", &serde_json::json!({})),
            "yes"
        );
        assert_eq!(
            render("{{ false | ternary('yes', 'no') }}", &serde_json::json!({})),
            "no"
        );
    }

    #[test]
    fn quote_wraps_single_quotes() {
        let ctx = serde_json::json!({"s": "a'b"});
        let out = render("{{ s | quote }}", &ctx);
        assert_eq!(out, "'a'\\''b'");
    }

    #[test]
    fn password_hash_bcrypt_prefix() {
        let ctx = serde_json::json!({"s": "secret"});
        let out = render("{{ s | password_hash }}", &ctx);
        assert!(out.starts_with("$2"), "got {out}");
    }

    #[test]
    fn path_filters() {
        let ctx = serde_json::json!({"p": "/a/b/c.txt"});
        assert_eq!(render("{{ p | basename }}", &ctx), "c.txt");
        assert_eq!(render("{{ p | dirname }}", &ctx), "/a/b");
        let ext = render("{{ p | splitext }}", &ctx);
        assert!(ext.contains('c') && ext.contains("txt"));
    }

    #[test]
    fn flatten_nested() {
        let ctx = serde_json::json!({"s": [[1, 2], [3, [4, 5]]]});
        assert_eq!(render("{{ s | flatten | join(',') }}", &ctx), "1,2,3,4,5");
    }

    #[test]
    fn flatten_with_depth() {
        let ctx = serde_json::json!({"s": [[1, 2], [3, [4, 5]]]});
        assert_eq!(render("{{ s | flatten(1) | length }}", &ctx), "2");
        assert_eq!(render("{{ s | flatten(2) | length }}", &ctx), "4");
        // no depth arg == full flatten
        assert_eq!(render("{{ s | flatten | length }}", &ctx), "5");
    }

    #[test]
    fn hash_sha256_and_md5() {
        // sha256("abc") == ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let ctx = serde_json::json!({"s": "abc"});
        assert_eq!(
            render("{{ s | hash('sha256') }}", &ctx),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // md5("abc") == 900150983cd24fb0d6963f7d28e17f72
        assert_eq!(
            render("{{ s | hash('md5') }}", &ctx),
            "900150983cd24fb0d6963f7d28e17f72"
        );
    }

    #[test]
    fn hash_default_sha256() {
        let ctx = serde_json::json!({"s": "abc"});
        assert_eq!(
            render("{{ s | hash }}", &ctx),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hash_sha512_length() {
        let ctx = serde_json::json!({"s": "abc"});
        let out = render("{{ s | hash('sha512') }}", &ctx);
        assert_eq!(out.len(), 128); // 64 bytes * 2 hex chars
    }

    #[test]
    fn hash_sha1_errors() {
        let env = env();
        let res = env.render_str("{{ 'x' | hash('sha1') }}", serde_json::json!({}));
        assert!(res.is_err());
    }

    #[test]
    fn to_nice_json_pretty() {
        let ctx = serde_json::json!({"d": {"a": 1}});
        let out = render("{{ d | to_nice_json }}", &ctx);
        assert!(out.contains('\n'));
        assert!(out.contains("\"a\""));
    }

    #[test]
    fn to_nice_yaml_pretty() {
        let ctx = serde_json::json!({"d": {"a": 1}});
        let out = render("{{ d | to_nice_yaml }}", &ctx);
        assert!(out.contains("a:"));
    }

    #[test]
    fn from_yaml_all_multidoc() {
        let ctx = serde_json::json!({"s": "a: 1\n---\nb: 2"});
        let out = render("{{ s | from_yaml_all | length }}", &ctx);
        assert_eq!(out, "2");
    }

    #[test]
    fn path_join_basic() {
        let ctx = serde_json::json!({"a": "/etc", "b": "nginx/conf"});
        assert_eq!(render("{{ a | path_join(b) }}", &ctx), "/etc/nginx/conf");
    }

    #[test]
    fn path_join_collapses_slashes() {
        let ctx = serde_json::json!({"a": "/etc/", "b": "/x"});
        assert_eq!(render("{{ a | path_join(b) }}", &ctx), "/etc/x");
    }

    #[test]
    fn strftime_epoch() {
        // 0 == 1970-01-01T00:00:00Z
        let ctx = serde_json::json!({"t": 0});
        assert_eq!(render("{{ t | strftime('%Y-%m-%d') }}", &ctx), "1970-01-01");
    }

    #[test]
    fn strftime_iso_string() {
        let ctx = serde_json::json!({"t": "2020-01-02T03:04:05Z"});
        assert_eq!(render("{{ t | strftime('%Y') }}", &ctx), "2020");
    }

    #[test]
    fn to_datetime_default_fmt() {
        let ctx = serde_json::json!({"t": "2020-01-02 03:04:05"});
        let out = render("{{ t | to_datetime }}", &ctx);
        assert!(out.starts_with("2020-01-02T03:04:05"));
    }

    #[test]
    fn urlencode_basic() {
        let ctx = serde_json::json!({"s": "a b/c"});
        assert_eq!(render("{{ s | urlencode }}", &ctx), "a%20b%2Fc");
    }

    #[test]
    fn urlencode_unreserved_kept() {
        let ctx = serde_json::json!({"s": "AZaz09-_.~"});
        assert_eq!(render("{{ s | urlencode }}", &ctx), "AZaz09-_.~");
    }

    #[test]
    fn urlsplit_full() {
        let ctx = serde_json::json!({"u": "http://example.com/foo?a=1#bar"});
        let out = render("{{ u | urlsplit }}", &ctx);
        assert!(out.contains("example.com"));
        assert!(out.contains("/foo"));
        assert!(out.contains("a=1"));
        assert!(out.contains("bar"));
    }

    #[test]
    fn type_debug_string() {
        let ctx = serde_json::json!({"s": "hi"});
        assert_eq!(render("{{ s | type_debug }}", &ctx), "string");
    }

    #[test]
    fn type_debug_number() {
        let ctx = serde_json::json!({"n": 5});
        assert_eq!(render("{{ n | type_debug }}", &ctx), "number");
    }

    #[test]
    fn human_readable_units() {
        let ctx = serde_json::json!({"a": 1536, "b": 1_073_741_824, "z": 0});
        assert_eq!(render("{{ a | human_readable }}", &ctx), "1.5 KB");
        assert_eq!(render("{{ b | human_readable }}", &ctx), "1.0 GB");
        assert_eq!(render("{{ z | human_readable }}", &ctx), "0 B");
    }

    #[test]
    fn human_to_bytes_roundtrip() {
        let ctx = serde_json::json!({});
        assert_eq!(
            render("{{ '1.5 GB' | human_to_bytes }}", &ctx),
            "1610612736"
        );
        assert_eq!(render("{{ '500MB' | human_to_bytes }}", &ctx), "524288000");
        assert_eq!(
            render("{{ '1073741824' | human_to_bytes }}", &ctx),
            "1073741824"
        );
    }

    #[test]
    fn center_pads() {
        let ctx = serde_json::json!({"s": "hi"});
        assert_eq!(render("{{ s | center(6) | length }}", &ctx), "6");
        let out = render("{{ s | center(6) }}", &ctx);
        assert!(out.starts_with(' ') && out.ends_with(' '));
        assert!(out.contains("hi"));
    }

    #[test]
    fn center_wide_string_unchanged() {
        let ctx = serde_json::json!({"s": "hello"});
        assert_eq!(render("{{ s | center(3) }}", &ctx), "hello");
    }

    #[test]
    fn win_basename_and_dirname() {
        let ctx = serde_json::json!({"p": "C:\\Users\\me\\file.txt"});
        assert_eq!(render("{{ p | win_basename }}", &ctx), "file.txt");
        assert_eq!(render("{{ p | win_dirname }}", &ctx), "C:\\Users\\me");
    }

    #[test]
    fn win_splitext_basic() {
        let ctx = serde_json::json!({"p": "C:\\dir\\archive.tar.gz"});
        let out = render("{{ p | win_splitext }}", &ctx);
        assert!(out.contains("archive.tar"));
        assert!(out.contains(".gz"));
    }

    #[test]
    fn regex_findall_ind_offsets() {
        let ctx = serde_json::json!({"s": "a1 b2"});
        let out = render("{{ s | regex_findall_ind('\\d') | length }}", &ctx);
        assert_eq!(out, "2");
    }

    #[test]
    fn random_from_list_in_set() {
        let env = env();
        let ctx = serde_json::json!({"l": [10, 20, 30]});
        let out = env.render_str("{{ l | random }}", &ctx).unwrap_or_default();
        assert!(out == "10" || out == "20" || out == "30");
    }

    #[test]
    fn random_from_int_in_range() {
        let env = env();
        let ctx = serde_json::json!({"n": 5});
        let out: i64 = env
            .render_str("{{ n | random }}", &ctx)
            .unwrap_or_default()
            .parse()
            .unwrap_or(-1);
        assert!((0..5).contains(&out));
    }

    #[test]
    fn ipv4_valid_and_invalid() {
        let env = env();
        let ctx = serde_json::json!({"g": "192.168.1.1", "b": "999.1.1.1"});
        assert_eq!(
            env.render_str("{{ g | ipv4 }}", &ctx).unwrap_or_default(),
            "192.168.1.1"
        );
        let bad = env.render_str("{% if b | ipv4 %}yes{% else %}no{% endif %}", &ctx);
        assert_eq!(bad.unwrap_or_default(), "no");
    }

    #[test]
    fn ipv6_valid_and_invalid() {
        let env = env();
        let ctx = serde_json::json!({"g": "::1", "b": "nope"});
        assert_eq!(
            env.render_str("{{ g | ipv6 }}", &ctx).unwrap_or_default(),
            "::1"
        );
        let bad = env.render_str("{% if b | ipv6 %}yes{% else %}no{% endif %}", &ctx);
        assert_eq!(bad.unwrap_or_default(), "no");
    }

    #[test]
    fn ipwrap_wraps_ipv6_only() {
        let ctx = serde_json::json!({"v6": "::1", "v4": "1.2.3.4"});
        assert_eq!(render("{{ v6 | ipwrap }}", &ctx), "[::1]");
        assert_eq!(render("{{ v4 | ipwrap }}", &ctx), "1.2.3.4");
    }
}
