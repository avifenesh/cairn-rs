use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct OverlayProvider {
    pub base_dir: PathBuf,
}
