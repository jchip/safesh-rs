//! `sys_traits` adapter so `deno_permissions` reads PATH, HOME and cwd from a [`HostEnv`] snapshot.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

use sys_traits::impls::RealSys;
use sys_traits::{
    BaseEnvVar, BaseFsCanonicalize, BaseFsMetadata, BaseFsReadDir, EnvCurrentDir, EnvHomeDir,
};

use crate::host::HostEnv;

/// Real filesystem, injected env.
#[derive(Debug, Clone)]
pub(crate) struct HostSys {
    home: PathBuf,
    path: Option<OsString>,
    cwd: PathBuf,
}

impl HostSys {
    pub(crate) fn new(host: &HostEnv, cwd: &Path) -> Self {
        Self {
            home: host.home.clone(),
            path: host.path.clone(),
            cwd: cwd.to_path_buf(),
        }
    }
}

impl BaseEnvVar for HostSys {
    fn base_env_var_os(&self, key: &OsStr) -> Option<OsString> {
        match key.to_str() {
            Some("PATH") => self.path.clone(),
            Some("HOME") => Some(self.home.clone().into_os_string()),
            _ => None,
        }
    }
}

impl EnvHomeDir for HostSys {
    fn env_home_dir(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }
}

impl EnvCurrentDir for HostSys {
    fn env_current_dir(&self) -> io::Result<PathBuf> {
        Ok(self.cwd.clone())
    }
}

impl BaseFsCanonicalize for HostSys {
    fn base_fs_canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        RealSys.base_fs_canonicalize(path)
    }
}

impl BaseFsMetadata for HostSys {
    type Metadata = <RealSys as BaseFsMetadata>::Metadata;

    fn base_fs_metadata(&self, path: &Path) -> io::Result<Self::Metadata> {
        RealSys.base_fs_metadata(path)
    }

    fn base_fs_symlink_metadata(&self, path: &Path) -> io::Result<Self::Metadata> {
        RealSys.base_fs_symlink_metadata(path)
    }
}

impl BaseFsReadDir for HostSys {
    type ReadDirEntry = <RealSys as BaseFsReadDir>::ReadDirEntry;

    fn base_fs_read_dir(
        &self,
        path: &Path,
    ) -> io::Result<Box<dyn Iterator<Item = io::Result<Self::ReadDirEntry>>>> {
        RealSys.base_fs_read_dir(path)
    }
}
