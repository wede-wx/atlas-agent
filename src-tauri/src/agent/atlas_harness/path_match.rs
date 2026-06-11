//! Atlas Harness — path & glob matching (hardened, dependency-free).
//!
//! Why this file exists
//! --------------------
//! The original `contract_gate` matched a Preserve glob against the *raw*
//! `target_path` string via a tiny regex. That had three silent-bypass holes,
//! and a silent bypass of a **hard Preserve** is the single most dangerous
//! failure mode for a goal-fidelity gate (the action is forbidden but slips
//! through unnoticed):
//!
//!   1. No path normalization. `src/ui/**` did not match `./src/ui/App.tsx`,
//!      `src/ui/../ui/App.tsx`, or a Windows-style `src\ui\App.tsx`.
//!   2. Absolute paths. A preserve of `src/ui/**` did not match an absolute
//!      `/home/u/proj/src/ui/App.tsx` even though it is the same file.
//!   3. A regex dependency for what is fundamentally a glob.
//!
//! Design choices
//! --------------
//! * **Pure & lexical.** No filesystem access. `std::fs::canonicalize` would
//!   touch disk and fail for not-yet-created files, and would make the gate
//!   non-deterministic. We normalize `.`/`..`/`\`/`//` lexically instead.
//! * **Suffix matching.** A relative Preserve glob (`src/ui/**`) is matched
//!   against the path *and* against every leading-directory-stripped suffix of
//!   it. This closes the absolute-prefix / nested-root hole without knowing the
//!   workspace root.
//! * **Bias toward over-matching.** For a *safety* boundary the safe error
//!   direction is a false **block** (the user can override with a Deviation
//!   Notice), never a false **allow** (a silent bypass). Suffix matching can
//!   occasionally over-match a deeply-nested same-named directory; that is the
//!   intended, documented trade-off.
//!
//! The matcher itself is a small iterative backtracking glob (`**` = any,
//! including `/`; `*` = any run with no `/`; `?` = one non-`/` char). It was
//! validated against the full bypass + false-positive corpus before porting.

