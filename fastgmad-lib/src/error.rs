use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct FastGmadError {
	pub kind: FastGmadErrorKind,
	pub context: Option<String>,
}

impl FastGmadError {
	pub fn io(error: std::io::Error, context: &str, path: Option<&Path>) -> Self {
		let kind = match path {
			Some(p) => FastGmadErrorKind::PathIoError {
				path: p.to_path_buf(),
				error,
			},
			None => FastGmadErrorKind::IoError(error),
		};
		Self {
			kind,
			context: Some(context.to_string()),
		}
	}
}

impl std::fmt::Display for FastGmadError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		if let Some(context) = &self.context {
			write!(f, "{} while {}", self.kind, context)
		} else {
			write!(f, "{}", self.kind)
		}
	}
}

impl std::error::Error for FastGmadError {}

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
pub enum FastGmadErrorKind {
	#[error("I/O error ({0})")]
	IoError(std::io::Error),

	#[error("I/O error ({error}) (from path \"{path}\")")]
	PathIoError { path: PathBuf, error: std::io::Error },

	#[error("invalid GMA file ({0})")]
	InvalidGma(String),
}
