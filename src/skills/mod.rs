//! Multi-platform skill installer. Renders one canonical guidance into each host's native
//! format, detecting present targets, with a versioned marker for idempotent install/uninstall.
//! Writing text files is NOT covered by the "never install" policy (which is about LSP/SCIP).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const MARK_BEGIN: &str = "<!-- codemap:skill v1 -->";
const MARK_END: &str = "<!-- /codemap:skill -->";
const DESCRIPTION: &str = "Navigate code by symbol BEFORE Grep/Read: resolve_symbol -> outline/callers/callees (locations only) -> read_symbol (the only tool that returns code).";

fn body() -> String {
    "\
Use codemap's MCP tools to navigate code by symbol BEFORE using Grep or Read.

Cheap-to-expensive ladder:
1. codemap_resolve_symbol(name) -> stable ids + name_paths (no code)
2. codemap_get_file_outline / codemap_get_callers / codemap_get_callees -> locations only
3. codemap_read_symbol(id) -> the ONLY tool that returns code (minimal range)

Rules:
- Do NOT Grep or Read to discover where something is defined or who calls it — use resolve/callers.
- If you think \"one Read is faster than three calls\", that thought is the signal to use codemap tools.
- Trust edges by their prov/res tag (scip/lsp resolved > tree_sitter ambiguous); prefer resolved edges over guesses."
        .to_string()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Claude,
    Cursor,
    Copilot,
    Agents,
    Kilo,
}

impl Target {
    pub fn all() -> [Target; 5] {
        [
            Target::Claude,
            Target::Cursor,
            Target::Copilot,
            Target::Agents,
            Target::Kilo,
        ]
    }

    pub fn id(self) -> &'static str {
        match self {
            Target::Claude => "claude",
            Target::Cursor => "cursor",
            Target::Copilot => "copilot",
            Target::Agents => "agents",
            Target::Kilo => "kilo",
        }
    }

    pub fn from_id(s: &str) -> Option<Target> {
        Target::all().into_iter().find(|t| t.id() == s)
    }

    /// File path for this target's skill, or None if it can't be located (e.g. no HOME).
    fn path(self, root: &Path) -> Option<PathBuf> {
        match self {
            Target::Claude => Some(root.join(".claude/skills/codemap/SKILL.md")),
            Target::Cursor => Some(root.join(".cursor/rules/codemap.mdc")),
            Target::Copilot => Some(root.join(".github/copilot-instructions.md")),
            Target::Agents => Some(root.join("AGENTS.md")),
            Target::Kilo => home().map(|h| h.join(".config/kilo/skills/codemap/SKILL.md")),
        }
    }

    /// Existing path whose presence means the host is in use here.
    fn anchor(self, root: &Path) -> Option<PathBuf> {
        match self {
            Target::Claude => Some(root.join(".claude")),
            Target::Cursor => Some(root.join(".cursor")),
            Target::Copilot => Some(root.join(".github")),
            Target::Agents => Some(root.join("AGENTS.md")),
            Target::Kilo => home().map(|h| h.join(".config/kilo")),
        }
    }

    /// Dedicated file (whole file is ours) vs a marked section inside a shared file.
    fn dedicated(self) -> bool {
        !matches!(self, Target::Copilot | Target::Agents)
    }

    fn rendered(self) -> String {
        let block = format!("{MARK_BEGIN}\n{}\n{MARK_END}\n", body());
        match self {
            Target::Claude | Target::Kilo => {
                format!("---\nname: codemap\ndescription: {DESCRIPTION}\n---\n\n{block}")
            }
            Target::Cursor => format!("---\nalwaysApply: true\n---\n\n{block}"),
            Target::Copilot | Target::Agents => {
                format!("{MARK_BEGIN}\n## codemap\n\n{}\n{MARK_END}", body())
            }
        }
    }
}

#[derive(Debug)]
pub enum Action {
    Written,
    Updated,
    Removed,
    Skipped(String),
    WouldWrite,
}

#[derive(Debug)]
pub struct Report {
    pub target: &'static str,
    pub path: String,
    pub action: Action,
}

pub fn detect(root: &Path) -> Vec<Target> {
    Target::all()
        .into_iter()
        .filter(|t| t.anchor(root).map(|p| p.exists()).unwrap_or(false))
        .collect()
}

