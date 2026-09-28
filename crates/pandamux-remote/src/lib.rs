pub mod bootstrap;
pub mod config;
pub mod exec;
pub mod pool;
pub mod proxy;
pub mod sftp;
pub mod worktree_sync;

pub use bootstrap::{
    DaemonOwnership, InMemoryBinarySource, RemoteBinarySource, RemoteBootstrapResult,
    RemoteServerRuntime, bootstrap_remote_environment, detect_remote_platform,
    install_remote_binaries_if_needed, start_or_discover_daemon, teardown_remote_daemon,
};
pub use config::{SshAuth, SshConfig, SshErrorCategory, SshFailure};
pub use exec::{ExecOutput, ExecStreamMsg, execute_remote_command, execute_remote_command_stream};
pub use pool::{ClientHandler, HostKey, SshConnectionPool, connect_client, connect_client_stream};
pub use proxy::{ProxyTunnel, open_exec_proxy_tunnel, open_streamlocal_proxy_tunnel};
pub use sftp::{
    RemoteFolderEntry, RemoteFolderListing, browse_remote_folders, read_remote_file,
    upload_bytes_sftp, upload_file_sftp,
};
pub use worktree_sync::{apply_worktree_bundle, create_worktree_bundle};