/// Normalize a path lexically (no I/O): `\`→`/`, collapse `//`, resolve `.`/`..`.
/// Absolute paths keep their leading `/`; `..` segments that would climb above
/// an absolute root are dropped.
pub fn normalize_rel_path(input: &str) -> String {
    let unified = input.replace('\\', "/");
    let is_abs = unified.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in unified.split('/') {
        match seg {
            "" | "." => continue,
            ".." => match out.last() {
                Some(&last) if last != ".." => {
                    out.pop();
                }
                _ => {
                    if !is_abs {
                        out.push("..");
                    }
                }
            },
            other => out.push(other),
        }
    }
    let joined = out.join("/");
    if is_abs {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Dependency-free glob match. `**` matches anything (including `/`); `*`
/// matches any run that contains no `/`; `?` matches exactly one non-`/` char.
/// Everything else is a literal. Both ends are implicitly anchored.
pub fn glob_match(glob: &str, path: &str) -> bool {
    let g: Vec<char> = glob.chars().collect();
    let t: Vec<char> = path.chars().collect();
    let (mut gi, mut ti) = (0usize, 0usize);
    // Backtrack bookmarks for the most recent `*` / `**`.
    let mut star: Option<usize> = None;
    let mut star_t = 0usize;
    let mut double = false;

    while ti < t.len() {
        if gi < g.len() && g[gi] == '?' {
            if t[ti] == '/' {
                break;
            }
            gi += 1;
            ti += 1;
            continue;
        }
        if gi < g.len() && g[gi] == '*' {
            if gi + 1 < g.len() && g[gi + 1] == '*' {
                double = true;
                star = Some(gi);
                gi += 2;
            } else {
                double = false;
                star = Some(gi);
                gi += 1;
            }
            star_t = ti;
            continue;
        }
        if gi < g.len() && g[gi] == t[ti] {
            gi += 1;
            ti += 1;
            continue;
        }
        // Mismatch: backtrack to the last star, extending what it consumed.
        match star {
            Some(s) => {
                // A single `*` may not cross a path separator.
                if !double && t.get(star_t) == Some(&'/') {
                    return false;
                }
                star_t += 1;
                ti = star_t;
                gi = s + if double { 2 } else { 1 };
            }
            None => return false,
        }
    }
    // Trailing `*` / `**` (and a dangling `/` from `a/**`) consume the empty tail.
    while gi < g.len() && (g[gi] == '*' || g[gi] == '/') {
        gi += 1;
    }
    gi == g.len()
}

/// Match a (possibly relative) glob against a path, trying the normalized path
/// and every leading-directory-stripped suffix. This is the function the
/// ContractGate should call — it closes the absolute / `./` / `..` / `\` holes.
pub fn path_matches_glob(glob: &str, path: &str) -> bool {
    let ng = normalize_rel_path(glob);
    let np = normalize_rel_path(path);
    let np = np.strip_prefix('/').unwrap_or(&np);
    let segs: Vec<&str> = np.split('/').filter(|s| !s.is_empty()).collect();
    for start in 0..segs.len().max(1) {
        let suffix = segs[start.min(segs.len())..].join("/");
        if glob_match(&ng, &suffix) {
            return true;
        }
    }
    false
}

/// Boundary-aware "does `path` fall under `entry`" test shared by the
/// scope checks (ContractGate's out_of_scope, ImpactEvidenceGate's in_scope).
/// Glob entries go through the glob matcher; plain entries match as a
/// path-segment prefix or a whole segment — never as a raw substring, so
/// `src/legacy` matches `src/legacy/x.rs` but not `src/legacyx.rs`, and
/// `src/x` does not match `tests/src/xylophone.rs`.
pub fn path_under_entry(entry: &str, path: &str) -> bool {
    if entry.contains('*') || entry.contains('?') {
        return path_matches_glob(entry, path);
    }
    let np = normalize_rel_path(path);
    let ne = normalize_rel_path(entry);
    let np = np.trim_start_matches('/');
    let ne = ne.trim_start_matches('/');
    if ne.is_empty() {
        return false;
    }
    if np == ne || np.starts_with(&format!("{ne}/")) {
        return true;
    }
    // 绝对路径 / 嵌套根：按段后缀对齐再比一次（与 path_matches_glob 同思路）。
    if ne.contains('/') {
        let segs: Vec<&str> = np.split('/').filter(|s| !s.is_empty()).collect();
        for start in 0..segs.len() {
            let suffix = segs[start..].join("/");
            if suffix == ne || suffix.starts_with(&format!("{ne}/")) {
                return true;
            }
        }
        false
    } else {
        np.split('/').any(|seg| seg == ne)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_lexical() {
        assert_eq!(normalize_rel_path("./src/ui/App.tsx"), "src/ui/App.tsx");
        assert_eq!(normalize_rel_path("src/ui/../ui/App.tsx"), "src/ui/App.tsx");
        assert_eq!(normalize_rel_path("src\\ui\\App.tsx"), "src/ui/App.tsx");
        assert_eq!(normalize_rel_path("src//ui///App.tsx"), "src/ui/App.tsx");
        assert_eq!(
            normalize_rel_path("/home/u/proj/src/ui/App.tsx"),
            "/home/u/proj/src/ui/App.tsx"
        );
    }

    #[test]
    fn preserve_glob_closes_the_known_bypasses() {
        // Every one of these used to slip past a hard Preserve.
        assert!(path_matches_glob("src/ui/**", "src/ui/App.tsx"));
        assert!(path_matches_glob("src/ui/**", "./src/ui/App.tsx"));
        assert!(path_matches_glob("src/ui/**", "src/ui/../ui/App.tsx"));
        assert!(path_matches_glob("src/ui/**", "src\\ui\\App.tsx"));
        assert!(path_matches_glob(
            "src/ui/**",
            "/home/u/proj/src/ui/App.tsx"
        ));
        assert!(path_matches_glob("src/ui/**", "src/ui/nested/deep/X.tsx"));
        assert!(path_matches_glob("**/*.test.ts", "src/a/b.test.ts"));
        assert!(path_matches_glob("src/*/index.ts", "src/foo/index.ts"));
    }

    #[test]
    fn path_under_entry_is_boundary_aware() {
        assert!(path_under_entry("src/feature", "src/feature/x.rs"));
        assert!(path_under_entry("src/feature", "./src/feature/x.rs"));
        assert!(path_under_entry("src/feature", "/abs/proj/src/feature/x.rs"));
        assert!(path_under_entry("tests", "src/app/tests/unit.rs"));
        assert!(!path_under_entry("src/feature", "src/featurex/x.rs"));
        assert!(!path_under_entry("src/x", "tests/src/xylophone.rs"));
        assert!(path_under_entry("src/ui/**", "src/ui/App.tsx"));
    }

    #[test]
    fn preserve_glob_does_not_over_broaden_on_siblings() {
        assert!(!path_matches_glob("src/ui/**", "src/ux/App.tsx"));
        assert!(!path_matches_glob("src/ui/**", "src/uikit/App.tsx"));
        // A single `*` must not cross a separator.
        assert!(!path_matches_glob("src/*/index.ts", "src/foo/bar/index.ts"));
    }
}
