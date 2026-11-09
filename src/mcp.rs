//! MCP server: exposes the notebook to AI agents over stdio.
//!
//! Run with `think --mcp`. The agent launches this as a subprocess and speaks
//! JSON-RPC 2.0 over stdin/stdout. No network, no auth — the OS process
//! boundary is the security boundary.

use std::fs;
use std::path::PathBuf;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    schemars, tool, tool_router,
    transport::stdio,
    ErrorData, ServerHandler, ServiceExt,
};
use serde::Deserialize;

use crate::paths::Paths;

/// The MCP server state. Holds the notebook path so tools can read/write it.
#[derive(Clone)]
pub struct ThinkMcp {
    #[allow(dead_code)] // used by the tool_router macro
    tool_router: ToolRouter<ThinkMcp>,
    notebook: PathBuf,
}

impl ThinkMcp {
    pub fn new() -> Self {
        let paths = Paths::new();
        Self {
            tool_router: Self::tool_router(),
            notebook: paths.notebook,
        }
    }

    /// For tests: use a custom notebook path.
    #[cfg(test)]
    pub fn with_notebook(notebook: PathBuf) -> Self {
        Self {
            tool_router: Self::tool_router(),
            notebook,
        }
    }
}

// ─── Tool parameters ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(description = "Parameters for writing to the notebook")]
struct WriteParams {
    #[schemars(description = "The full text to write to the notebook")]
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(description = "Parameters for inserting text at a position")]
struct InsertParams {
    #[schemars(
        description = "The 1-based line number after which to insert. 0 inserts at the beginning."
    )]
    line: usize,
    #[schemars(description = "The text to insert. It becomes its own line(s).")]
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(description = "Parameters for searching the notebook")]
struct SearchParams {
    #[schemars(description = "The string to search for (case-insensitive)")]
    query: String,
}

// ─── Tools ─────────────────────────────────────────────────────────────────────

#[tool_router]
impl ThinkMcp {
    #[tool(
        name = "read_notebook",
        description = "Read the full text of the notebook."
    )]
    async fn read_notebook(&self) -> Result<CallToolResult, ErrorData> {
        let text = fs::read_to_string(&self.notebook).map_err(|e| {
            ErrorData::internal_error(format!("could not read notebook: {e}"), None)
        })?;
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }

    #[tool(
        name = "write_notebook",
        description = "Replace the full text of the notebook with the given content."
    )]
    async fn write_notebook(
        &self,
        Parameters(params): Parameters<WriteParams>,
    ) -> Result<CallToolResult, ErrorData> {
        fs::create_dir_all(
            self.notebook
                .parent()
                .ok_or_else(|| ErrorData::internal_error("no parent dir", None))?,
        )
        .map_err(|e| ErrorData::internal_error(format!("could not create directory: {e}"), None))?;
        fs::write(&self.notebook, &params.text).map_err(|e| {
            ErrorData::internal_error(format!("could not write notebook: {e}"), None)
        })?;
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Wrote {} bytes to the notebook.",
            params.text.len()
        ))]))
    }

    #[tool(
        name = "insert_text",
        description = "Insert text at a given line number. Line 0 inserts at the beginning."
    )]
    async fn insert_text(
        &self,
        Parameters(params): Parameters<InsertParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let existing = fs::read_to_string(&self.notebook).unwrap_or_default();
        let mut lines: Vec<String> = existing.lines().map(String::from).collect();

        if params.line == 0 {
            // Insert at the beginning
            for l in params.text.lines().rev() {
                lines.insert(0, l.to_owned());
            }
        } else if params.line >= lines.len() {
            // Append at the end
            for l in params.text.lines() {
                lines.push(l.to_owned());
            }
        } else {
            // Insert after the given line (1-based)
            let idx = params.line;
            for (i, l) in params.text.lines().enumerate() {
                lines.insert(idx + i, l.to_owned());
            }
        }

        let new_text = lines.join("\n");
        if !new_text.is_empty() {
            let new_text = format!("{new_text}\n");
            fs::write(&self.notebook, new_text).map_err(|e| {
                ErrorData::internal_error(format!("could not write notebook: {e}"), None)
            })?;
        }

        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Inserted {} line(s) at position {}.",
            params.text.lines().count(),
            params.line
        ))]))
    }

    #[tool(
        name = "search",
        description = "Search the notebook for a string (case-insensitive). Returns matching lines with their line numbers."
    )]
    async fn search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let text = fs::read_to_string(&self.notebook).map_err(|e| {
            ErrorData::internal_error(format!("could not read notebook: {e}"), None)
        })?;
        let query = params.query.to_lowercase();
        let matches: Vec<String> = text
            .lines()
            .enumerate()
            .filter(|(_, line)| line.to_lowercase().contains(&query))
            .map(|(i, line)| format!("{}: {line}", i + 1))
            .collect();

        if matches.is_empty() {
            Ok(CallToolResult::success(vec![ContentBlock::text(
                "No matches found.",
            )]))
        } else {
            Ok(CallToolResult::success(vec![ContentBlock::text(
                matches.join("\n"),
            )]))
        }
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

