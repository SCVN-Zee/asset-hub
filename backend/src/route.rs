//! The route table. It matches the raw, still-encoded path, as `new URL(req.url).pathname` does,
//! so `%2F` in an owner or repo name never matches.

use crate::validate::{is_cache_segment, is_repo_segment};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Other,
}

/// A route under `/v1/repos/:owner/:repo`. Every one runs behind authentication and repo access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoRoute {
    PostEvents,
    GetPopular,
    GetObject(String),
    PutObject(String),
    ListThumbs,
    GetThumbs,
    PutThumbs,
}

/// A request that matched a repo route.
#[derive(Debug, PartialEq, Eq)]
pub struct Matched {
    /// The owner and repo as written in the path. The caller lower-cases them.
    pub owner: String,
    pub repo: String,
    pub route: RepoRoute,
}

/// Splits `/v1/repos/<owner>/<repo>/<rest>` into the owner, the repo and the rest (with its slash).
fn split_repo_path(path: &str) -> Option<(&str, &str, &str)> {
    let rest = path.strip_prefix("/v1/repos/")?;
    let (owner, rest) = rest.split_once('/')?;
    let slash = rest.find('/')?;
    let (repo, tail) = rest.split_at(slash);
    (is_repo_segment(owner) && is_repo_segment(repo)).then_some((owner, repo, tail))
}

/// The seven routes, in a fixed order. `None` when the method or the path matches none of them.
pub fn match_repo_route(method: Method, path: &str) -> Option<Matched> {
    let (owner, repo, rest) = split_repo_path(path)?;
    let cache_name = rest.strip_prefix("/cache/").filter(|name| is_cache_segment(name));
    let route = match (method, rest) {
        (Method::Post, "/events") => RepoRoute::PostEvents,
        (Method::Get, "/popular") => RepoRoute::GetPopular,
        (Method::Get, _) if cache_name.is_some() => RepoRoute::GetObject(cache_name?.to_owned()),
        (Method::Put, _) if cache_name.is_some() => RepoRoute::PutObject(cache_name?.to_owned()),
        (Method::Get, "/thumbs") => RepoRoute::ListThumbs,
        (Method::Post, "/thumbs/get") => RepoRoute::GetThumbs,
        (Method::Post, "/thumbs/put") => RepoRoute::PutThumbs,
        _ => return None,
    };
    Some(Matched { owner: owner.to_owned(), repo: repo.to_owned(), route })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(method: Method, rest: &str) -> Option<RepoRoute> {
        match_repo_route(method, &format!("/v1/repos/supercent-io/game{rest}")).map(|m| m.route)
    }

    #[test]
    fn all_seven_routes_match() {
        assert_eq!(route(Method::Post, "/events"), Some(RepoRoute::PostEvents));
        assert_eq!(route(Method::Get, "/popular"), Some(RepoRoute::GetPopular));
        assert_eq!(route(Method::Get, "/cache/catalog"), Some(RepoRoute::GetObject("catalog".into())));
        assert_eq!(route(Method::Put, "/cache/facts-tmpfont"), Some(RepoRoute::PutObject("facts-tmpfont".into())));
        assert_eq!(route(Method::Get, "/thumbs"), Some(RepoRoute::ListThumbs));
        assert_eq!(route(Method::Post, "/thumbs/get"), Some(RepoRoute::GetThumbs));
        assert_eq!(route(Method::Post, "/thumbs/put"), Some(RepoRoute::PutThumbs));
    }

    #[test]
    fn owner_and_repo_are_captured_as_written() {
        let matched = match_repo_route(Method::Get, "/v1/repos/SuperCent-IO/Hb.Game_2/popular");
        assert_eq!(
            matched,
            Some(Matched { owner: "SuperCent-IO".into(), repo: "Hb.Game_2".into(), route: RepoRoute::GetPopular })
        );
    }

    #[test]
    fn a_wrong_method_does_not_match() {
        assert_eq!(route(Method::Get, "/events"), None);
        assert_eq!(route(Method::Other, "/cache/catalog"), None);
        assert_eq!(route(Method::Post, "/popular"), None);
        assert_eq!(route(Method::Put, "/thumbs/put"), None);
        assert_eq!(route(Method::Get, "/thumbs/get"), None);
    }

    #[test]
    fn a_trailing_slash_or_upper_case_does_not_match() {
        assert_eq!(route(Method::Get, "/popular/"), None);
        assert_eq!(route(Method::Get, "/Popular"), None);
        assert_eq!(route(Method::Post, "/events/"), None);
        assert_eq!(match_repo_route(Method::Get, "/V1/repos/o/r/popular"), None);
    }

    #[test]
    fn an_encoded_slash_never_matches() {
        assert_eq!(match_repo_route(Method::Get, "/v1/repos/o%2Fx/r/popular"), None);
        assert_eq!(match_repo_route(Method::Get, "/v1/repos/o/r%2Fx/popular"), None);
        assert_eq!(route(Method::Get, "/cache/a%2Fb"), None);
    }

    #[test]
    fn a_cache_name_has_word_characters_and_dashes_only() {
        assert_eq!(route(Method::Get, "/cache/catalog.json"), None);
        assert_eq!(route(Method::Get, "/cache/"), None);
        assert_eq!(route(Method::Get, "/cache/a/b"), None);
        assert_eq!(route(Method::Get, "/cache/facts-other"), Some(RepoRoute::GetObject("facts-other".into())));
    }

    #[test]
    fn missing_pieces_do_not_match() {
        assert_eq!(match_repo_route(Method::Get, "/v1/repos/o/r"), None);
        assert_eq!(match_repo_route(Method::Get, "/v1/repos/o"), None);
        assert_eq!(match_repo_route(Method::Get, "/v1/repos//r/popular"), None);
        assert_eq!(match_repo_route(Method::Get, "/v1/health"), None);
    }
}
