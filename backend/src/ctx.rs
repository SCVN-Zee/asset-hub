//! What a repo handler gets once the caller has proven read access to `owner/repo`.

use worker::{Context, D1Database, Env, Request};

use crate::auth::Caller;
use crate::config::Config;

pub struct RepoCtx<'a> {
    pub req: &'a Request,
    pub env: &'a Env,
    /// Work that may finish after the reply goes through `exec.wait_until`.
    pub exec: &'a Context,
    pub cfg: &'a Config,
    pub caller: &'a Caller,
    /// `owner/repo`, lower-case.
    pub slug: &'a str,
}

impl RepoCtx<'_> {
    pub fn db(&self) -> worker::Result<D1Database> {
        self.env.d1("DB")
    }
}
