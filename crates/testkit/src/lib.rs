//! Test doubles: fake MCP servers, fake SSO, fake Jira, fake Artifactory.

mod auth;
mod catalog;
mod confirm;
mod jira;
mod mock_mcp;
mod session;
mod sso;
mod trace;

pub use auth::{FakeAuthUi, FakeBasicServer, FakeWindow};
pub use catalog::{catalog_tools, test_tools};
pub use confirm::FakeConfirmationBroker;
pub use jira::{FakeJira, INVALID_JQL, JiraRequest, RAW_ERROR_MARKER};
pub use mock_mcp::{Generation, MockMcpBuilder, MockMcpServer, RecordedRequest};
pub use session::FakeSessionGate;
pub use sso::{ApiCall, FAKE_SSO_AUDIENCE, FAKE_SSO_COOKIE, FAKE_SSO_ISSUER, FakeSso, SsoToken};
pub use trace::MemoryTraceSink;
