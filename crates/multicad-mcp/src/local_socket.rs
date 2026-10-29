//! Same-user local connections to a running desktop window. The window
//! listens on a Unix socket inside a private 0700 directory; a client starts
//! `multicad --mcp --app`, which only bridges its stdio to that socket.
use crate::{host::Host, server::Server};
use rmcp::ServiceExt;
use std::{
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub enum Event {
    Connected { connection: u64, client: String },
    Disconnected { connection: u64 },
}

/// The current user's identity, read from a file this process just created,
/// so ownership checks need no platform calls.
fn own_uid(directory: &Path) -> Result<u32, String> {
    let probe = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    Ok(probe.as_file().metadata().map_err(|e| e.to_string())?.uid())
}

/// The per-user socket path: under `$XDG_RUNTIME_DIR` when set, otherwise the
/// platform temporary directory (per-user on macOS).
pub fn default_endpoint() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let uid = own_uid(&base)?;
    Ok(base.join(format!("multicad-mcp-{uid}")).join("app.sock"))
}

/// Create or verify the socket's directory: owned by this user, mode 0700.
fn private_directory(socket: &Path) -> Result<(), String> {
    let directory = socket
        .parent()
        .ok_or("The MCP socket path has no directory")?;
    match std::fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("Cannot create {}: {e}", directory.display())),
    }
    let metadata = std::fs::symlink_metadata(directory).map_err(|e| e.to_string())?;
    let parent = directory.parent().unwrap_or(Path::new("/"));
    if !metadata.is_dir()
        || metadata.uid() != own_uid(parent)?
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(format!(
            "{} must be a directory owned by this user with mode 0700",
            directory.display()
        ));
    }
    Ok(())
}

pub struct Listener {
    runtime: Option<tokio::runtime::Runtime>,
    path: PathBuf,
}
impl Listener {
    /// Listen at `path`. `host` is called once per connection; `events`
    /// reports clients as they connect and leave.
    pub fn bind(
        path: PathBuf,
        host: impl Fn(u64) -> Arc<dyn Host> + Send + Sync + 'static,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self, String> {
        private_directory(&path)?;
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err("Another MultiCAD window already accepts MCP connections".into());
            }
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("multicad-mcp")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let listener = {
            let _guard = runtime.enter();
            tokio::net::UnixListener::bind(&path)
                .map_err(|e| format!("Cannot listen at {}: {e}", path.display()))?
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        let host = Arc::new(host);
        let events = Arc::new(events);
        runtime.spawn(async move {
            let next = AtomicU64::new(1);
            while let Ok((stream, _)) = listener.accept().await {
                let connection = next.fetch_add(1, Ordering::Relaxed);
                let server = Server::new(host(connection));
                let events = Arc::clone(&events);
                tokio::spawn(async move {
                    let Ok(service) = server.serve(stream).await else {
                        return;
                    };
                    let client = service
                        .peer()
                        .peer_info()
                        .map(|i| i.client_info.name.clone())
                        .unwrap_or_else(|| "unnamed".into());
                    events(Event::Connected { connection, client });
                    let _ = service.waiting().await;
                    events(Event::Disconnected { connection });
                });
            }
        });
        Ok(Self {
            runtime: Some(runtime),
            path,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Copy this process's stdin to the window and its replies to stdout until
/// either side closes.
pub fn bridge_stdio(path: &Path) -> Result<(), String> {
    let stream = UnixStream::connect(path).map_err(|e| {
        format!(
            "No MultiCAD window accepts MCP connections at {} ({e}). Enable \"Allow local MCP connections\" in Command and MCP history.",
            path.display()
        )
    })?;
    let mut upstream = stream.try_clone().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0; 16 * 1024];
        while let Ok(n) = stdin.read(&mut buffer) {
            if n == 0 || upstream.write_all(&buffer[..n]).is_err() {
                break;
            }
        }
        let _ = upstream.shutdown(std::net::Shutdown::Write);
    });
    let mut downstream = stream;
    let mut stdout = std::io::stdout().lock();
    let mut buffer = [0; 16 * 1024];
    loop {
        let n = downstream.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        stdout
            .write_all(&buffer[..n])
            .and_then(|()| stdout.flush())
            .map_err(|e| e.to_string())?;
    }
}
