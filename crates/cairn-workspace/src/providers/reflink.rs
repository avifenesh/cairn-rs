use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ReflinkProvider {
    pub base_dir: PathBuf,
}
