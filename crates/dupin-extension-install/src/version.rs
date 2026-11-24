//! Which tag counts as the newest version of a repository.

/// The highest `1.2.3` or `v1.2.3` tag; a pre-release (`v2.0.0-beta.1`) ranks below its release.
pub(crate) fn newest_version(tags: &[String]) -> Option<&str> {
    tags.iter()
        .filter_map(|tag| version_key(tag).map(|key| (key, tag)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, tag)| tag.as_str())
}

/// One dot-separated pre-release identifier, ordered as SemVer orders them: numbers by value,
/// and below any name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PreRelease {
    Number(u64),
    Name(String),
}

fn version_key(tag: &str) -> Option<(Vec<u64>, bool, Vec<PreRelease>)> {
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let mut parts = core
        .split('.')
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        })
        .collect::<Option<Vec<u64>>>()?;
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    parts.resize(4, 0);
    let release = pre.is_none();
    let pre = pre
        .unwrap_or_default()
        .split('.')
        .filter(|part| !part.is_empty())
        .map(|part| match part.parse() {
            Ok(number) if part.bytes().all(|byte| byte.is_ascii_digit()) => PreRelease::Number(number),
            _ => PreRelease::Name(part.to_owned()),
        })
        .collect();
    Some((parts, release, pre))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_version_tag_wins_over_names_and_pre_releases() {
        let tags: Vec<String> = [
            "v1.2.0",
            "v1.10.0",
            "latest",
            "v2.0.0-beta.1",
            "1.9",
            "v1.10.0-rc.1",
        ]
        .map(str::to_owned)
        .into();
        assert_eq!(newest_version(&tags), Some("v2.0.0-beta.1"));
        let tags: Vec<String> = ["v1.2.0", "v1.10.0", "nightly"].map(str::to_owned).into();
        assert_eq!(newest_version(&tags), Some("v1.10.0"));
        let tags: Vec<String> = ["v1.10.0-rc.1", "v1.10.0"].map(str::to_owned).into();
        assert_eq!(newest_version(&tags), Some("v1.10.0"));
        assert_eq!(newest_version(&["nightly".to_owned()]), None);
        let tags: Vec<String> = ["v1.0.0-rc.9", "v1.0.0-rc.10", "v1.0.0-rc"]
            .map(str::to_owned)
            .into();
        assert_eq!(newest_version(&tags), Some("v1.0.0-rc.10"));
        let tags: Vec<String> = ["v1.0.0-alpha.beta", "v1.0.0-alpha.1"].map(str::to_owned).into();
        assert_eq!(newest_version(&tags), Some("v1.0.0-alpha.beta"));
    }
}
