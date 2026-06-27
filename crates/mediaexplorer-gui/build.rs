//! Build script: embed the current git short hash and commit date so the
//! About window can show which revision the binary was built from. Both fall
//! back to empty strings when git is unavailable (e.g. building from a source
//! tarball without a `.git` directory), and the About window hides the
//! corresponding row.

use std::path::Path;
use std::process::Command;

fn main() {
    let hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let date = git(&["log", "-1", "--date=short", "--format=%cd"]).unwrap_or_default();

    println!("cargo:rustc-env=GIT_HASH={hash}");
    println!("cargo:rustc-env=BUILD_DATE={date}");

    // Re-run when the checked-out revision changes so the values stay fresh on
    // incremental builds. Watch HEAD, the branch ref it points at, and
    // packed-refs (which holds refs that have not been written out loosely).
    if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
        let git_dir = Path::new(&git_dir);
        let head = git_dir.join("HEAD");
        if let Ok(contents) = std::fs::read_to_string(&head) {
            if let Some(reference) = contents.strip_prefix("ref:").map(str::trim) {
                println!(
                    "cargo:rerun-if-changed={}",
                    git_dir.join(reference).display()
                );
            }
        }
        println!("cargo:rerun-if-changed={}", head.display());
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join("packed-refs").display()
        );
    }
}

/// Run `git` with the given arguments, returning trimmed stdout on success.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}
