//! Concrete `HarnessTool` implementations — one unit struct per upstream tool.

mod bash;
mod glob;
mod grep;
mod read;
mod webfetch;
mod write;

pub use bash::{HarnessBash, HarnessBashKill, HarnessBashOutput};
pub use glob::HarnessGlob;
pub use grep::HarnessGrep;
pub use read::HarnessRead;
pub use webfetch::HarnessWebFetch;
pub use write::{HarnessEdit, HarnessMultiEdit, HarnessWrite};
