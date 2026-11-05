//! The notebook's own git repository, and the push to the private remote.

use std::{fs, path::Path};

use git2::{
    build::RepoBuilder, Cred, FetchOptions, IndexAddOption, PushOptions, RemoteCallbacks,
    Repository, Signature,
};

pub(crate) fn callbacks(token: &str) -> RemoteCallbacks<'static> {
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
        Repository::open(repository_path).map_err(|e| e.message().to_owned())?
    } else {
        let mut fetch = FetchOptions::new();
        fetch.remote_callbacks(callbacks(token));
        RepoBuilder::new()
            .fetch_options(fetch)
            .clone(url, repository_path)
            .map_err(|e| format!("could not clone repository: {}", e.message()))?
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
    fs::copy(notebook, target).map_err(|e| e.to_string())?;

    let mut index = repository.index().map_err(|e| e.message().to_owned())?;
    index
        .add_all(["notebook.txt"], IndexAddOption::DEFAULT, None)
        .map_err(|e| e.message().to_owned())?;
    index.write().map_err(|e| e.message().to_owned())?;

    let tree_id = index.write_tree().map_err(|e| e.message().to_owned())?;
    let tree = repository
        .find_tree(tree_id)
        .map_err(|e| e.message().to_owned())?;
    let signature = Signature::now("Quiet Notebook", "notebook@local.invalid")
        .map_err(|e| e.message().to_owned())?;
    let parent = repository
        .head()
        .ok()
        .and_then(|head| head.peel_to_commit().ok());
    if let Some(parent) = parent.as_ref() {
        if parent.tree_id() == tree_id {
            return Ok(());
        }
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Update notebook",
                &tree,
                &[parent],
            )
            .map_err(|e| e.message().to_owned())?;
    } else {
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Update notebook",
                &tree,
                &[],
            )
            .map_err(|e| e.message().to_owned())?;
    }

    let head = repository.head().map_err(|e| e.message().to_owned())?;
    let branch = head.shorthand().unwrap_or("main");
    let mut remote = repository
        .find_remote("origin")
        .map_err(|e| e.message().to_owned())?;
    let mut push = PushOptions::new();
    push.remote_callbacks(callbacks(token));
    remote
        .push(
            &[&format!("refs/heads/{branch}:refs/heads/{branch}")],
            Some(&mut push),
        )
        .map_err(|e| format!("could not push: {}", e.message()))
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
}
