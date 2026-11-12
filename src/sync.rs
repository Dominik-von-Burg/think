//! The notebook's own git repository, and the push to the private remote.

use std::{fs, path::Path};

use git2::{
    build::RepoBuilder, Cred, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks,
    Repository, Signature,
};

fn callbacks(token: &str) -> RemoteCallbacks<'_> {
    let token = token.to_owned();
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_, _, _| Cred::userpass_plaintext("x-access-token", &token));
    callbacks
}

pub(crate) fn commit_and_push(
    notebook: &Path,
    repository_path: &Path,
    url: &str,
    token: &str,
) -> Result<(), String> {
    let repository = if repository_path.exists() {
        Repository::open(repository_path)
            .map_err(|e| format!("Could not open the local sync repository: {}", e.message()))?
    } else {
        let mut fetch = FetchOptions::new();
        fetch.remote_callbacks(callbacks(token));
        RepoBuilder::new()
            .fetch_options(fetch)
            .clone(url, repository_path)
            .map_err(|e| format!("Could not clone the GitHub repository: {}", e.message()))?
    };

    // A freshly cloned empty GitHub repository has no commit yet, even if its
    // symbolic HEAD still points at Git's legacy `master` name. Start it on main.
    if repository
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok())
        .is_none()
    {
        repository
            .set_head("refs/heads/main")
            .map_err(|e| e.message().to_owned())?;
    }
    repository
        .remote_set_url("origin", url)
        .map_err(|e| e.message().to_owned())?;

    let target = repository_path.join("notebook.txt");
    fs::copy(notebook, target)
        .map_err(|e| format!("Could not copy the saved notebook into the sync repository: {e}"))?;

    let mut index = repository.index().map_err(|e| e.message().to_owned())?;
    index
        .add_all(["notebook.txt"], IndexAddOption::DEFAULT, None)
        .map_err(|e| e.message().to_owned())?;
    index.write().map_err(|e| e.message().to_owned())?;

    let tree_id = index.write_tree().map_err(|e| e.message().to_owned())?;
    let parent = repository
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok());
    if parent
        .as_ref()
        .is_none_or(|parent| parent.tree_id() != tree_id)
    {
        let tree = repository
            .find_tree(tree_id)
            .map_err(|e| e.message().to_owned())?;
        let signature = Signature::now("Quiet Notebook", "notebook@local.invalid")
            .map_err(|e| e.message().to_owned())?;
        let parents = parent.iter().collect::<Vec<_>>();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Update notebook",
                &tree,
                &parents,
            )
            .map_err(|e| format!("Could not commit the notebook: {}", e.message()))?;
    }

    let head = repository.head().map_err(|e| e.message().to_owned())?;
    let branch = head.shorthand().unwrap_or("main");
    let mut remote = repository
        .find_remote("origin")
        .map_err(|e| e.message().to_owned())?;
    let mut rejected = None;
    let result = {
        let mut callbacks = callbacks(token);
        callbacks.push_update_reference(|reference, status| {
            if let Some(status) = status {
                rejected = Some(format!("GitHub rejected the push to {reference}: {status}"));
            }
            Ok(())
        });
        let mut push = PushOptions::new();
        push.remote_callbacks(callbacks);
        remote.push(
            &[&format!("refs/heads/{branch}:refs/heads/{branch}")],
            Some(&mut push),
        )
    };
    match rejected {
        Some(error) => Err(error),
        None => result.map_err(|e| format!("Could not push to GitHub: {}", e.message())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use git2::Repository;
    use std::fs;

    // Keep: the sync path, the markdown header detection, and the fade bands,
    // which are pure functions of the scroll position.
    #[test]
    fn first_sync_creates_a_main_commit_and_pushes_it() {
        let root = std::env::temp_dir().join(format!("think-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let remote_path = root.join("remote.git");
        Repository::init_bare(&remote_path).unwrap();
        let notebook = root.join("notebook.txt");
        fs::write(&notebook, "private words").unwrap();

        commit_and_push(
            &notebook,
            &root.join("clone"),
            remote_path.to_str().unwrap(),
            "token",
        )
        .unwrap();

        let remote = Repository::open_bare(&remote_path).unwrap();
        let commit = remote
            .find_reference("refs/heads/main")
            .unwrap()
            .peel_to_commit()
            .unwrap();
        assert_eq!(commit.message(), Some("Update notebook"));
        drop(commit);
        drop(remote);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_pushes_the_existing_commit_after_a_failed_push() {
        let root = tempfile::tempdir().unwrap();
        let remote_path = root.path().join("remote.git");
        let offline_path = root.path().join("offline.git");
        let clone_path = root.path().join("clone");
        let notebook = root.path().join("notebook.txt");
        Repository::init_bare(&remote_path).unwrap();
        let url = remote_path.to_str().unwrap();
        fs::write(&notebook, "initial note").unwrap();
        commit_and_push(&notebook, &clone_path, url, "token").unwrap();

        fs::rename(&remote_path, &offline_path).unwrap();
        fs::write(&notebook, "edited note").unwrap();
        let error = commit_and_push(&notebook, &clone_path, url, "token").unwrap_err();
        assert!(error.contains("Could not push"), "{error}");
        let local = Repository::open(&clone_path).unwrap();
        let pending = local.refname_to_id("refs/heads/main").unwrap();

        fs::rename(&offline_path, &remote_path).unwrap();
        commit_and_push(&notebook, &clone_path, url, "token").unwrap();
        let remote = Repository::open_bare(&remote_path).unwrap();
        assert_eq!(remote.refname_to_id("refs/heads/main").unwrap(), pending);
        assert_eq!(local.refname_to_id("refs/heads/main").unwrap(), pending);
    }
}
