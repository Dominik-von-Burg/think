# think

A private, automatically synced plain-text notebook. It saves your notes to `notebook.txt` and, after 3 seconds without typing, commits and pushes the file to a private GitHub repository.

## Build

Install [Rust](https://rustup.rs), then run:

```sh
cargo run --release
```

On Windows, `.\build.ps1` builds `dist\Think.exe`.

## Tests

Run `cargo test --locked` on Linux or Windows. On Debian or Ubuntu, install
`xvfb`, `xauth`, `xdotool`, `x11-utils`, and `openbox` to check the native window
frame and autosave with a temporary notebook:

```sh
cargo build --locked
xvfb-run -a -s '-screen 0 1280x900x24' bash tests/linux-desktop.sh target/debug/think
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

## MCP server

Think can run as a [Model Context Protocol](https://modelcontextprotocol.io/) server so that AI agents can read and edit your notebook:

```sh
think --mcp
```

It speaks JSON-RPC over stdio. Point your MCP client (Claude Desktop, Cursor, etc.) at the binary with the `--mcp` argument. The server exposes four tools: `read_notebook`, `write_notebook`, `insert_text`, and `search`.

## License

MIT. See [LICENSE](LICENSE). Bundled font and icon licenses are in `assets/`.
