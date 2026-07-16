use super::{binary, Host};
use crate::platform::Architecture;
use anyhow::Result;

pub use super::binary::GithubRepository;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectPackageFormat {
    Deb,
    AppImage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectPackageMode {
    EnsurePresent,
    Update,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPackageSelector(binary::BinaryPackageSelector);

impl DirectPackageSelector {
    pub fn new(include: impl Into<String>, excludes: Vec<String>) -> Result<Self> {
        Ok(Self(binary::BinaryPackageSelector::new(include, excludes)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectPackageOperation(binary::BinaryPackageOperation);

impl DirectPackageOperation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        format: DirectPackageFormat,
        provides: Vec<String>,
        repository: GithubRepository,
        architecture: Architecture,
        selector: DirectPackageSelector,
        mode: DirectPackageMode,
    ) -> Result<Self> {
        Ok(Self(binary::BinaryPackageOperation::new(
            name,
            match format {
                DirectPackageFormat::Deb => binary::BinaryPackageFormat::Deb,
                DirectPackageFormat::AppImage => binary::BinaryPackageFormat::AppImage,
            },
            provides,
            architecture,
            binary::BinarySourceOperation::GithubLatest {
                repository,
                selector: selector.0,
                sha256: None,
            },
            match mode {
                DirectPackageMode::EnsurePresent => binary::BinaryPackageMode::EnsurePresent,
                DirectPackageMode::Update => binary::BinaryPackageMode::Update,
            },
        )?))
    }

    pub(crate) fn display_args(&self) -> Vec<String> {
        self.0.display_args()
    }
}

pub(crate) fn execute(host: &Host<'_>, package: &DirectPackageOperation) -> Result<()> {
    binary::execute(host, &package.0)
}
