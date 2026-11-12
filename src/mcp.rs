//! The desktop app's opt-in Streamable HTTP MCP endpoint.

use std::{io, net::TcpListener, ops::Range, thread::JoinHandle};

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    schemars,
    serde_json::json,
    tool, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, ServerHandler,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::{
    runtime::Builder,
    sync::{mpsc, oneshot},
};

const ADDRESS: &str = "127.0.0.1:8374";
pub(crate) const URL: &str = "http://127.0.0.1:8374/mcp";
pub(crate) const TOKEN_FILE: &str = "mcp-token";

pub(crate) struct McpServer {
    pub(crate) token: String,
    pub(crate) requests: mpsc::Receiver<NotebookRequest>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl McpServer {
    pub(crate) fn start(ctx: egui::Context, token: String) -> io::Result<Self> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid MCP access token",
            ));
        }
        let listener = TcpListener::bind(ADDRESS)?;
        listener.set_nonblocking(true)?;
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let (sender, requests) = mpsc::channel(32);
        let router = router(sender, ctx, &token);
        // A current-thread runtime needs a thread driving it while egui runs.
        let listener = {
            let _guard = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };
        let (stop, stopped) = oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("think-mcp".to_owned())
            .spawn(move || {
                runtime.block_on(async move {
                    tokio::select! {
                        _ = axum::serve(listener, router) => {}
                        _ = stopped => {}
                    }
                });
            })?;
        Ok(Self {
            token,
            requests,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.requests.close();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn new_token() -> io::Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(hex(&bytes))
}

fn router(sender: mpsc::Sender<NotebookRequest>, ctx: egui::Context, token: &str) -> Router {
    let expected = format!("Bearer {token}");
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts([ADDRESS, "localhost:8374"])
        .enforce_origin_validation();
    let service = StreamableHttpService::new(
        move || {
            Ok(ThinkMcp {
                sender: sender.clone(),
                ctx: ctx.clone(),
            })
        },
        Default::default(),
        config,
    );
    let service: StreamableHttpService<ThinkMcp, LocalSessionManager> = service;
    Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn(move |request, next| {
            authorize(request, next, expected.clone())
        }))
}

async fn authorize(request: Request, next: Next, expected: String) -> Response {
    if request.headers().contains_key(header::ORIGIN) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let approved = request
        .headers()
        .get(header::AUTHORIZATION)
        .is_some_and(|value| bool::from(value.as_bytes().ct_eq(expected.as_bytes())));
    if !approved {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }
    next.run(request).await
}

#[derive(Clone)]
struct ThinkMcp {
    sender: mpsc::Sender<NotebookRequest>,
    ctx: egui::Context,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WriteParams {
    #[schemars(description = "The full text to write to the notebook")]
    text: String,
    #[schemars(description = "The revision returned by read_notebook or search")]
    expected_revision: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InsertParams {
    #[schemars(
        description = "The 1-based line after which to insert. 0 inserts at the beginning."
    )]
    line: usize,
    text: String,
    #[schemars(description = "The revision returned by read_notebook or search")]
    expected_revision: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchParams {
    query: String,
}

#[tool_router]
impl ThinkMcp {
    #[tool(
        name = "read_notebook",
        description = "Read the live notebook and its revision."
    )]
    async fn read_notebook(&self) -> Result<CallToolResult, ErrorData> {
        Ok(self.request(Command::Read).await)
    }

    #[tool(
        name = "write_notebook",
        description = "Replace the live notebook if the expected revision still matches. Read it again if it changed."
    )]
    async fn write_notebook(
        &self,
        Parameters(params): Parameters<WriteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(self.request(Command::Write(params)).await)
    }

    #[tool(
        name = "insert_text",
        description = "Insert lines after the given line if the expected revision still matches. Read again if the notebook changed."
    )]
    async fn insert_text(
        &self,
        Parameters(params): Parameters<InsertParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(self.request(Command::Insert(params)).await)
    }

    #[tool(
        name = "search",
        description = "Find case-insensitive matching lines in the live notebook and return its revision."
    )]
    async fn search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(self.request(Command::Search(params.query)).await)
    }
}

impl ThinkMcp {
    async fn request(&self, command: Command) -> CallToolResult {
        let (reply, result) = oneshot::channel();
        if self
            .sender
            .try_send(NotebookRequest { command, reply })
            .is_err()
        {
            return tool_error(
                "Think is busy or MCP access was revoked. Read again before retrying.",
            );
        }
        self.ctx.request_repaint();
        result
            .await
            .unwrap_or_else(|_| tool_error("Think closed or MCP access was revoked."))
    }
}

#[rmcp::tool_handler]
impl ServerHandler for ThinkMcp {
    fn get_info(&self) -> ServerConfig {
        let mut config = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        config.server_info.name = "think".into();
        config.server_info.version = env!("CARGO_PKG_VERSION").into();
        config
    }
}

enum Command {
    Read,
    Search(String),
    Write(WriteParams),
    Insert(InsertParams),
}

pub(crate) struct NotebookRequest {
    command: Command,
    pub(crate) reply: oneshot::Sender<CallToolResult>,
}

pub(crate) struct Edit {
    pub(crate) text: String,
    pub(crate) inserted: Option<Range<usize>>,
}

