use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use tempfile::TempDir;

use crate::{
    config::{Config, MAX_CONCURRENT_JOBS},
    fofa::{ApiMode, FofaClient, QueryValidator, RelayQuotaAuthManager, RetryPolicy},
    jobs::JobManager,
    searches::SearchStore,
};

pub struct AppState {
    pub config: Arc<Config>,
    pub fofa: Arc<FofaClient>,
    pub quota: Option<Arc<RelayQuotaAuthManager>>,
    pub validator: QueryValidator,
    pub jobs: Arc<JobManager>,
    pub searches: SearchStore,
    temp_dir: TempDir,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("cyberscope/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("创建 HTTP 客户端失败")?;

        let fofa = Arc::new(
            FofaClient::new(
                http.clone(),
                config.fofa_api_base_url.clone(),
                config.fofa_api_key.clone(),
                RetryPolicy::default(),
            )
            .context("创建 FOFA 客户端失败")?,
        );
        let quota = if config.relay_quota_enabled {
            Some(Arc::new(
                RelayQuotaAuthManager::new(
                    http,
                    config.fofa_api_base_url.clone(),
                    config.fofa_api_key.clone(),
                )
                .context("创建中转额度客户端失败")?,
            ))
        } else {
            None
        };
        let validator = QueryValidator::new(ApiMode::from_base_url(&config.fofa_api_base_url));
        let temp_dir = create_private_runtime_dir().context("创建私有临时目录失败")?;
        let searches = SearchStore::connect(&config.database_path)
            .await
            .context("初始化 SQLite 数据库失败")?;

        Ok(Self {
            config,
            fofa,
            quota,
            validator,
            jobs: JobManager::new(MAX_CONCURRENT_JOBS),
            searches,
            temp_dir,
        })
    }

    pub fn temp_root(&self) -> &Path {
        self.temp_dir.path()
    }
}

fn create_private_runtime_dir() -> std::io::Result<TempDir> {
    let base = std::env::temp_dir().join("cyberscope");
    ensure_private_runtime_root(&base)?;
    cleanup_stale_runs(&base)?;

    let run = tempfile::Builder::new().prefix("run-").tempdir_in(&base)?;
    set_private_directory_permissions(run.path())?;
    Ok(run)
}

fn ensure_private_runtime_root(base: &Path) -> std::io::Result<()> {
    match fs::create_dir(base) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }

    let metadata = fs::symlink_metadata(base)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "temporary root is not a real directory",
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "temporary root is owned by a different user",
            ));
        }
    }
    set_private_directory_permissions(base)
}

fn cleanup_stale_runs(base: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let path: PathBuf = entry.path();
        let is_owned_run = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("run-"));
        if !is_owned_run {
            continue;
        }

        let metadata = fs::symlink_metadata(&path);
        let is_real_directory = metadata.as_ref().is_ok_and(|metadata| {
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink()
        });
        if !is_real_directory {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "unexpected entry in private temporary root",
            ));
        }
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/state.rs"]
mod tests;
