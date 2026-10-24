//! `diffy://` links and handing them to the running window.
//!
//! A link is a GitHub pull request URL with `diffy://` in place of
//! `https://`: `diffy://github.com/owner/repo/pull/123`. Plain `https://`
//! pull request URLs are accepted too, so `diffy <url>` works from a shell.
//!
//! macOS delivers clicks to the running app as an Apple Event. Linux and
//! Windows launch a new process with the link as its argument; that process
//! forwards it to the first instance and exits.

use quark_app::platform::single_instance::{self, Instance, PrimaryInstance};

use crate::core::forge::github::pull_request::parse_pr_url;

/// The schemes a link may use: the release app's, and the dev bundle's so
/// both can be installed side by side.
const SCHEMES: [&str; 2] = ["diffy://", "diffy-dev://"];
const INSTANCE_ID: &str = "io.github.seatedro.diffy";

/// The canonical `https://` URL of the pull request `arg` links to.
pub fn pull_request_url(arg: &str) -> Option<String> {
    let arg = arg.trim();
    let rest = SCHEMES
        .iter()
        .find_map(|scheme| strip_prefix_ignore_case(arg, scheme))
        .unwrap_or(arg);
    parse_pr_url(rest).map(|pr| pr.url())
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

pub enum Launch {
    /// This process runs the app; it hears later launches' links through
    /// the instance, when it could claim one.
    Run(Option<PrimaryInstance>),
    /// The running app took the link; this process should exit.
    Forwarded,
}

/// Claims the first-instance socket, or forwards `link` to the app that
/// holds it. A launch without a link always runs, so opening a second
/// window from a shell keeps working.
pub fn claim(link: Option<&str>) -> Launch {
    let args: Vec<String> = link.map(str::to_owned).into_iter().collect();
    match single_instance::acquire(&instance_id(), &args) {
        Ok(Instance::Primary(primary)) => Launch::Run(Some(primary)),
        Ok(Instance::Secondary) if link.is_some() => Launch::Forwarded,
        Ok(Instance::Secondary) => Launch::Run(None),
        Err(error) => {
            tracing::warn!("could not claim the single instance socket: {error}");
            Launch::Run(None)
        }
    }
}

/// Apps with their own config dir (the dev bundle, parallel test bundles)
/// get their own socket, so links never reach another build.
fn instance_id() -> String {
    use std::hash::{Hash, Hasher};
    let Some(dir) = super::startup::app_config_dir() else {
        return INSTANCE_ID.to_owned();
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    dir.hash(&mut hasher);
    format!("{INSTANCE_ID}.{:x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_the_pull_request() {
        let url = Some("https://github.com/seatedro/diffy/pull/40".to_owned());
        for link in [
            "diffy://github.com/seatedro/diffy/pull/40",
            "DIFFY://github.com/seatedro/diffy/pull/40/files",
            "diffy-dev://github.com/seatedro/diffy/pull/40",
            "https://github.com/seatedro/diffy/pull/40?w=1",
            " diffy://github.com/seatedro/diffy/pull/40/ ",
        ] {
            assert_eq!(pull_request_url(link), url, "{link}");
        }
        for other in [
            "diffy://github.com/seatedro/diffy/issues/40",
            "diffy://example.com/a/b/pull/1",
            "seatedro/diffy#40",
            "--repo",
            "/home/me/code/diffy",
        ] {
            assert_eq!(pull_request_url(other), None, "{other}");
        }
    }
}
