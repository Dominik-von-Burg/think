# think

A private, automatically synced plain-text notebook. It saves your notes to `notebook.txt` and, after 3 seconds without typing, commits and pushes the file to a private GitHub repository.

## Build

Install [Rust](https://rustup.rs), then run:

```sh
cargo run --release
```

On Windows, `.\build.ps1` builds `dist\Think.exe`.

On Linux, Think uses GTK 3 and the desktop icon theme to draw its window buttons
when GTK is available.

## Tests

Run `cargo test --locked` on Linux or Windows. On Linux, install `xvfb`, `xauth`,
and `xdotool` to check the window corners and autosave with a temporary notebook:

```sh
cargo build --locked
xvfb-run -a -s '-screen 0 1280x900x24' bash tests/linux-desktop.sh target/debug/think
xvfb-run -a -s '-screen 0 1280x900x24' python3 tests/mcp-http.py target/debug/think
```

## Downloads

GitHub Actions builds both binaries on every push and pull request: `think-linux-x86_64.tar.gz`
(unpacked binary is `think`) and `think-windows-x86_64.exe`. They are attached to the run under
[Actions](https://github.com/Dominik-von-Burg/think/actions), and pushing a tag named `v*`
publishes the same two files on a [release](https://github.com/Dominik-von-Burg/think/releases).
The Linux binary is built on Ubuntu 22.04 and needs only `libc`, `libm`, `libz` and `libgcc_s`,
which every current desktop distribution ships.

## Sync setup

1. Create an empty private GitHub repository.
2. Open the gear in the app and enter the repository's HTTPS URL.
3. Create a fine-grained personal access token with read/write access to the repository's Contents, and paste it in.

The token is stored in your OS credential store (Windows Credential Manager, Linux Secret Service / GNOME Keyring). The repository URL is stored in `settings.toml` in the app's data directory (`~/.local/share/think/` on Linux, `%LOCALAPPDATA%\think\` on Windows).

If a sync fails, click **Sync failed** in the title bar to see the reason. Use **Copy details** to copy the error or **Retry sync** to try again without editing the notebook.

## MCP server

Think serves [MCP Streamable HTTP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#streamable-http) while the desktop app is open. It is off until you open the gear, select **MCP access**, and click **Approve MCP client**.

Configure your local MCP client with the URL `http://127.0.0.1:8374/mcp` and an `Authorization: Bearer <token>` header. Use **Copy access token** in Think and treat the token like a password. Clients that support Streamable HTTP and a bearer header can connect directly. Remove any older command-based `think --mcp` configuration.

The endpoint listens only on loopback and rejects browser origins. The approved token grants read and write access. Think stores it in its per-user data directory and starts the endpoint on subsequent launches. **Revoke access** removes the approval and stops the endpoint. Approving again generates a new token.

`read_notebook` and `search` return the live text or search results and a revision. Pass that revision as `expected_revision` to `insert_text` or `write_notebook`. If you type after the read, Think rejects the stale request. Successful edits appear in the editor and are saved immediately. Inserted text is highlighted until you have been idle for five seconds. **Undo MCP edit** is available until the next notebook edit.

## License

MIT. See [LICENSE](LICENSE). Bundled font and icon licenses are in `assets/`.
