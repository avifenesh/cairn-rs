//! Stdio transport for plugin process communication per RFC 007.
//!
//! Manages the child process lifecycle: spawn with allowlisted env,
//! write JSON-RPC requests to stdin, read responses from stdout,
//! handle shutdown and timeout.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use cairn_plugin_proto::wire::{JsonRpcRequest, JsonRpcResponse};

/// Errors from the stdio transport layer.
#[derive(Debug)]
pub enum TransportError {
    SpawnFailed(std::io::Error),
    WriteFailed(std::io::Error),
    ReadFailed(std::io::Error),
    InvalidResponse(String),
    ProcessExited(Option<i32>),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::SpawnFailed(e) => write!(f, "spawn failed: {e}"),
            TransportError::WriteFailed(e) => write!(f, "write failed: {e}"),
            TransportError::ReadFailed(e) => write!(f, "read failed: {e}"),
            TransportError::InvalidResponse(msg) => write!(f, "invalid response: {msg}"),
            TransportError::ProcessExited(code) => write!(f, "process exited: {code:?}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Configuration for spawning a plugin process.
#[derive(Clone, Debug)]
pub struct SpawnConfig {
    pub command: Vec<String>,
    pub allowed_env: Vec<String>,
    pub working_dir: Option<String>,
}

/// A running plugin process with stdio handles.
pub struct PluginProcess {
    child: Child,
}

impl PluginProcess {
    /// Spawn a plugin process with restricted environment.
    pub fn spawn(config: &SpawnConfig) -> Result<Self, TransportError> {
        if config.command.is_empty() {
            return Err(TransportError::SpawnFailed(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "empty command",
            )));
        }

        let mut cmd = Command::new(&config.command[0]);
        if config.command.len() > 1 {
            cmd.args(&config.command[1..]);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Clear environment and only pass allowed vars
        cmd.env_clear();
        for key in &config.allowed_env {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }

        if let Some(dir) = &config.working_dir {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(TransportError::SpawnFailed)?;
        Ok(Self { child })
    }

    /// Send a JSON-RPC request to the plugin's stdin.
    pub fn send(&mut self, request: &JsonRpcRequest) -> Result<(), TransportError> {
        let stdin = self.child.stdin.as_mut().ok_or_else(|| {
            TransportError::WriteFailed(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "stdin not available",
            ))
        })?;

        let json = serde_json::to_string(request).map_err(|e| {
            TransportError::WriteFailed(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e.to_string(),
            ))
        })?;

        stdin
            .write_all(json.as_bytes())
            .map_err(TransportError::WriteFailed)?;
        stdin
            .write_all(b"\n")
            .map_err(TransportError::WriteFailed)?;
        stdin.flush().map_err(TransportError::WriteFailed)?;

        Ok(())
    }

    /// Read a JSON-RPC response line from the plugin's stdout.
    pub fn recv(&mut self) -> Result<JsonRpcResponse, TransportError> {
        let stdout = self.child.stdout.as_mut().ok_or_else(|| {
            TransportError::ReadFailed(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "stdout not available",
            ))
        })?;

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(TransportError::ReadFailed)?;

        if line.is_empty() {
            return Err(TransportError::ProcessExited(None));
        }

        serde_json::from_str(&line).map_err(|e| TransportError::InvalidResponse(e.to_string()))
    }

    /// Check if the child process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Kill the child process.
    pub fn kill(&mut self) -> Result<(), TransportError> {
        self.child
            .kill()
            .map_err(|_| TransportError::ProcessExited(None))
    }

    /// Wait for the child to exit and return the exit code.
    pub fn wait(&mut self) -> Result<Option<i32>, TransportError> {
        let status = self
            .child
            .wait()
            .map_err(|_| TransportError::ProcessExited(None))?;
        Ok(status.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_config_construction() {
        let config = SpawnConfig {
            command: vec!["echo".to_owned(), "hello".to_owned()],
            allowed_env: vec!["PATH".to_owned()],
            working_dir: None,
        };
        assert_eq!(config.command.len(), 2);
    }

    #[test]
    fn empty_command_fails() {
        let config = SpawnConfig {
            command: vec![],
            allowed_env: vec![],
            working_dir: None,
        };
        let result = PluginProcess::spawn(&config);
        assert!(result.is_err());
    }

    #[test]
    fn transport_error_display() {
        let err = TransportError::InvalidResponse("bad json".to_owned());
        assert!(err.to_string().contains("bad json"));
    }
}
