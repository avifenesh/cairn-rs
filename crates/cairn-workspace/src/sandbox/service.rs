use std::collections::HashMap;
use std::sync::Arc;

use crate::providers::SandboxProvider;
use crate::sandbox::SandboxStrategy;

pub struct SandboxService {
    pub providers: HashMap<SandboxStrategy, Arc<dyn SandboxProvider>>,
}
