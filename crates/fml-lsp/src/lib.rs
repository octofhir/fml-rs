//! Language server for the [FHIR Mapping Language](https://build.fhir.org/ig/HL7/mapping-language-ig/).
//!
//! - diagnostics: syntax, compile and type problems as you type;
//! - completion: elements after `.` (from the type of the variable, followed through the path),
//!   variables in scope, groups after `then`, transforms after `=`, types after `:`, keywords;
//! - hover: the type of a variable or an element, a group's signature, a transform's description;
//! - go to definition, find references and rename for variables and groups;
//! - signature help for transforms and group calls;
//! - the outline (document symbols), formatting, and semantic tokens for colouring.
//!
//! [`Server`] does no I/O: it takes JSON-RPC messages and returns what to send back, so a host
//! can run it over standard input and output ([`run_stdio`]) or over a WebSocket.
//!
//! ```
//! # #[cfg(feature = "fhirschema")] {
//! use fml_lsp::Server;
//! use serde_json::json;
//!
//! let mut server = Server::with_release(fml_check::Release::R5);
//! server.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}}));
//! let replies = server.handle(&json!({
//!     "jsonrpc": "2.0",
//!     "method": "textDocument/didOpen",
//!     "params": {"textDocument": {"uri": "file:///map.fml", "languageId": "fml", "version": 1, "text": "group G(source src : Patient) {\n  src.nmae -> src.name;\n}\n"}}
//! }));
//! assert_eq!(replies[0]["method"], "textDocument/publishDiagnostics");
//! # }
//! ```

mod completion;
mod document;
mod hover;
mod navigate;
mod reference;
mod semantic;
mod server;
mod signature;
mod stdio;
mod symbols;

pub use lsp_types;
pub use server::{Server, SharedDefinitions};
pub use stdio::{run, run_stdio};
