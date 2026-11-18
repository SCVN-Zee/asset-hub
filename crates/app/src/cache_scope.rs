//! Which orgs, repos and branches the background catalog index caches.
//!
//! The settings store what is left out; everything else is cached, new repos and branches
//! included. Every place that scans, pulls, merges or lists asks the predicates here.

use std::sync::{Arc, Mutex, RwLock};

use tauri::async_runtime::JoinHandle;

use crate::dto::CacheExclusions;

/// The owner of `repo` ("owner/name").
pub fn org_of(repo: &str) -> &str {
    repo.split('/').next().unwrap_or(repo)
}

/// `branch` when `entry` ("owner/name@branch") is one of `repo`.
fn branch_of<'a>(entry: &'a str, repo: &str) -> Option<&'a str> {
    entry.strip_prefix(repo)?.strip_prefix('@')
}

impl CacheExclusions {
    pub fn is_empty(&self) -> bool {
        self.orgs.is_empty() && self.repos.is_empty() && self.branches.is_empty()
    }

    /// The repo, or its org, is left out.
    pub fn repo_excluded(&self, repo: &str) -> bool {
        let org = org_of(repo);
        self.orgs.iter().any(|o| o == org) || self.repos.iter().any(|r| r == repo)
    }

    /// The branch, or its repo, is left out.
    pub fn branch_excluded(&self, repo: &str, branch: &str) -> bool {
        self.repo_excluded(repo) || self.branches.iter().any(|entry| branch_of(entry, repo) == Some(branch))
    }

    /// The branches of `repo` that are left out one by one.
    pub fn excluded_branches_of(&self, repo: &str) -> Vec<String> {
        self.branches.iter().filter_map(|entry| branch_of(entry, repo)).map(str::to_owned).collect()
    }

    /// True when some exclusion of `repo` (its repo or branch entries) is not in `now`: the repo
    /// may hold something that was left out before and is cached now.
    pub fn loosened_for(&self, now: &CacheExclusions, repo: &str) -> bool {
        (self.repo_excluded(repo) && !now.repo_excluded(repo)) || self.excluded_branches_of(repo).iter().any(|b| !now.branch_excluded(repo, b))
    }
}

/// The exclusions in force, shared by the settings command and the catalog.
#[derive(Clone, Default)]
pub struct SharedExclusions(Arc<RwLock<CacheExclusions>>);

impl SharedExclusions {
    pub fn get(&self) -> CacheExclusions {
        self.0.read().map(|e| e.clone()).unwrap_or_default()
    }

    pub fn set(&self, exclusions: CacheExclusions) {
        if let Ok(mut held) = self.0.write() {
            *held = exclusions;
        }
    }
}

/// The scan that follows a change of the exclusions: a newer change replaces one still waiting.
#[derive(Default)]
pub struct PendingScan {
    /// The exclusions before the oldest change not scanned yet.
    baseline: Mutex<Option<CacheExclusions>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl PendingScan {
    /// Stops the scan still waiting and answers the exclusions the new one compares against:
    /// those before the oldest change it covers.
    pub fn supersede(&self, before: CacheExclusions) -> CacheExclusions {
        if let Some(task) = self.task.lock().ok().and_then(|mut t| t.take()) {
            task.abort();
        }
        let Ok(mut baseline) = self.baseline.lock() else { return before };
        let base = baseline.take().unwrap_or(before);
        *baseline = Some(base.clone());
        base
    }

    pub fn started(&self, task: JoinHandle<()>) {
        if let Ok(mut held) = self.task.lock() {
            *held = Some(task);
        }
    }

    /// The scan has run to its end.
    pub fn finished(&self) {
        if let Ok(mut baseline) = self.baseline.lock() {
            *baseline = None;
        }
        if let Ok(mut task) = self.task.lock() {
            *task = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some() -> CacheExclusions {
        CacheExclusions { orgs: vec!["acme".into()], repos: vec!["lab/old".into()], branches: vec!["lab/new@wip".into(), "lab/new@a@b".into()] }
    }

    #[test]
    fn an_org_or_a_repo_leaves_its_repos_out() {
        let e = some();
        assert!(e.repo_excluded("acme/any") && e.repo_excluded("lab/old"));
        assert!(!e.repo_excluded("lab/new") && !e.repo_excluded("acmeco/x"));
    }

    #[test]
    fn a_branch_is_left_out_with_its_repo_or_by_name() {
        let e = some();
        assert!(e.branch_excluded("acme/any", "main") && e.branch_excluded("lab/old", "main"));
        assert!(e.branch_excluded("lab/new", "wip") && e.branch_excluded("lab/new", "a@b"));
        assert!(!e.branch_excluded("lab/new", "main") && !e.branch_excluded("lab/newer", "wip"));
        assert_eq!(e.excluded_branches_of("lab/new"), vec!["wip".to_owned(), "a@b".to_owned()]);
    }

    #[test]
    fn nothing_is_left_out_by_default() {
        let e = CacheExclusions::default();
        assert!(e.is_empty() && !e.repo_excluded("a/b") && !e.branch_excluded("a/b", "main"));
    }

    #[test]
    fn a_repo_is_loosened_when_an_exclusion_of_it_is_lifted() {
        let before = some();
        let lifted_repo = CacheExclusions { repos: vec![], ..some() };
        let lifted_branch = CacheExclusions { branches: vec!["lab/new@a@b".into()], ..some() };
        assert!(before.loosened_for(&lifted_repo, "lab/old"));
        assert!(before.loosened_for(&lifted_branch, "lab/new"));
        assert!(!before.loosened_for(&lifted_branch, "lab/old"));
        assert!(!CacheExclusions::default().loosened_for(&before, "lab/new"));
    }

    #[test]
    fn the_shared_exclusions_follow_the_last_set() {
        let shared = SharedExclusions::default();
        assert!(shared.get().is_empty());
        shared.set(some());
        assert_eq!(shared.get(), some());
    }

    #[tokio::test]
    async fn a_newer_change_replaces_a_scan_that_is_still_waiting() {
        let pending = PendingScan::default();
        let first = pending.supersede(CacheExclusions::default());
        assert!(first.is_empty());
        let waiting = tauri::async_runtime::spawn(std::future::pending::<()>());
        pending.started(waiting);
        // The second change compares against what was in force before the first.
        let second = pending.supersede(some());
        assert!(second.is_empty());
        pending.finished();
        assert_eq!(pending.supersede(some()), some());
    }
}
