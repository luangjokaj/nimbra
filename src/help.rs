//! The Help menu's links: reporting a problem on GitHub and this version's release notes.

/// Where a problem is reported: a new issue on the app's repository.
const NEW_ISSUE: &str = "https://github.com/nimbra-mail/desktop/issues/new";

/// A new issue, its body set out for the report and ending with the versions. Nothing else
/// about the Mac or the mail goes in, and nothing is sent until the user submits it.
pub fn report_problem_url(nimbra: &str, macos: &str) -> String {
    let macos = if macos.is_empty() { "unknown" } else { macos };
    let body = format!(
        "**What happened?**\n\n\n**What did you expect?**\n\n\n**Steps to reproduce**\n\n1. \n\n\
         ---\nNimbra {nimbra} · macOS {macos}\n"
    );
    let mut url = url::Url::parse(NEW_ISSUE).expect("a valid address");
    url.query_pairs_mut().append_pair("body", &body);
    url.into()
}

/// Help › Report a Problem…: opens a new issue in the browser.
pub fn report_problem(cx: &mut gpui::App) {
    let url = report_problem_url(env!("CARGO_PKG_VERSION"), &crate::usage::macos_version());
    cx.open_url(&url);
}

/// Help › Release Notes: this version's, on the website.
pub fn release_notes(cx: &mut gpui::App) {
    cx.open_url(&crate::update::release_notes(env!("CARGO_PKG_VERSION")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_opens_a_new_issue_with_the_versions() {
        let url = url::Url::parse(&report_problem_url("0.1.8", "26")).unwrap();
        assert_eq!(url.host_str(), Some("github.com"));
        assert_eq!(url.path(), "/nimbra-mail/desktop/issues/new");
        let body = url
            .query_pairs()
            .find(|(k, _)| k == "body")
            .map(|(_, v)| v.into_owned())
            .unwrap();
        assert!(body.starts_with("**What happened?**"));
        assert!(body.ends_with("Nimbra 0.1.8 · macOS 26\n"));
    }

    #[test]
    fn an_unread_macos_version_says_so() {
        let url = url::Url::parse(&report_problem_url("0.1.8", "")).unwrap();
        let body: String = url.query_pairs().map(|(_, v)| v.into_owned()).collect();
        assert!(body.contains("macOS unknown"));
    }
}
