use crate::flakeref::{Download, GitForge, GitRepo, Indirect, MercurialRepo, PathRef, url::Url};

/// The kind of a flake reference, one variant per Nix input type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlakeRefType {
    /// `github`, `gitlab` and `sourcehut`.
    GitForge(GitForge),
    /// `git`.
    Git(GitRepo),
    /// `hg`.
    Mercurial(MercurialRepo),
    /// `tarball`: an archive downloaded and unpacked.
    Tarball(Download),
    /// `file`: a file downloaded as is.
    File(Download),
    /// `path`.
    Path(PathRef),
    /// `indirect`: a flake in the registry.
    Indirect(Indirect),
}

impl FlakeRefType {
    /// The Nix type name, the `type` attribute.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::GitForge(forge) => forge.platform.as_str(),
            Self::Git(_) => "git",
            Self::Mercurial(_) => "hg",
            Self::Tarball(_) => "tarball",
            Self::File(_) => "file",
            Self::Path(_) => "path",
            Self::Indirect(_) => "indirect",
        }
    }

    pub(crate) fn ref_(&self) -> Option<&str> {
        match self {
            Self::GitForge(GitForge { ref_, .. })
            | Self::Git(GitRepo { ref_, .. })
            | Self::Mercurial(MercurialRepo { ref_, .. })
            | Self::Indirect(Indirect { ref_, .. }) => ref_.as_deref(),
            Self::Tarball(_) | Self::File(_) | Self::Path(_) => None,
        }
    }

    pub(crate) fn rev(&self) -> Option<&str> {
        match self {
            Self::GitForge(GitForge { rev, .. })
            | Self::Git(GitRepo { rev, .. })
            | Self::Mercurial(MercurialRepo { rev, .. })
            | Self::Indirect(Indirect { rev, .. })
            | Self::Tarball(Download { rev, .. })
            | Self::File(Download { rev, .. })
            | Self::Path(PathRef { rev, .. }) => rev.as_deref(),
        }
    }

    /// The typed `ref_` slot, for the kinds that take a ref.
    pub(crate) fn ref_slot(&mut self) -> Option<&mut Option<String>> {
        match self {
            Self::GitForge(GitForge { ref_, .. })
            | Self::Git(GitRepo { ref_, .. })
            | Self::Mercurial(MercurialRepo { ref_, .. })
            | Self::Indirect(Indirect { ref_, .. }) => Some(ref_),
            Self::Tarball(_) | Self::File(_) | Self::Path(_) => None,
        }
    }

    pub(crate) fn rev_slot(&mut self) -> &mut Option<String> {
        match self {
            Self::GitForge(GitForge { rev, .. })
            | Self::Git(GitRepo { rev, .. })
            | Self::Mercurial(MercurialRepo { rev, .. })
            | Self::Indirect(Indirect { rev, .. })
            | Self::Tarball(Download { rev, .. })
            | Self::File(Download { rev, .. })
            | Self::Path(PathRef { rev, .. }) => rev,
        }
    }

    /// The `url` of a git repository, parsed.
    fn git_url(&self) -> Option<Url<'_>> {
        match self {
            Self::Git(git) => Url::parse(&git.url).ok(),
            _ => None,
        }
    }

    /// Owner and repository from the path of a git URL, `host/<owner>/<repo>[.git]`.
    fn git_owner_repo(&self) -> Option<(String, Option<String>)> {
        let url = self.git_url()?;
        let mut segments = url.non_empty_segments();
        let owner = segments.next()?.to_string();
        let repo = segments
            .next()
            .map(|r| r.strip_suffix(".git").unwrap_or(r).to_string());
        Some((owner, repo))
    }

    pub(crate) fn repo(&self) -> Option<String> {
        match self {
            Self::GitForge(GitForge { repo, .. }) => Some(repo.clone()),
            _ => self.git_owner_repo().and_then(|(_, repo)| repo),
        }
    }

    pub(crate) fn owner(&self) -> Option<String> {
        match self {
            Self::GitForge(GitForge { owner, .. }) => Some(owner.clone()),
            _ => self.git_owner_repo().map(|(owner, _)| owner),
        }
    }

    /// The host a forge or git repository is fetched from. For a git URL, a port is kept
    /// when it is not the scheme's default.
    pub(crate) fn domain(&self) -> Option<String> {
        if let Self::GitForge(forge) = self {
            return Some(
                forge
                    .host
                    .clone()
                    .unwrap_or_else(|| forge.platform.default_host().to_string()),
            );
        }
        let url = self.git_url()?;
        let authority = url.authority.as_ref()?;
        if !authority.has_host() {
            return None;
        }
        let host = authority.host_str();
        let default_port = match url.scheme {
            "https" => Some(443),
            "http" => Some(80),
            "ssh" => Some(22),
            _ => None,
        };
        Some(match authority.port {
            Some(port) if Some(port) != default_port => format!("{host}:{port}"),
            _ => host,
        })
    }
}

#[cfg(test)]
mod resource_type_methods {
    use crate::{Context, FlakeRef};
    use rstest::rstest;

