//! Sync only notebook.txt through GitHub's Contents API.

use std::{fmt, fs, io, path::Path, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use git2::{ObjectType, Oid, Repository};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use ureq::{http::Response, Agent, Body};
use url::Url;

use crate::notebook;

const API: &str = "https://api.github.com";
const FILE: &str = "notebook.txt";
const STATE: &str = "sync-state.toml";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct Target {
    repository_id: u64,
    branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteVersion {
    target: Target,
    sha: Option<String>,
    pub(crate) repository_url: String,
}

#[derive(Debug)]
pub(crate) enum SyncError {
    Message(String),
    Conflict {
        message: String,
        remote: RemoteVersion,
    },
}

impl From<String> for SyncError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) | Self::Conflict { message, .. } => formatter.write_str(message),
        }
    }
}

impl SyncError {
    pub(crate) fn redact(self, token: &str) -> Self {
        match self {
            Self::Message(message) if !token.is_empty() => {
                Self::Message(message.replace(token, "[redacted]"))
            }
            other => other,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Repo {
    owner: String,
    name: String,
}

impl Repo {
    fn parse(value: &str) -> Result<Self, String> {
        let error = || {
            "Enter a GitHub repository URL such as https://github.com/owner/repository.".to_owned()
        };
        let url = Url::parse(value.trim()).map_err(|_| error())?;
        if url.scheme() != "https"
            || url.host_str() != Some("github.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error());
        }
        let (owner, name) = url
            .path()
            .trim_matches('/')
            .split_once('/')
            .ok_or_else(error)?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        let valid = |part: &str| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        };
        if !valid(owner) || !valid(name) {
            return Err(error());
        }
        Ok(Self {
            owner: owner.to_ascii_lowercase(),
            name: name.to_ascii_lowercase(),
        })
    }

    fn web_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }
}

#[derive(Deserialize)]
struct RepositoryInfo {
    id: u64,
    default_branch: String,
}

#[derive(Deserialize)]
struct FileInfo {
    sha: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize, Serialize)]
struct SyncState {
    target: Target,
    sha: String,
}

#[derive(Serialize)]
struct Update<'a> {
    message: &'static str,
    content: String,
    branch: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha: Option<&'a str>,
}

struct Github<'a> {
    agent: Agent,
    endpoint: String,
    authorization: String,
    repo: &'a Repo,
}

impl<'a> Github<'a> {
    fn new(root: &str, repo: &'a Repo, token: &str) -> Self {
        let agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(25)))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            agent,
            endpoint: format!("{root}/repos/{}/{}", repo.owner, repo.name),
            authorization: format!("Bearer {token}"),
            repo,
        }
    }

    fn get(&self, url: &str) -> Result<Response<Body>, String> {
        self.agent
            .get(url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "Think-notebook")
            .call()
            .map_err(|error| format!("Could not contact the GitHub API: {error}"))
    }

    fn target(&self) -> Result<Target, String> {
        let response = self.get(&self.endpoint)?;
        let info: RepositoryInfo = decode(response, "read the repository's default branch")?;
        if info.default_branch.is_empty() {
            return Err("GitHub did not report a default branch.".to_owned());
        }
        Ok(Target {
            repository_id: info.id,
            branch: info.default_branch,
        })
    }

    fn file(&self, target: &Target) -> Result<RemoteVersion, String> {
        let mut url = Url::parse(&format!("{}/contents/{FILE}", self.endpoint))
            .expect("GitHub endpoint and repository names are valid URLs");
        url.query_pairs_mut().append_pair("ref", &target.branch);
        let response = self.get(url.as_str())?;
        let sha = if response.status().as_u16() == 404 {
            None
        } else {
            let info: FileInfo = decode(response, "read notebook.txt on GitHub")?;
            if info.kind != "file" || Oid::from_str(&info.sha).is_err() {
                return Err(
                    "GitHub's notebook.txt is not a regular file with a valid revision.".to_owned(),
                );
            }
            Some(info.sha)
        };
        Ok(RemoteVersion {
            target: target.clone(),
            sha,
            repository_url: self.repo.web_url(),
        })
    }

    fn put(&self, remote: &RemoteVersion, text: &str) -> Result<(), (Option<u16>, String)> {
        let body = Update {
            message: "Update notebook",
            content: STANDARD.encode(text.as_bytes()),
            branch: &remote.target.branch,
            sha: remote.sha.as_deref(),
        };
        let response = self
            .agent
            .put(format!("{}/contents/{FILE}", self.endpoint))
            .header("Authorization", &self.authorization)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "Think-notebook")
            .send_json(&body)
            .map_err(|error| (None, format!("Could not contact the GitHub API: {error}")))?;
        let status = response.status().as_u16();
        if matches!(status, 200 | 201) {
            Ok(())
        } else {
            Err((
                Some(status),
                api_error(response, "write notebook.txt on GitHub"),
            ))
        }
    }
}

