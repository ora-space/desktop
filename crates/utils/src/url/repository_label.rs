//! Derives short, mutually distinguishable display labels for a set of canonical repository URLs.
//!
//! The labels are built from path segments compared literally. No segment is interpreted as an
//! owner, a project, a subgroup, or a platform literal such as `_git` or `scm`, because those
//! structures differ across hosts and guessing them picks the wrong segment on some of them.

/// The marker that stands for the segments a label skips between its distinguishing segment and
/// the repository name.
const ELISION: &str = "…";

/// Returns one display label per URL, in input order, so that no two labels collide.
///
/// Every URL is split into its authority (kept with any non-default port) followed by its path
/// segments. A URL whose last segment is unique in the set is labelled by that segment alone.
/// Inside a group sharing the last segment, segments are compared position by position from the
/// end, and the first position at which this URL's segment differs from every other member of the
/// group gives the label `segment/name`, or `segment/…/name` when positions were skipped. URLs of
/// different depths are aligned from the end; a member with no segment at a position counts as
/// different from one that has a segment there. When no single position tells a URL apart, its
/// label falls back to the whole URL without the scheme, which is unique whenever the inputs are.
/// If two labels of one name group still read the same, the whole group falls back that way.
///
/// Callers pass canonical URLs so that equivalent spellings already compare equal.
pub fn distinguishing_repository_labels(canonical_urls: &[&str]) -> Vec<String> {
    let split: Vec<Vec<&str>> = canonical_urls.iter().map(|url| segments(url)).collect();
    let mut labels: Vec<String> = split
        .iter()
        .zip(canonical_urls)
        .map(|(own, url)| label_one(own, url, &split))
        .collect();
    // Each label is chosen to differ from every rival at one position, yet two labels can still
    // read the same (`x/…/m` taken at different depths). A collision shown to the user defeats
    // the purpose, so the whole name group falls back to full URLs rather than only the pair.
    for index in 0..labels.len() {
        let collides =
            (0..labels.len()).any(|other| other != index && labels[other] == labels[index]);
        if collides {
            let name = split[index].last().copied();
            for (member, url) in canonical_urls.iter().enumerate() {
                if split[member].last().copied() == name {
                    labels[member] = without_scheme(url).to_owned();
                }
            }
        }
    }
    labels
}

/// Labels one URL against every other URL sharing its repository name.
fn label_one(own: &[&str], url: &str, split: &[Vec<&str>]) -> String {
    let Some(name) = own.last().copied().filter(|_| own.len() > 1) else {
        // A bare authority has no repository name to show, so only the full URL is honest.
        return without_scheme(url).to_owned();
    };
    let rivals: Vec<&Vec<&str>> = split
        .iter()
        .filter(|other| !std::ptr::eq(other.as_slice(), own) && other.last() == Some(&name))
        .collect();
    if rivals.is_empty() {
        return name.to_owned();
    }
    // Offset 1 is the shared name itself; the search starts one segment further left.
    (2..=own.len())
        .find_map(|offset| {
            let segment = own[own.len() - offset];
            rivals
                .iter()
                .all(|rival| segment_from_end(rival, offset) != Some(segment))
                .then(|| match offset {
                    2 => format!("{segment}/{name}"),
                    _ => format!("{segment}/{ELISION}/{name}"),
                })
        })
        .unwrap_or_else(|| without_scheme(url).to_owned())
}

/// Splits a URL into its authority and non-empty path segments, ignoring the scheme.
fn segments(url: &str) -> Vec<&str> {
    without_scheme(url)
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// Returns the segment `offset` positions from the end (1 is the last), absent past the start.
fn segment_from_end<'a>(segments: &[&'a str], offset: usize) -> Option<&'a str> {
    segments
        .len()
        .checked_sub(offset)
        .map(|index| segments[index])
}

/// Drops a leading `scheme://`, which every label omits because it never distinguishes sources.
fn without_scheme(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_scheme, rest)| rest)
}

#[cfg(test)]
mod tests {
    use super::distinguishing_repository_labels;
    use pretty_assertions::assert_eq;

    /// Shortens the call under test so each case reads as input and expected output.
    fn labels(urls: &[&str]) -> Vec<String> {
        distinguishing_repository_labels(urls)
    }

    /// A repository name no other source shares is shown alone.
    #[test]
    fn a_unique_name_is_the_whole_label() {
        assert_eq!(
            labels(&[
                "https://github.com/acme/awesome-plugins",
                "https://github.com/acme/tools",
            ]),
            ["awesome-plugins", "tools"],
        );
    }

