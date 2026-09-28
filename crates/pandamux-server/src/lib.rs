pub mod attachment;
pub mod checkpoint;
pub mod driver_registry;
pub mod environment_router;
pub mod fs;
pub mod git;
pub mod mcp_server;
pub mod router;
pub mod runtime;
pub mod server;
pub mod store;
pub mod terminal;
pub mod thread_manager;
pub mod worktree;

pub use attachment::AttachmentManager;
pub use checkpoint::{
    ChangedFileStat, CheckpointPhase, compute_changed_files, create_checkpoint, is_git_repository,
    prune_old_checkpoints, prune_thread_checkpoints, rollback_to_checkpoint,
};
pub use driver_registry::DriverRegistry;
pub use environment_router::{EnvironmentRouter, EnvironmentTransport, MockEnvironmentTransport};
pub use git::{get_git_status, git_commit, git_create_pr, git_push};
pub use mcp_server::McpServer;
pub use router::Router;
pub use runtime::{RUNTIME_FILENAME, RuntimeInfo};
pub use server::{Server, ServerConfig};
pub use store::{Store, StoreError};
pub use terminal::TerminalServerManager;
pub use thread_manager::ThreadManager;

#[cfg(test)]
mod tests {
    use super::*;
    use pandamux_protocol::ServerRole;

    #[tokio::test]
    async fn server_lifecycle_and_graceful_drain() {
        let temp_dir = std::env::temp_dir().join(format!("pandamux_test_{}", uuid::Uuid::new_v4()));
        let config = ServerConfig {
            role: ServerRole::Hub,
            environment_id: "env-local".to_string(),
            runtime_dir: temp_dir.clone(),
            db_path: None,
        };

        let server = Server::new(config).expect("create server");

        // Test request processing
        let ping_line = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"system.ping\"}\n";
        let res_line = server.process_line(ping_line).await.expect("process ping");
        assert!(res_line.contains("\"result\":{\"pong\":true"));

        // Register runtime file
        let runtime_path = server
            .register_runtime("\\\\.\\pipe\\pandamux-test", "secret-token")
            .expect("register runtime");
        assert!(runtime_path.exists());

        let read_info = RuntimeInfo::read_from_dir(&temp_dir)
            .expect("read runtime")
            .expect("info present");
        assert_eq!(read_info.token, "secret-token");
        assert_eq!(read_info.role, "hub");

        // Provable graceful shutdown and cleanup
        server.shutdown();
        server.cleanup_runtime();

        assert!(!runtime_path.exists());
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
