# think

A private, automatically synced plain-text notebook. It saves your notes to `notebook.txt` and, after 3 seconds without typing, commits and pushes the file to a private GitHub repository.

## Build

Install [Rust](https://rustup.rs), then run:

```sh
cargo run --release
```

On Windows, `.\build.ps1` builds `dist\Think.exe`.

## Sync setup

1. Create an empty private GitHub repository.
2. Open the gear in the app and enter the repository's HTTPS URL.
3. Create a fine-grained personal access token with read/write access to the repository's Contents, and paste it in.

The token is stored in plain text in `settings.toml` in the app's data directory (`~/.local/share/think/` on Linux, `%LOCALAPPDATA%\think\` on Windows).

## License

MIT. See [LICENSE](LICENSE). Bundled font and icon licenses are in `assets/`.