    /// A fork shares the repository name and differs only in the segment right before it.
    #[test]
    fn forks_are_told_apart_by_the_adjacent_segment() {
        assert_eq!(
            labels(&[
                "https://github.com/acme/marketplace",
                "https://github.com/obsismc/marketplace",
            ]),
            ["acme/marketplace", "obsismc/marketplace"],
        );
    }

    /// The default marketplace's URL joins the comparison set, so its lone fork is told apart by
    /// the adjacent segment instead of showing the bare repository name (origin ADR D3 example).
    #[test]
    fn the_default_marketplace_disambiguates_its_lone_fork() {
        assert_eq!(
            labels(&[
                "https://github.com/ora-space/marketplace",
                "https://github.com/obsismc/marketplace",
            ]),
            ["ora-space/marketplace", "obsismc/marketplace"],
        );
    }

    /// Hosts differ while every path segment matches, so the label skips to the authority.
    #[test]
    fn the_same_path_on_two_hosts_shows_the_host() {
        assert_eq!(
            labels(&[
                "https://github.com/acme/tools",
                "https://gitlab.com/acme/tools",
            ]),
            ["github.com/…/tools", "gitlab.com/…/tools"],
        );
    }

    /// A non-default port is part of the authority and therefore of the label.
    #[test]
    fn a_self_hosted_port_is_kept_in_the_label() {
        assert_eq!(
            labels(&[
                "https://git.example.com:8443/scm/infra/registry",
                "https://git.example.com/scm/infra/registry",
            ]),
            [
                "git.example.com:8443/…/registry",
                "git.example.com/…/registry"
            ],
        );
    }

    /// Azure's `_git` literal is compared like any other segment, never skipped or interpreted.
    #[test]
    fn azure_projects_are_told_apart_without_parsing_git_literals() {
        assert_eq!(
            labels(&[
                "https://dev.azure.com/contoso/analytics/_git/registry",
                "https://dev.azure.com/contoso/platform/_git/registry",
                "https://dev.azure.com/fabrikam/registry",
            ]),
            [
                "analytics/…/registry",
                "platform/…/registry",
                "fabrikam/registry"
            ],
        );
        // When `_git` is the first differing segment it is shown verbatim.
        assert_eq!(
            labels(&[
                "https://dev.azure.com/contoso/tools/_git/registry",
                "https://github.com/contoso/tools/registry",
            ]),
            ["_git/registry", "tools/registry"],
        );
    }

    /// GitLab subgroups nest to different depths; the shallower path has no segment there.
    #[test]
    fn gitlab_subgroups_of_different_depths_are_aligned_from_the_end() {
        assert_eq!(
            labels(&[
                "https://gitlab.com/acme/platform/tools/registry",
                "https://gitlab.com/acme/registry",
            ]),
            ["tools/registry", "acme/registry"],
        );
    }

    /// A rival that runs out of segments at a position counts as different from one that has a
    /// segment there, so the deeper URL is told apart by the segment the shallower one lacks.
    #[test]
    fn a_rival_missing_a_segment_is_different_from_one_that_has_it() {
        assert_eq!(
            labels(&[
                "https://corp.example.com/gitlab/acme/tools",
                "https://gitlab/acme/tools",
            ]),
            ["corp.example.com/…/tools", "gitlab/acme/tools"],
        );
    }

    /// No single segment separates the first URL from both rivals, so it falls back to the URL.
    #[test]
    fn crossing_differences_fall_back_to_the_full_url() {
        assert_eq!(
            labels(&[
                "https://github.com/a/x/m",
                "https://github.com/a/y/m",
                "https://github.com/b/x/m",
            ]),
            ["github.com/a/x/m", "y/m", "b/…/m"],
        );
    }

    /// Two labels chosen at different depths can read the same; the whole group then shows URLs.
    #[test]
    fn colliding_labels_send_the_whole_group_to_full_urls() {
        assert_eq!(
            labels(&[
                "https://github.com/p/x/a/m",
                "https://github.com/x/b/a/m",
                "https://github.com/q/b/c/m",
                "https://github.com/acme/tools",
            ]),
            [
                "github.com/p/x/a/m",
                "github.com/x/b/a/m",
                "github.com/q/b/c/m",
                "tools",
            ],
        );
    }

    /// A URL without a path has no repository name, so only its authority can label it.
    #[test]
    fn a_bare_authority_is_labelled_by_itself() {
        assert_eq!(
            labels(&[
                "https://git.example.com:8443",
                "https://github.com/acme/tools"
            ]),
            ["git.example.com:8443", "tools"],
        );
    }
}
