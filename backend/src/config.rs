//! The bindings and secrets the Worker reads. Each is a plain string on `env`.

use js_sys::Reflect;
use wasm_bindgen::JsValue;
use worker::Env;

use crate::http::Halt;

pub struct Config {
    /// Comma-separated GitHub owners whose repos are served.
    pub allowed_owners: String,
    pub upstash_url: String,
    pub upstash_token: String,
}

/// The B2 bindings. They load only in the routes that talk to B2, so a missing B2 secret breaks the
/// cache routes and nothing else.
pub struct B2Config {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub key_id: String,
    pub app_key: String,
}

fn text(env: &Env, name: &str) -> Result<String, Halt> {
    let value = Reflect::get(env.as_ref(), &JsValue::from_str(name))?;
    // Only the name of the missing binding is logged, never a value.
    value.as_string().ok_or_else(|| Halt::failed(format!("missing binding {name}")))
}

impl Config {
    pub fn load(env: &Env) -> Result<Self, Halt> {
        Ok(Self {
            allowed_owners: text(env, "ALLOWED_OWNERS")?,
            upstash_url: text(env, "UPSTASH_REDIS_REST_URL")?,
            upstash_token: text(env, "UPSTASH_REDIS_REST_TOKEN")?,
        })
    }
}

impl B2Config {
    pub fn load(env: &Env) -> Result<Self, Halt> {
        Ok(Self {
            endpoint: text(env, "B2_ENDPOINT")?,
            region: text(env, "B2_REGION")?,
            bucket: text(env, "B2_BUCKET")?,
            key_id: text(env, "B2_KEY_ID")?,
            app_key: text(env, "B2_APP_KEY")?,
        })
    }
}
