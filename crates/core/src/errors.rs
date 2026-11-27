//! Errors a shell can tell apart: a stable `code` and `params` next to the text. The text (`Display`) stays
//! what it was, so the tray and the console print the same lines; `classify` reads the code, never the text.
//! The codes are the public contract of the JSON output (`docs/cli-json.md`).

use serde_json::{Value, json};
use std::fmt;

pub const BUSY: &str = "busy";
pub const GAME_RUNNING: &str = "game_running";
pub const NOT_FOUND: &str = "not_found";
pub const INVALID_ARGS: &str = "invalid_args";
pub const CONFIRMATION_REQUIRED: &str = "confirmation_required";
pub const NO_WOW: &str = "no_wow";
pub const CONFIG_INVALID: &str = "config_invalid";
pub const CONFIG_EXISTS: &str = "config_exists";
pub const IO: &str = "io";
pub const NETWORK: &str = "network";
pub const PARTIAL: &str = "partial";
pub const JSON_UNSUPPORTED: &str = "json_unsupported";
pub const FAILED: &str = "failed";

/// An error with a code. Raise it as the error itself (`Err(Coded::..().into())`) or as the context of
/// another one (`.with_context(|| Coded::..)`): the text is the message either way.
#[derive(Debug, Clone)]
pub struct Coded {
    pub code: &'static str,
    pub params: Value,
    message: String,
}

impl Coded {
    pub fn new(code: &'static str, params: Value, message: impl Into<String>) -> Self {
        Coded {
            code,
            params,
            message: message.into(),
        }
    }

    /// Another process holds the folder's lock; `holder` is its `.info` line, `<pid> <operation>`.
    pub fn busy(flavor: &str, holder: &str, message: impl Into<String>) -> Self {
        let (pid, operation) = match holder.split_once(' ') {
            Some((pid, op)) => (pid.parse::<u32>().ok(), Some(op)),
            None => (None, None),
        };
        Self::new(BUSY, json!({ "flavor": flavor, "pid": pid, "operation": operation }), message)
    }

    pub fn game_running(game: &str, flavor: Option<&str>, message: impl Into<String>) -> Self {
        Self::new(GAME_RUNNING, json!({ "game": game, "flavor": flavor }), message)
    }

    /// `what`: `snapshot`, `character`, `profile` or `flavor`.
    pub fn not_found(what: &str, key: &str, message: impl Into<String>) -> Self {
        Self::new(NOT_FOUND, json!({ "what": what, "key": key }), message)
    }

    pub fn no_wow(message: impl Into<String>) -> Self {
        Self::new(NO_WOW, json!({}), message)
    }

    pub fn config_invalid(path: &std::path::Path, message: impl Into<String>) -> Self {
        Self::new(CONFIG_INVALID, json!({ "path": path.display().to_string() }), message)
    }

    /// `kind`: the name of clap's `ErrorKind` (`UnknownArgument`), given as `unknown_argument`.
    pub fn invalid_args(kind: &str, message: impl Into<String>) -> Self {
        Self::new(INVALID_ARGS, json!({ "kind": snake(kind) }), message)
    }

    /// A setting value `config set` refuses: `reason` is `type`, `range` or `format`.
    pub fn invalid_value(key: &str, reason: &str, message: impl Into<String>) -> Self {
        Self::new(INVALID_ARGS, json!({ "kind": "invalid_value", "key": key, "reason": reason }), message)
    }

    pub fn confirmation_required(message: impl Into<String>) -> Self {
        Self::new(CONFIRMATION_REQUIRED, json!({}), message)
    }

    pub fn json_unsupported(command: &str, message: impl Into<String>) -> Self {
        Self::new(JSON_UNSUPPORTED, json!({ "command": command }), message)
    }

    /// A run that did part of its work: the rest is in the `data` next to the error.
    pub fn partial(message: impl Into<String>) -> Self {
        Self::new(PARTIAL, json!({}), message)
    }

    pub fn config_exists(path: &std::path::Path, message: impl Into<String>) -> Self {
        Self::new(CONFIG_EXISTS, json!({ "path": path.display().to_string() }), message)
    }
}

impl fmt::Display for Coded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Coded {}

/// `NotFound` -> `not_found`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// The code and params of an error: the first `Coded` in it, else an I/O error (`kind`, `os_code`), else `failed`.
pub fn classify(err: &anyhow::Error) -> (&'static str, Value) {
    if let Some(c) = err.downcast_ref::<Coded>() {
        return (c.code, c.params.clone());
    }
    if let Some(e) = err.chain().find_map(|e| e.downcast_ref::<std::io::Error>()) {
        return (IO, json!({ "kind": snake(&format!("{:?}", e.kind())), "os_code": e.raw_os_error() }));
    }
    (FAILED, json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn codes_do_not_depend_on_the_language() {
        // Tests run in Russian: the classifier must not read the text.
        let busy: anyhow::Error = Coded::busy("_retail_", "42 backup", tr!("app.busy", flavor = "Retail", what = "42 backup")).into();
        assert_eq!(classify(&busy), (BUSY, json!({ "flavor": "_retail_", "pid": 42, "operation": "backup" })));
        let unknown: anyhow::Error = Coded::busy("_retail_", "?", "x").into();
        assert_eq!(classify(&unknown).1, json!({ "flavor": "_retail_", "pid": null, "operation": null }));
        let game: anyhow::Error = Coded::game_running("Wow.exe", Some("_retail_"), tr!("app.close_game", game = "Wow.exe", flavor = "Retail")).into();
        assert_eq!(classify(&game), (GAME_RUNNING, json!({ "game": "Wow.exe", "flavor": "_retail_" })));
        let missing: anyhow::Error = Coded::not_found("character", "A/B/C", tr!("chars.not_found", spec = "A/B/C")).into();
        assert_eq!(classify(&missing), (NOT_FOUND, json!({ "what": "character", "key": "A/B/C" })));
    }

    #[test]
    fn a_code_survives_context_and_other_errors_are_io_or_failed() {
        let wrapped = Err::<(), _>(Coded::no_wow("x")).context("one").context("two").unwrap_err();
        assert_eq!(classify(&wrapped).0, NO_WOW);
        // As a context over another error: the text keeps both, the code is the context's.
        let over = Err::<(), _>(std::io::Error::other("cause"))
            .with_context(|| Coded::config_invalid(std::path::Path::new("c.toml"), "bad config"))
            .unwrap_err();
        assert_eq!(classify(&over), (CONFIG_INVALID, json!({ "path": "c.toml" })));
        assert_eq!(format!("{over:#}"), "bad config: cause");
        let io: anyhow::Error = std::io::Error::from_raw_os_error(5).into();
        assert_eq!(classify(&io.context("writing")), (IO, json!({ "kind": "permission_denied", "os_code": 5 })));
        assert_eq!(classify(&anyhow::anyhow!("plain")), (FAILED, json!({})));
    }
}
