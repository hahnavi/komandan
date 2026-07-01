//! Playbook parsing: YAML → [`model`] IR.

pub mod model;
pub mod yaml;

use std::path::Path;

use crate::error::ParseError;

pub use model::{
    Block, Expr, GatherFacts, HostMatcher, LoopSource, LoopSpec, ModuleRef, Play, Playbook,
    RoleRef, Serial, Task, TaskNode, Vars,
};
pub use model::{leaf_tasks, play_leaf_tasks};
pub use yaml::{parse_playbook, parse_tasks_text, parse_vars_text};

/// Parse a playbook document from disk.
///
/// `import_playbook:` directives are expanded recursively: each referenced
/// file is loaded relative to the importing file and its plays spliced inline.
///
/// # Errors
///
/// [`ParseError::Load`] if the file cannot be read; [`ParseError::Yaml`] /
/// [`ParseError::Play`] / [`ParseError::Task`] for parse/structural problems.
pub fn parse_playbook_file(path: &Path) -> Result<Playbook, ParseError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        ParseError::load(format!("failed to read playbook {}: {e}", path.display()))
    })?;
    let mut pb = parse_playbook(&text)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    expand_imports(&mut pb, base_dir, 0)?;
    Ok(pb)
}

/// Recursively expand `import_playbook` directives in a parsed playbook,
/// loading referenced files relative to `base_dir` and splicing their plays
/// inline. Depth-limited to prevent infinite recursion from circular imports.
///
/// # Errors
///
/// [`ParseError::Load`] if an imported file cannot be read; [`ParseError`]
/// variants from re-parsing the imported file. [`ParseError::Play`] if the
/// recursion depth exceeds 20 (likely a circular import).
fn expand_imports(pb: &mut Playbook, base_dir: &Path, depth: u32) -> Result<(), ParseError> {
    if depth > 20 {
        return Err(ParseError::play(
            "import_playbook recursion too deep (>20 levels); possible circular import".to_string(),
        ));
    }
    let mut expanded: Vec<Play> = Vec::new();
    for play in pb.0.drain(..) {
        if let Some(import_path) = play.import_playbook {
            let resolved = base_dir.join(&import_path);
            let text = std::fs::read_to_string(&resolved).map_err(|e| {
                ParseError::load(format!(
                    "failed to import_playbook {}: {e}",
                    resolved.display()
                ))
            })?;
            let mut imported = parse_playbook(&text)?;
            let import_base = resolved.parent().unwrap_or_else(|| Path::new("."));
            expand_imports(&mut imported, import_base, depth + 1)?;
            expanded.extend(imported.0);
        } else {
            expanded.push(play);
        }
    }
    pb.0 = expanded;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ParseError;
    use tempfile::TempDir;

    #[test]
    fn import_playbook_splices_plays() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(
            base.join("web.yml"),
            "- hosts: all\n  tasks:\n    - debug: msg=web\n",
        )?;
        std::fs::write(
            base.join("site.yml"),
            "- import_playbook: web.yml\n- hosts: all\n  tasks:\n    - debug: msg=main\n",
        )?;
        let pb = parse_playbook_file(&base.join("site.yml"))?;
        assert_eq!(pb.0.len(), 2, "should have 2 plays after expansion");
        assert!(
            pb.0[0].import_playbook.is_none(),
            "expanded plays should not have import_playbook"
        );
        assert!(pb.0[1].import_playbook.is_none());
        Ok(())
    }

    #[test]
    fn import_playbook_recursive() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(
            base.join("inner.yml"),
            "- hosts: all\n  tasks:\n    - debug: msg=inner\n",
        )?;
        std::fs::write(base.join("middle.yml"), "- import_playbook: inner.yml\n")?;
        std::fs::write(base.join("site.yml"), "- import_playbook: middle.yml\n")?;
        let pb = parse_playbook_file(&base.join("site.yml"))?;
        assert_eq!(pb.0.len(), 1, "recursive import should yield 1 play");
        assert!(pb.0[0].import_playbook.is_none());
        Ok(())
    }

    #[test]
    fn import_playbook_detects_cycle() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        std::fs::write(base.join("a.yml"), "- import_playbook: b.yml\n")?;
        std::fs::write(base.join("b.yml"), "- import_playbook: a.yml\n")?;
        let result = parse_playbook_file(&base.join("a.yml"));
        assert!(result.is_err(), "circular import should fail");
        let msg = match result {
            Err(ParseError::Play(m)) => m,
            other => panic!("expected ParseError::Play, got {other:?}"),
        };
        assert!(
            msg.contains("recursion"),
            "error should mention recursion: {msg}"
        );
        Ok(())
    }

    #[test]
    fn import_playbook_resolves_relative_to_importing_file() -> anyhow::Result<()> {
        let tmp = TempDir::new()?;
        let base = tmp.path();
        // web.yml is in a subdirectory; the importing playbook references it
        // relative to its own location.
        std::fs::create_dir_all(base.join("plays"))?;
        std::fs::write(
            base.join("plays/web.yml"),
            "- hosts: all\n  tasks:\n    - debug: msg=web\n",
        )?;
        std::fs::write(base.join("site.yml"), "- import_playbook: plays/web.yml\n")?;
        let pb = parse_playbook_file(&base.join("site.yml"))?;
        assert_eq!(pb.0.len(), 1);
        Ok(())
    }
}
