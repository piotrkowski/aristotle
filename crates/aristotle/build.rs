// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sets `ARISTOTLE_COMMIT`, the short commit the app was built from, for
//! Settings › About. It comes from git, or from the `REVISION` file of a
//! release's source tarball (`tools/package.sh source`), and is empty when
//! neither is there.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    // Only this repository's own git, not one the source tree was unpacked into.
    let toplevel = git(&root, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let commit = if toplevel.is_some_and(|t| t.canonicalize().ok() == Some(root.clone())) {
        // Rebuild when HEAD moves: HEAD itself, the branch it points to, and
        // packed-refs (only files that exist, or Cargo would rerun every time).
        let path = |p: &str| {
            git(
                &root,
                &["rev-parse", "--path-format=absolute", "--git-path", p],
            )
        };
        let mut watch = vec![path("HEAD"), path("packed-refs")];
        if let Some(head) = watch[0]
            .as_ref()
            .and_then(|h| std::fs::read_to_string(h).ok())
        {
            if let Some(r) = head.trim().strip_prefix("ref: ") {
                watch.push(path(r));
            }
        }
        for p in watch.into_iter().flatten() {
            if Path::new(&p).exists() {
                println!("cargo:rerun-if-changed={p}");
            }
        }
        git(&root, &["rev-parse", "--short=7", "HEAD"])
    } else {
        let rev = root.join("REVISION");
        println!("cargo:rerun-if-changed={}", rev.display());
        std::fs::read_to_string(rev).ok().and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("aristotle "))
                .map(|c| c.trim().chars().take(7).collect())
        })
    };
    println!(
        "cargo:rustc-env=ARISTOTLE_COMMIT={}",
        commit.unwrap_or_default()
    );
}
