//! Small helpers for decoding vivo's KDL config from a `kdl::KdlDocument`.
//!
//! The config structs decode themselves with these, so unknown nodes, repeated
//! singular nodes, missing required nodes and wrongly-typed values are all
//! reported with the file, line and column of the offending node.

use std::fmt;

use kdl::{KdlDocument, KdlNode, KdlValue};

/// All nodes in `nodes` called `name`.
pub(crate) fn named<'n>(nodes: &'n [KdlNode], name: &'n str) -> impl Iterator<Item = &'n KdlNode> {
    nodes.iter().filter(move |n| n.name().value() == name)
}

/// Where a config is being decoded from, for error messages.
pub(crate) struct Source<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

#[derive(Debug)]
pub struct ConfigError {
    message: String,
    location: Option<(usize, usize)>,
    path: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.location {
            Some((line, col)) => write!(f, "{}:{line}:{col}: {}", self.path, self.message),
            None => write!(f, "{}: {}", self.path, self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

impl<'a> Source<'a> {
    /// `offset` is in chars, matching `kdl`'s spans.
    fn error_at(&self, offset: usize, message: impl Into<String>) -> ConfigError {
        let before: Vec<char> = self.text.chars().take(offset).collect();
        let line = before.iter().filter(|&&c| c == '\n').count() + 1;
        let col = before.iter().rev().take_while(|&&c| c != '\n').count() + 1;
        ConfigError {
            message: message.into(),
            location: Some((line, col)),
            path: self.path.to_string(),
        }
    }

    /// An error about the file as a whole (no line to point at).
    pub(crate) fn file_error(&self, message: impl Into<String>) -> ConfigError {
        ConfigError {
            message: message.into(),
            location: None,
            path: self.path.to_string(),
        }
    }

    pub(crate) fn error(&self, node: &KdlNode, message: impl Into<String>) -> ConfigError {
        self.error_at(node.span().offset(), message)
    }

    /// Parses KDL text (v2, falling back to v1 for older config files).
    pub(crate) fn parse_document(&self) -> Result<KdlDocument, ConfigError> {
        KdlDocument::parse(self.text).map_err(|e| match e.diagnostics.first() {
            Some(d) => self.error_at(
                d.span.offset(),
                d.message
                    .clone()
                    .unwrap_or_else(|| "invalid KDL".to_string()),
            ),
            None => ConfigError {
                message: "invalid KDL".to_string(),
                location: None,
                path: self.path.to_string(),
            },
        })
    }

    /// The child nodes of `node` (empty if it has no `{ }` block).
    pub(crate) fn children<'n>(&self, node: &'n KdlNode) -> &'n [KdlNode] {
        node.children().map(KdlDocument::nodes).unwrap_or(&[])
    }

    /// Errors on any node in `nodes` whose name isn't in `known`.
    pub(crate) fn only(&self, nodes: &[KdlNode], known: &[&str]) -> Result<(), ConfigError> {
        match nodes.iter().find(|n| !known.contains(&n.name().value())) {
            Some(n) => Err(self.error(
                n,
                format!(
                    "unexpected node '{}' (expected one of: {})",
                    n.name().value(),
                    known.join(", ")
                ),
            )),
            None => Ok(()),
        }
    }

    /// The single node called `name`, if present. Errors if it appears twice.
    pub(crate) fn optional<'n>(
        &self,
        nodes: &'n [KdlNode],
        name: &str,
    ) -> Result<Option<&'n KdlNode>, ConfigError> {
        let mut found = nodes.iter().filter(|n| n.name().value() == name);
        let first = found.next();
        match found.next() {
            Some(dup) => Err(self.error(dup, format!("duplicate '{name}' node"))),
            None => Ok(first),
        }
    }

    /// The single node called `name`; `parent` locates the error if it's missing.
    pub(crate) fn required<'n>(
        &self,
        parent: &KdlNode,
        nodes: &'n [KdlNode],
        name: &str,
    ) -> Result<&'n KdlNode, ConfigError> {
        self.optional(nodes, name)?
            .ok_or_else(|| self.error(parent, format!("missing required '{name}' node")))
    }

    /// The value of a node written as `name <value>` with nothing else on it.
    fn single_value<'n>(&self, node: &'n KdlNode) -> Result<&'n KdlValue, ConfigError> {
        let name = node.name().value();
        if node.entries().iter().any(|e| e.name().is_some()) {
            return Err(self.error(node, format!("'{name}' does not take properties")));
        }
        if node.children().is_some() {
            return Err(self.error(node, format!("'{name}' does not take a child block")));
        }
        match node.entries() {
            [only] => Ok(only.value()),
            [] => Err(self.error(node, format!("'{name}' needs a value"))),
            _ => Err(self.error(node, format!("'{name}' takes exactly one value"))),
        }
    }

    pub(crate) fn string_value(&self, node: &KdlNode) -> Result<String, ConfigError> {
        match self.single_value(node)? {
            KdlValue::String(s) => Ok(s.clone()),
            _ => Err(self.error(node, format!("'{}' must be a string", node.name().value()))),
        }
    }

    pub(crate) fn bool_value(&self, node: &KdlNode) -> Result<bool, ConfigError> {
        match self.single_value(node)? {
            KdlValue::Bool(b) => Ok(*b),
            _ => Err(self.error(
                node,
                format!("'{}' must be true or false", node.name().value()),
            )),
        }
    }

    pub(crate) fn int_value<T: TryFrom<i128>>(&self, node: &KdlNode) -> Result<T, ConfigError> {
        let name = node.name().value();
        match self.single_value(node)? {
            KdlValue::Integer(i) => {
                T::try_from(*i).map_err(|_| self.error(node, format!("'{name}' is out of range")))
            }
            _ => Err(self.error(node, format!("'{name}' must be a whole number"))),
        }
    }

    /// The one positional string argument of a node like `task "name" { .. }`.
    /// Properties are rejected; the `{ }` block is left for the caller.
    pub(crate) fn argument(&self, node: &KdlNode) -> Result<String, ConfigError> {
        let name = node.name().value();
        if node.entries().iter().any(|e| e.name().is_some()) {
            return Err(self.error(node, format!("'{name}' does not take properties")));
        }
        match node.entries() {
            [only] => match only.value() {
                KdlValue::String(s) => Ok(s.clone()),
                _ => Err(self.error(node, format!("'{name}' argument must be a string"))),
            },
            [] => Err(self.error(node, format!("'{name}' needs an argument"))),
            _ => Err(self.error(node, format!("'{name}' takes exactly one argument"))),
        }
    }

    /// Like [`argument`](Self::argument), also rejecting a `{ }` block.
    pub(crate) fn leaf_argument(&self, node: &KdlNode) -> Result<String, ConfigError> {
        if node.children().is_some() {
            return Err(self.error(
                node,
                format!("'{}' does not take a child block", node.name().value()),
            ));
        }
        self.argument(node)
    }
}