/// Install to the given targets (empty = auto-detected). `dry` only reports.
pub fn install(root: &Path, only: &[Target], dry: bool) -> Result<Vec<Report>> {
    let targets = if only.is_empty() {
        detect(root)
    } else {
        only.to_vec()
    };
    let mut reports = Vec::new();
    for t in targets {
        let Some(path) = t.path(root) else {
            reports.push(Report {
                target: t.id(),
                path: "<no HOME>".into(),
                action: Action::Skipped("cannot locate".into()),
            });
            continue;
        };
        let existed = path.exists();
        if dry {
            reports.push(Report {
                target: t.id(),
                path: disp(&path),
                action: Action::WouldWrite,
            });
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("create {}", disp(parent)))?;
        }
        let content = if t.dedicated() {
            t.rendered()
        } else {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            upsert_section(&existing, &t.rendered())
        };
        std::fs::write(&path, content).with_context(|| format!("write {}", disp(&path)))?;
        reports.push(Report {
            target: t.id(),
            path: disp(&path),
            action: if existed {
                Action::Updated
            } else {
                Action::Written
            },
        });
    }
    Ok(reports)
}

/// Remove codemap's skill from the given targets (empty = all known).
pub fn uninstall(root: &Path, only: &[Target]) -> Result<Vec<Report>> {
    let targets = if only.is_empty() {
        Target::all().to_vec()
    } else {
        only.to_vec()
    };
    let mut reports = Vec::new();
    for t in targets {
        let Some(path) = t.path(root) else { continue };
        if !path.exists() {
            continue;
        }
        if t.dedicated() {
            let owned = std::fs::read_to_string(&path)
                .map(|c| c.contains(MARK_BEGIN))
                .unwrap_or(false);
            if owned {
                std::fs::remove_file(&path).with_context(|| format!("remove {}", disp(&path)))?;
                reports.push(Report {
                    target: t.id(),
                    path: disp(&path),
                    action: Action::Removed,
                });
            }
        } else {
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            if existing.contains(MARK_BEGIN) {
                std::fs::write(&path, strip_section(&existing))?;
                reports.push(Report {
                    target: t.id(),
                    path: disp(&path),
                    action: Action::Removed,
                });
            }
        }
    }
    Ok(reports)
}

fn upsert_section(existing: &str, block: &str) -> String {
    if let (Some(b), Some(e)) = (existing.find(MARK_BEGIN), existing.find(MARK_END)) {
        let end = e + MARK_END.len();
        format!("{}{block}{}", &existing[..b], &existing[end..])
    } else if existing.trim().is_empty() {
        format!("{block}\n")
    } else {
        format!("{}\n\n{block}\n", existing.trim_end())
    }
}

fn strip_section(existing: &str) -> String {
    if let (Some(b), Some(e)) = (existing.find(MARK_BEGIN), existing.find(MARK_END)) {
        let end = e + MARK_END.len();
        let head = existing[..b].trim_end();
        let tail = existing[end..].trim_start();
        if head.is_empty() {
            format!("{tail}\n").trim_start().to_string()
        } else if tail.is_empty() {
            format!("{head}\n")
        } else {
            format!("{head}\n\n{tail}\n")
        }
    } else {
        existing.to_string()
    }
}

const HOOK_BEGIN: &str = "# >>> codemap-hook >>>";
const HOOK_END: &str = "# <<< codemap-hook <<<";
const HOOK_NAMES: &[&str] = &["post-commit", "post-merge", "post-checkout"];

/// Install opt-in git hooks that run `codemap index --incremental` after commit/merge/checkout.
pub fn install_hooks(root: &Path) -> Result<Vec<Report>> {
    if !root.join(".git").exists() {
        return Ok(vec![Report {
            target: "git-hooks",
            path: disp(&root.join(".git")),
            action: Action::Skipped("no .git".into()),
        }]);
    }
    let hooks_dir = root.join(".git/hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let block = format!(
        "{HOOK_BEGIN}\ncommand -v codemap >/dev/null 2>&1 && codemap index --incremental >/dev/null 2>&1 || true\n{HOOK_END}"
    );
    let mut reports = Vec::new();
    for name in HOOK_NAMES {
        let path = hooks_dir.join(name);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let existed = !existing.is_empty();
        let content = if existing.contains(HOOK_BEGIN) {
            replace_block(&existing, &block)
        } else if existing.trim().is_empty() {
            format!("#!/bin/sh\n{block}\n")
        } else {
            format!("{}\n{block}\n", existing.trim_end())
        };
        std::fs::write(&path, content)?;
        set_exec(&path)?;
        reports.push(Report {
            target: "git-hooks",
            path: disp(&path),
            action: if existed {
                Action::Updated
            } else {
                Action::Written
            },
        });
    }
    Ok(reports)
}