fn decode<T: DeserializeOwned>(mut response: Response<Body>, operation: &str) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(api_error(response, operation));
    }
    response
        .body_mut()
        .read_json()
        .map_err(|error| format!("Could not {operation}: invalid GitHub response ({error})"))
}

fn api_error(mut response: Response<Body>, operation: &str) -> String {
    #[derive(Deserialize)]
    struct ErrorBody {
        message: String,
    }
    let status = response.status().as_u16();
    let reason = response
        .body_mut()
        .read_json::<ErrorBody>()
        .ok()
        .map(|body| format!(": {}", body.message))
        .unwrap_or_default();
    format!("Could not {operation} (HTTP {status}){reason}")
}

pub(crate) fn sync_notebook(
    notebook_path: &Path,
    data_dir: &Path,
    url: &str,
    token: &str,
    approved: Option<&RemoteVersion>,
) -> Result<(), SyncError> {
    let repo = Repo::parse(url)?;
    let github = Github::new(API, &repo, token);
    sync(&github, notebook_path, data_dir, approved).map_err(|error| error.redact(token))
}

fn sync(
    github: &Github<'_>,
    notebook_path: &Path,
    data_dir: &Path,
    approved: Option<&RemoteVersion>,
) -> Result<(), SyncError> {
    let text = fs::read_to_string(notebook_path)
        .map_err(|error| format!("Could not read the saved notebook: {error}"))?;
    let local_sha = Oid::hash_object(ObjectType::Blob, text.as_bytes())
        .map_err(|error| format!("Could not calculate the notebook revision: {error}"))?
        .to_string();
    let target = github.target()?;
    let mut remote = github.file(&target)?;
    let state_path = data_dir.join(STATE);
    let baseline = match fs::read_to_string(&state_path) {
        Ok(contents) => toml::from_str::<SyncState>(&contents)
            .ok()
            .filter(|state| state.target == target)
            .map(|state| state.sha),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            legacy_baseline(data_dir, github.repo, &target.branch)
        }
        Err(error) => {
            return Err(format!("Could not read the notebook's sync state: {error}").into())
        }
    };

    // Equal content also recovers a sync whose successful response was lost.
    if remote.sha.as_deref() != Some(local_sha.as_str()) {
        let allowed = if let Some(approved) = approved {
            approved == &remote
        } else {
            remote.sha == baseline
        };
        if !allowed {
            return Err(conflict(remote));
        }
        if let Err((status, error)) = github.put(&remote, &text) {
            if matches!(status, Some(409 | 422)) {
                if let Ok(latest) = github.file(&target) {
                    if latest.sha.as_deref() == Some(local_sha.as_str()) {
                        remote = latest;
                    } else if latest != remote {
                        return Err(conflict(latest));
                    }
                }
            }
            if remote.sha.as_deref() != Some(local_sha.as_str()) {
                return Err(error.into());
            }
        }
    }

    let contents = toml::to_string(&SyncState {
        target,
        sha: local_sha,
    })
    .map_err(|error| format!("Could not encode the sync state: {error}"))?;
    notebook::save(&state_path, &contents)
        .map_err(|error| format!("Could not record the synced GitHub version: {error}").into())
}

fn conflict(remote: RemoteVersion) -> SyncError {
    let message = if remote.sha.is_some() {
        "GitHub has a different copy of notebook.txt. Compare it with your local text before replacing it."
    } else {
        "notebook.txt was removed from GitHub. You can upload your local copy to restore it."
    };
    SyncError::Conflict {
        message: message.to_owned(),
        remote,
    }
}

