//! Frontend-independent application services shared by the native desktop and terminal UI.
//! Network, credential, Git, and Claude operations run through the same injected executor.
pub mod backend;
pub mod demo;
pub mod projects;
pub mod settings;
