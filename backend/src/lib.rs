//! The team Worker: shared thumbnail and cache storage plus popularity counts.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod auth;
mod b2;
mod config;
mod ctx;
mod events;
mod hex;
mod http;
mod js;
mod kinds;
mod objects;
mod popular;
mod redis;
mod route;
mod shards;
mod sigv4;
mod storage;
mod sweep;
mod thumbs;
mod validate;

use worker::{console_error, event, Context, Env, Request, Result, ScheduleContext, ScheduledEvent};

use crate::auth::{assert_repo_access, authenticate};
use crate::config::Config;
use crate::ctx::RepoCtx;
use crate::http::{error_json, json, stop, Halt, Handled};
use crate::redis::Redis;
use crate::route::{match_repo_route, Method, RepoRoute};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The JSON `GET /v1/health` answers with.
fn health_body() -> String {
    format!(r#"{{"ok":true,"version":"{VERSION}"}}"#)
}

fn method_of(req: &Request) -> Method {
    match req.method() {
        worker::Method::Get => Method::Get,
        worker::Method::Post => Method::Post,
        worker::Method::Put => Method::Put,
        _ => Method::Other,
    }
}

/// Health, then a repo route behind `authenticate` and `assert_repo_access`, then 404.
async fn handle(req: &Request, env: &Env, exec: &Context) -> Handled<web_sys::Response> {
    let url = req.url()?;
    let method = method_of(req);

    if url.path() == "/v1/health" && method == Method::Get {
        return Ok(json(health_body(), 200, &[])?);
    }

    if let Some(matched) = match_repo_route(method, url.path()) {
        let cfg = Config::load(env)?;
        let redis = Redis::new(&cfg.upstash_url, &cfg.upstash_token);
        let mut caller = authenticate(req, &redis).await?;
        let owner = matched.owner.to_ascii_lowercase();
        let repo = matched.repo.to_ascii_lowercase();
        assert_repo_access(&mut caller, &owner, &repo, &cfg, &redis).await?;

        let slug = format!("{owner}/{repo}");
        let ctx = RepoCtx { req, env, exec, cfg: &cfg, caller: &caller, slug: &slug };
        return match matched.route {
            RepoRoute::PostEvents => events::post_events(&ctx).await,
            RepoRoute::GetPopular => popular::get_popular(&ctx).await,
            RepoRoute::GetObject(name) => objects::get_object(&ctx, &name).await,
            RepoRoute::PutObject(name) => objects::put_object(&ctx, &name).await,
            RepoRoute::ListThumbs => thumbs::list_thumbs(&ctx).await,
            RepoRoute::GetThumbs => thumbs::get_thumbs(&ctx).await,
            RepoRoute::PutThumbs => thumbs::put_thumbs(&ctx).await,
        };
    }

    Err(stop("notFound", 404))
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, exec: Context) -> Result<web_sys::Response> {
    match handle(&req, &env, &exec).await {
        Ok(res) | Err(Halt::Reply(res)) => Ok(res),
        Err(Halt::Failed(label)) => {
            // The label holds the operation and status of a B2 error, or an error name. It never
            // holds a key, a token or a signature.
            console_error!("internal {label}");
            error_json("internal", 500)
        }
    }
}

#[event(scheduled)]
async fn scheduled(event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    // The sweep takes three D1 queries and comes first, so a rebuild that runs out of time never
    // stops it. A failure of one job does not stop the other.
    let shard = sweep::shard_for_run(event.schedule());
    if let Err(err) = sweep::sweep_shard(&sweep::D1Sweep::new(&env), shard, js::now_ms()).await {
        console_error!("thumbs.sweep {}", err.label());
    }
    if let Err(Halt::Failed(label)) = popular::rebuild_popular(&env).await {
        console_error!("popular.rebuild {label}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_body_matches_the_contract() {
        assert_eq!(health_body(), r#"{"ok":true,"version":"0.1.0"}"#);
    }
}