impl NotebookRequest {
    pub(crate) fn run(
        &self,
        current: &str,
        save: impl FnOnce(&str) -> io::Result<()>,
    ) -> (CallToolResult, Option<Edit>) {
        self.command.run(current, save)
    }
}

impl Command {
    fn run(
        &self,
        current: &str,
        save: impl FnOnce(&str) -> io::Result<()>,
    ) -> (CallToolResult, Option<Edit>) {
        let current_revision = revision(current);
        let (expected, edit, message) = match self {
            Self::Read => return (result(current.to_owned(), &current_revision), None),
            Self::Search(query) => {
                let query = query.to_lowercase();
                let matches = current
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.to_lowercase().contains(&query))
                    .map(|(i, line)| format!("{}: {line}", i + 1))
                    .collect::<Vec<_>>();
                let text = if matches.is_empty() {
                    "No matches found.".to_owned()
                } else {
                    matches.join("\n")
                };
                return (result(text, &current_revision), None);
            }
            Self::Write(params) => (
                &params.expected_revision,
                Some(Edit {
                    text: params.text.clone(),
                    inserted: None,
                }),
                format!("Wrote {} bytes to the notebook.", params.text.len()),
            ),
            Self::Insert(params) => (
                &params.expected_revision,
                insert(current, params.line, &params.text),
                format!("Inserted {} line(s).", params.text.lines().count()),
            ),
        };
        if *expected != current_revision {
            return (
                tool_error("Notebook changed. Read it again before retrying your edit."),
                None,
            );
        }
        let Some(edit) = edit.filter(|edit| edit.text != current) else {
            return (
                result("Notebook is unchanged.".to_owned(), &current_revision),
                None,
            );
        };
        if let Err(error) = save(&edit.text) {
            return (
                tool_error(format!("Could not save notebook: {error}")),
                None,
            );
        }
        (result(message, &revision(&edit.text)), Some(edit))
    }
}

fn revision(text: &str) -> String {
    hex(Sha256::digest(text.as_bytes()).as_slice())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    output
}

fn result(text: String, revision: &str) -> CallToolResult {
    let mut result = CallToolResult::success(vec![
        ContentBlock::text(text.clone()),
        ContentBlock::text(format!("Notebook revision: {revision}")),
    ]);
    result.structured_content = Some(json!({ "text": text, "revision": revision }));
    result
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn insert(current: &str, line: usize, text: &str) -> Option<Edit> {
    if text.is_empty() {
        return None;
    }
    let offset = current.split_inclusive('\n').take(line).map(str::len).sum();
    let (before, after) = current.split_at(offset);
    let newline = match current.split_once('\n') {
        Some((first_line, _)) if first_line.ends_with('\r') => "\r\n",
        _ => "\n",
    };
    let mut updated = before.to_owned();
    if !before.is_empty() && !before.ends_with('\n') {
        updated.push_str(newline);
    }
    let start = updated.len();
    for line in text.lines() {
        updated.push_str(line);
        updated.push_str(newline);
    }
    let end = updated.len();
    updated.push_str(after);
    Some(Edit {
        text: updated,
        inserted: Some(start..end),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_search_share_the_revision_of_live_text() {
        let current = "Hello world\nhello again\nGoodbye";
        let (read, _) = Command::Read.run(current, |_| unreachable!());
        let (search, _) = Command::Search("HELLO".into()).run(current, |_| unreachable!());
        let read = read.structured_content.unwrap();
        let search = search.structured_content.unwrap();
        assert_eq!(read["text"], current);
        assert_eq!(search["text"], "1: Hello world\n2: hello again");
        assert_eq!(read["revision"], search["revision"]);
    }

    #[test]
    fn stale_insert_and_replace_cannot_overwrite_live_typing() {
        let expected_revision = revision("Before typing");
        for command in [
            Command::Insert(InsertParams {
                line: 1,
                text: "AI".into(),
                expected_revision: expected_revision.clone(),
            }),
            Command::Write(WriteParams {
                text: "AI replacement".into(),
                expected_revision,
            }),
        ] {
            let (result, edit) = command.run("Before typing\nNew human text", |_| {
                panic!("stale request must not save")
            });
            assert_eq!(result.is_error, Some(true));
            assert!(edit.is_none());
        }
    }

    #[test]
    fn inserts_keep_existing_line_endings_and_report_utf8_byte_ranges() {
        for (current, line, input, expected, highlighted) in [
            ("", 0, "first", "first\n", "first\n"),
            ("a\r\nb", 1, "é\nz", "a\r\né\r\nz\r\nb", "é\r\nz\r\n"),
            ("a", 99, "b", "a\nb\n", "b\n"),
            ("b\n", 0, "a", "a\nb\n", "a\n"),
        ] {
            let edit = insert(current, line, input).unwrap();
            assert_eq!(edit.text, expected);
            assert_eq!(&edit.text[edit.inserted.unwrap()], highlighted);
        }
        assert!(insert("a", 0, "").is_none());
    }

    #[test]
    fn failed_save_leaves_the_editor_untouched() {
        let command = Command::Write(WriteParams {
            text: "new".into(),
            expected_revision: revision("old"),
        });
        let (result, edit) = command.run("old", |_| Err(io::Error::other("disk failure")));
        assert_eq!(result.is_error, Some(true));
        assert!(edit.is_none());
    }
}