// Trust only the remote-tracking tree of the same repository and branch.
// A pending local commit can contain edits GitHub has never seen.
fn legacy_baseline(data_dir: &Path, repo: &Repo, branch: &str) -> Option<String> {
    let repository = Repository::open(data_dir.join("repository")).ok()?;
    let origin = repository.find_remote("origin").ok()?;
    if &Repo::parse(origin.url()?).ok()? != repo {
        return None;
    }
    let tree = repository
        .find_reference(&format!("refs/remotes/origin/{branch}"))
        .ok()?
        .peel_to_tree()
        .ok()?;
    let notebook = tree.get_path(Path::new(FILE)).ok()?;
    (notebook.kind() == Some(ObjectType::Blob)).then(|| notebook.id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::serde_json::{self, json, Value};
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        thread,
    };

    const REPO_PATH: &str = "/repos/owner/notes";
    const FILE_PATH: &str = "/repos/owner/notes/contents/notebook.txt";
    const TOKEN: &str = "fixture-token";

    struct Exchange {
        request: String,
        body: Option<Value>,
        status: &'static str,
        response: Value,
    }

    fn get(path: impl Into<String>, status: &'static str, response: Value) -> Exchange {
        Exchange {
            request: format!("GET {} HTTP/1.1", path.into()),
            body: None,
            status,
            response,
        }
    }

    fn put(status: &'static str, body: Value) -> Exchange {
        Exchange {
            request: format!("PUT {FILE_PATH} HTTP/1.1"),
            body: Some(body),
            status,
            response: json!({}),
        }
    }

    fn mock(exchanges: Vec<Exchange>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            for expected in exchanges {
                let mut stream = (0..300)
                    .find_map(|_| match listener.accept() {
                        Ok((stream, _)) => Some(stream),
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                            None
                        }
                        Err(error) => panic!("mock listener failed: {error}"),
                    })
                    .expect("Think did not make the expected API request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line.trim_end(), expected.request);
                let mut length = 0;
                let mut authenticated = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    let Some((name, value)) = line.split_once(':') else {
                        break;
                    };
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                    if name.eq_ignore_ascii_case("authorization") {
                        authenticated = value.trim() == format!("Bearer {TOKEN}");
                    }
                }
                assert!(authenticated);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if let Some(expected) = expected.body {
                    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), expected);
                } else {
                    assert!(body.is_empty());
                }
                let json = expected.response.to_string();
                write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    expected.status, json.len(), json,
                ).unwrap();
            }
        });
        (root, worker)
    }

    fn repo() -> Repo {
        Repo::parse("https://github.com/Owner/Notes.git").unwrap()
    }

    fn target() -> Target {
        Target {
            repository_id: 123,
            branch: "master".to_owned(),
        }
    }

    fn repo_metadata() -> Exchange {
        get(
            REPO_PATH,
            "200 OK",
            json!({"id": 123, "default_branch": "master"}),
        )
    }

    fn remote_file(sha: &str) -> Exchange {
        get(
            format!("{FILE_PATH}?ref=master"),
            "200 OK",
            json!({"type": "file", "sha": sha}),
        )
    }

    fn blob(text: &str) -> String {
        Oid::hash_object(ObjectType::Blob, text.as_bytes())
            .unwrap()
            .to_string()
    }

    fn set_baseline(dir: &Path, text: &str) {
        let state = SyncState {
            target: target(),
            sha: blob(text),
        };
        fs::write(dir.join(STATE), toml::to_string(&state).unwrap()).unwrap();
    }

    #[test]
    fn api_update_uses_the_notebook_sha_on_the_default_branch_without_a_clone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, "updated note").unwrap();
        set_baseline(dir.path(), "old note");
        let (root, worker) = mock(vec![
            repo_metadata(),
            remote_file(&blob("old note")),
            put(
                "200 OK",
                json!({
                    "message": "Update notebook",
                    "content": STANDARD.encode("updated note"),
                    "branch": "master",
                    "sha": blob("old note"),
                }),
            ),
        ]);
        sync(&Github::new(&root, &repo(), TOKEN), &path, dir.path(), None).unwrap();
        worker.join().unwrap();
        let state: SyncState =
            toml::from_str(&fs::read_to_string(dir.path().join(STATE)).unwrap()).unwrap();
        assert_eq!(state.sha, blob("updated note"));
        assert!(!dir.path().join("repository").exists());
    }

    #[test]
    fn api_creates_a_missing_notebook_without_a_sha() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, "new note").unwrap();
        let (root, worker) = mock(vec![
            repo_metadata(),
            get(
                format!("{FILE_PATH}?ref=master"),
                "404 Not Found",
                json!({}),
            ),
            put(
                "201 Created",
                json!({
                    "message": "Update notebook",
                    "content": STANDARD.encode("new note"),
                    "branch": "master",
                }),
            ),
        ]);
        sync(&Github::new(&root, &repo(), TOKEN), &path, dir.path(), None).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn conflicting_notebook_requires_approval_and_a_new_edit_invalidates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        fs::write(&path, "local edit").unwrap();
        set_baseline(dir.path(), "common note");
        let initial_remote = blob("remote edit");
        let newer_remote = blob("newer remote edit");
        let (root, worker) = mock(vec![
            repo_metadata(),
            remote_file(&initial_remote),
            repo_metadata(),
            remote_file(&initial_remote),
            put(
                "409 Conflict",
                json!({
                    "message": "Update notebook",
                    "content": STANDARD.encode("local edit"),
                    "branch": "master",
                    "sha": initial_remote,
                }),
            ),
            remote_file(&newer_remote),
            repo_metadata(),
            remote_file(&newer_remote),
            put(
                "200 OK",
                json!({
                    "message": "Update notebook",
                    "content": STANDARD.encode("local edit"),
                    "branch": "master",
                    "sha": newer_remote,
                }),
            ),
        ]);
        let repo = repo();
        let github = Github::new(&root, &repo, TOKEN);
        let Err(SyncError::Conflict { remote, .. }) = sync(&github, &path, dir.path(), None) else {
            panic!("expected a conflict before uploading");
        };
        let Err(SyncError::Conflict { remote: latest, .. }) =
            sync(&github, &path, dir.path(), Some(&remote))
        else {
            panic!("an edit on GitHub invalidates approval");
        };
        assert_eq!(latest.sha.as_deref(), Some(newer_remote.as_str()));
        sync(&github, &path, dir.path(), Some(&latest)).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "local edit");
        worker.join().unwrap();
    }

    #[test]
    fn legacy_migration_uses_the_tracking_commit_and_ignores_pending_local_changes() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = Repository::init(dir.path().join("repository")).unwrap();
        legacy
            .remote("origin", "https://github.com/Owner/Notes.git")
            .unwrap();
        let signature = git2::Signature::now("Think test", "test@example.invalid").unwrap();
        let write = |text: &str, parents: &[&git2::Commit<'_>]| {
            let oid = legacy.blob(text.as_bytes()).unwrap();
            let mut tree = legacy.treebuilder(None).unwrap();
            tree.insert(FILE, oid, 0o100644).unwrap();
            let tree = legacy.find_tree(tree.write().unwrap()).unwrap();
            legacy
                .commit(Some("HEAD"), &signature, &signature, "note", &tree, parents)
                .unwrap()
        };
        let synced = write("last uploaded", &[]);
        legacy
            .reference("refs/remotes/origin/master", synced, false, "test")
            .unwrap();
        let parent = legacy.find_commit(synced).unwrap();
        write("pending local edit", &[&parent]);
        assert_eq!(
            legacy_baseline(dir.path(), &repo(), "master"),
            Some(blob("last uploaded"))
        );
        assert_eq!(legacy_baseline(dir.path(), &repo(), "main"), None);
        let other = Repo::parse("https://github.com/owner/other").unwrap();
        assert_eq!(legacy_baseline(dir.path(), &other, "master"), None);
    }

    #[test]
    fn only_github_repository_urls_are_accepted() {
        for invalid in [
            "http://github.com/owner/repo",
            "https://github.com.example.org/owner/repo",
            "https://token@github.com/owner/repo",
            "https://github.com/owner/repo/tree/main",
        ] {
            assert!(Repo::parse(invalid).is_err(), "{invalid}");
        }
        assert_eq!(repo().web_url(), "https://github.com/owner/notes");
    }
}
