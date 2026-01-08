//! Document state and parsing logic
//!
//! This module handles document state management and dependency parsing
//! for different file types.

use crate::file_types::FileType;
use crate::parsers::Dependency;

pub struct DocumentState {
    pub dependencies: Vec<Dependency>,
    pub file_type: FileType,
}