/// Run the MCP server on stdio. Blocks until the client disconnects.
pub async fn run() {
    let server = ThinkMcp::new();
    let running = match server.serve(stdio()).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("MCP server initialization failed: {e}");
            return;
        }
    };
    let _ = running.waiting().await;
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_server(content: &str) -> (ThinkMcp, TempDir) {
        let dir = TempDir::new().unwrap();
        let notebook = dir.path().join("notebook.txt");
        fs::write(&notebook, content).unwrap();
        (ThinkMcp::with_notebook(notebook), dir)
    }

    fn tool_text(result: CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn read_notebook_returns_content() {
        let (server, _dir) = test_server("hello world\nsecond line\n");
        let result = server.read_notebook().await.unwrap();
        assert_eq!(tool_text(result), "hello world\nsecond line\n");
    }

    #[tokio::test]
    async fn read_notebook_empty_file() {
        let (server, _dir) = test_server("");
        let result = server.read_notebook().await.unwrap();
        assert_eq!(tool_text(result), "");
    }

    #[tokio::test]
    async fn read_notebook_missing_file_is_error() {
        let dir = TempDir::new().unwrap();
        let server = ThinkMcp::with_notebook(dir.path().join("nonexistent.txt"));
        let result = server.read_notebook().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn write_notebook_replaces_content() {
        let (server, _dir) = test_server("old content");
        let params = WriteParams {
            text: "new content\n".to_owned(),
        };
        let result = server.write_notebook(Parameters(params)).await.unwrap();
        assert!(tool_text(result).contains("Wrote"));

        let on_disk = fs::read_to_string(server.notebook).unwrap();
        assert_eq!(on_disk, "new content\n");
    }

    #[tokio::test]
    async fn write_notebook_creates_parent_dir() {
        let dir = TempDir::new().unwrap();
        let notebook = dir.path().join("sub/dir/notebook.txt");
        let server = ThinkMcp::with_notebook(notebook.clone());
        let params = WriteParams {
            text: "created".to_owned(),
        };
        server.write_notebook(Parameters(params)).await.unwrap();
        assert_eq!(fs::read_to_string(notebook).unwrap(), "created");
    }

    #[tokio::test]
    async fn insert_text_at_beginning() {
        let (server, _dir) = test_server("line2\nline3\n");
        let params = InsertParams {
            line: 0,
            text: "line1".to_owned(),
        };
        server.insert_text(Parameters(params)).await.unwrap();
        let on_disk = fs::read_to_string(server.notebook).unwrap();
        assert_eq!(on_disk, "line1\nline2\nline3\n");
    }

    #[tokio::test]
    async fn insert_text_in_middle() {
        let (server, _dir) = test_server("a\nb\nc\n");
        let params = InsertParams {
            line: 1,
            text: "x\ny".to_owned(),
        };
        server.insert_text(Parameters(params)).await.unwrap();
        let on_disk = fs::read_to_string(server.notebook).unwrap();
        assert_eq!(on_disk, "a\nx\ny\nb\nc\n");
    }

    #[tokio::test]
    async fn insert_text_at_end() {
        let (server, _dir) = test_server("a\nb\n");
        let params = InsertParams {
            line: 99,
            text: "c".to_owned(),
        };
        server.insert_text(Parameters(params)).await.unwrap();
        let on_disk = fs::read_to_string(server.notebook).unwrap();
        assert_eq!(on_disk, "a\nb\nc\n");
    }

    #[tokio::test]
    async fn insert_text_into_empty_file() {
        let (server, _dir) = test_server("");
        let params = InsertParams {
            line: 0,
            text: "first".to_owned(),
        };
        server.insert_text(Parameters(params)).await.unwrap();
        let on_disk = fs::read_to_string(server.notebook).unwrap();
        assert_eq!(on_disk, "first\n");
    }

    #[tokio::test]
    async fn search_finds_matches() {
        let (server, _dir) = test_server("Hello World\nhello there\nGOODBYE\n");
        let params = SearchParams {
            query: "hello".to_owned(),
        };
        let result = server.search(Parameters(params)).await.unwrap();
        let text = tool_text(result);
        assert!(text.contains("1: Hello World"));
        assert!(text.contains("2: hello there"));
        assert!(!text.contains("GOODBYE"));
    }

    #[tokio::test]
    async fn search_case_insensitive() {
        let (server, _dir) = test_server("Rust is great\n");
        let params = SearchParams {
            query: "rust".to_owned(),
        };
        let result = server.search(Parameters(params)).await.unwrap();
        assert!(tool_text(result).contains("1: Rust is great"));
    }

    #[tokio::test]
    async fn search_no_matches() {
        let (server, _dir) = test_server("nothing here\n");
        let params = SearchParams {
            query: "zzz".to_owned(),
        };
        let result = server.search(Parameters(params)).await.unwrap();
        assert_eq!(tool_text(result), "No matches found.");
    }

    #[tokio::test]
    async fn search_empty_notebook() {
        let (server, _dir) = test_server("");
        let params = SearchParams {
            query: "anything".to_owned(),
        };
        let result = server.search(Parameters(params)).await.unwrap();
        assert_eq!(tool_text(result), "No matches found.");
    }

    #[test]
    fn server_info_has_correct_name() {
        let dir = TempDir::new().unwrap();
        let server = ThinkMcp::with_notebook(dir.path().join("n.txt"));
        let info = server.get_info();
        assert_eq!(info.server_info.name, "think");
    }
}
