//! Test doubles: fake MCP servers, fake SSO, fake Artifactory.

mod catalog;
mod confirm;
mod mock_mcp;
mod session;

pub use catalog::{catalog_tools, test_tools};
pub use confirm::FakeConfirmationBroker;
pub use mock_mcp::{Generation, MockMcpBuilder, MockMcpServer, RecordedRequest};
pub use session::FakeSessionGate;