pub fn uninstall_hooks(root: &Path) -> Result<Vec<Report>> {
    let hooks_dir = root.join(".git/hooks");
    let mut reports = Vec::new();
    for name in HOOK_NAMES {
        let path = hooks_dir.join(name);
        let existing = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if existing.contains(HOOK_BEGIN) {
            std::fs::write(&path, strip_block(&existing))?;
            reports.push(Report {
                target: "git-hooks",
                path: disp(&path),
                action: Action::Removed,
            });
        }
    }
    Ok(reports)
}

fn replace_block(existing: &str, block: &str) -> String {
    if let (Some(b), Some(e)) = (existing.find(HOOK_BEGIN), existing.find(HOOK_END)) {
        format!(
            "{}{}{}",
            &existing[..b],
            block,
            &existing[e + HOOK_END.len()..]
        )
    } else {
        existing.to_string()
    }
}

fn strip_block(existing: &str) -> String {
    if let (Some(b), Some(e)) = (existing.find(HOOK_BEGIN), existing.find(HOOK_END)) {
        let head = existing[..b].trim_end();
        let tail = existing[e + HOOK_END.len()..].trim_start();
        format!("{head}\n{tail}").trim().to_string() + "\n"
    } else {
        existing.to_string()
    }
}

#[cfg(unix)]
fn set_exec(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_exec(_path: &Path) -> Result<()> {
    Ok(())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn disp(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_detects_and_writes_then_uninstalls() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::create_dir_all(root.join(".github")).unwrap();

        let detected = detect(root);
        assert!(detected.contains(&Target::Claude));
        assert!(detected.contains(&Target::Copilot));
        assert!(!detected.contains(&Target::Cursor));

        install(root, &[], false).unwrap();
        let skill = root.join(".claude/skills/codemap/SKILL.md");
        let copilot = root.join(".github/copilot-instructions.md");
        assert!(std::fs::read_to_string(&skill)
            .unwrap()
            .contains("name: codemap"));
        assert!(std::fs::read_to_string(&copilot)
            .unwrap()
            .contains(MARK_BEGIN));

        // Idempotent: a second install keeps a single marked block.
        install(root, &[], false).unwrap();
        let c = std::fs::read_to_string(&copilot).unwrap();
        assert_eq!(c.matches(MARK_BEGIN).count(), 1);

        uninstall(root, &[]).unwrap();
        assert!(!skill.exists());
        assert!(!std::fs::read_to_string(&copilot)
            .unwrap()
            .contains(MARK_BEGIN));
    }

    #[test]
    fn hooks_install_and_uninstall() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        // Pre-existing post-commit hook with user content must be preserved.
        std::fs::write(
            root.join(".git/hooks/post-commit"),
            "#!/bin/sh\necho mine\n",
        )
        .unwrap();

        install_hooks(root).unwrap();
        let pc = std::fs::read_to_string(root.join(".git/hooks/post-commit")).unwrap();
        assert!(pc.contains("echo mine"), "user content preserved");
        assert!(pc.contains("codemap index --incremental"));
        assert!(root.join(".git/hooks/post-merge").exists());

        uninstall_hooks(root).unwrap();
        let pc2 = std::fs::read_to_string(root.join(".git/hooks/post-commit")).unwrap();
        assert!(pc2.contains("echo mine"));
        assert!(!pc2.contains(HOOK_BEGIN));
    }

    #[test]
    fn uninstall_preserves_surrounding_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("AGENTS.md"), "# My agents\n\nKeep me.\n").unwrap();

        install(root, &[Target::Agents], false).unwrap();
        let after = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(after.contains("Keep me."));
        assert!(after.contains(MARK_BEGIN));

        uninstall(root, &[Target::Agents]).unwrap();
        let stripped = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(stripped.contains("Keep me."));
        assert!(!stripped.contains(MARK_BEGIN));
    }
}