    #[rstest]
    #[case("git+https://github.com/owner/repo", "github.com", "owner", "repo")]
    #[case(
        "git+https://git.clan.lol/kenji/test-release",
        "git.clan.lol",
        "kenji",
        "test-release"
    )]
    #[case(
        "git+https://codeberg.org/forgejo/forgejo",
        "codeberg.org",
        "forgejo",
        "forgejo"
    )]
    #[case("git+https://gitlab.com/user/project", "gitlab.com", "user", "project")]
    #[case("git+http://example.com/org/myrepo", "example.com", "org", "myrepo")]
    fn test_resource_git_url_extraction(
        #[case] url: &str,
        #[case] expected_domain: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(
            parsed.domain(),
            Some(expected_domain.to_string()),
            "Domain mismatch for {}",
            url
        );
        assert_eq!(
            parsed.owner(),
            Some(expected_owner.to_string()),
            "Owner mismatch for {}",
            url
        );
        assert_eq!(
            parsed.repo(),
            Some(expected_repo.to_string()),
            "Repo mismatch for {}",
            url
        );
    }

    #[rstest]
    #[case("git+https://github.com/owner/repo.git", "repo")]
    #[case("git+https://git.clan.lol/kenji/test-release.git", "test-release")]
    fn test_resource_git_url_with_git_suffix(#[case] url: &str, #[case] expected_repo: &str) {
        let parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(
            parsed.repo(),
            Some(expected_repo.to_string()),
            ".git suffix should be stripped"
        );
    }

    #[rstest]
    #[case("github:nixos/nixpkgs", "github.com", "nixos", "nixpkgs")]
    #[case("gitlab:owner/repo", "gitlab.com", "owner", "repo")]
    #[case("sourcehut:user/project", "git.sr.ht", "user", "project")]
    fn test_gitforge_domain_extraction(
        #[case] url: &str,
        #[case] expected_domain: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let parsed: FlakeRef = url.parse().unwrap();

        assert_eq!(
            parsed.domain(),
            Some(expected_domain.to_string()),
            "Domain mismatch for {}",
            url
        );
        assert_eq!(
            parsed.owner(),
            Some(expected_owner.to_string()),
            "Owner mismatch for {}",
            url
        );
        assert_eq!(
            parsed.repo(),
            Some(expected_repo.to_string()),
            "Repo mismatch for {}",
            url
        );
    }

    #[rstest]
    #[case("path:/foo/bar")]
    #[case("/foo/bar")]
    #[case("./relative/path")]
    #[case("flake:nixpkgs")]
    #[case("https://example.com/file.tar.gz")]
    fn test_non_git_resource_returns_none(#[case] url: &str) {
        let parsed = FlakeRef::parse(url, Context::FlakeInput).unwrap();

        assert_eq!(
            parsed.domain(),
            None,
            "Non-git resources should return None for domain"
        );
        assert_eq!(
            parsed.owner(),
            None,
            "Non-git resources should return None for owner"
        );
        assert_eq!(
            parsed.repo(),
            None,
            "Non-git resources should return None for repo"
        );
    }

    #[rstest]
    #[case(
        "git+https://example.com/a/b",
        Some("example.com"),
        Some("a"),
        Some("b")
    )]
    #[case("git+https://x.y.z/org/repo", Some("x.y.z"), Some("org"), Some("repo"))]
    #[case("git+https://host/o/r.git", Some("host"), Some("o"), Some("r"))]
    fn test_resource_url_minimal_parsing(
        #[case] url: &str,
        #[case] expected_domain: Option<&str>,
        #[case] expected_owner: Option<&str>,
        #[case] expected_repo: Option<&str>,
    ) {
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(parsed.domain().as_deref(), expected_domain);
        assert_eq!(parsed.owner().as_deref(), expected_owner);
        assert_eq!(parsed.repo().as_deref(), expected_repo);
    }

    #[rstest]
    #[case("git+https://domain.com/owner")] // Missing repo.
    #[case("git+https://domain.com")] // Missing owner and repo.
    fn test_resource_url_insufficient_components_returns_none(#[case] url: &str) {
        let parsed: FlakeRef = url.parse().unwrap();
        // With insufficient path components, should return None.
        assert!(
            parsed.repo().is_none() || parsed.owner().is_none(),
            "URLs with insufficient path components should return None for missing parts"
        );
    }

    #[rstest]
    #[case::git_https_default_port_returns_host_only("git+https://example.com/o/r", "example.com")]
    #[case::git_https_non_default_port_returns_host_with_port(
        "git+https://localhost:3000/o/r",
        "localhost:3000"
    )]
    #[case::git_https_explicit_default_port_strips(
        "git+https://example.com:443/o/r",
        "example.com"
    )]
    #[case::git_ssh_default_port_returns_host_only("git+ssh://example.com/o/r", "example.com")]
    #[case::git_ssh_non_default_port_returns_host_with_port(
        "git+ssh://example.com:2222/o/r",
        "example.com:2222"
    )]
    #[case::git_ssh_explicit_default_port_strips("git+ssh://example.com:22/o/r", "example.com")]
    #[case::git_http_non_default_port_returns_host_with_port(
        "git+http://example.com:8080/o/r",
        "example.com:8080"
    )]
    fn domain_retains_non_default_port(#[case] url: &str, #[case] expected: &str) {
        // Mirrors upstream HTTP-library `Authority` semantics: the public `domain()`
        // accessor reflects what a downstream fetcher must target, so a non-default
        // `:port` survives. `flake-edit` consumes `domain()` directly as
        // `api_host_for(domain)` input. Without this, self-hosted forges on non-default
        // ports silently fetched from the scheme's default port.
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(
            parsed.domain(),
            Some(expected.to_string()),
            "domain mismatch for {url}"
        );
    }

    #[rstest]
    #[case("git+ssh://git@host/owner/repo", "host")]
    #[case("git+https://host/owner/repo", "host")]
    fn domain_strips_user_and_path(#[case] url: &str, #[case] expected_host: &str) {
        let parsed: FlakeRef = url.parse().unwrap();
        assert_eq!(
            parsed.domain(),
            Some(expected_host.to_string()),
            "domain mismatch for {url}"
        );
    }
}
