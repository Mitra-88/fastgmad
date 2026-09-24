#[cfg(feature = "binary")]
use std::ffi::OsString;
use std::{num::NonZeroUsize, path::PathBuf};

#[derive(Debug)]
pub struct ExtractGmaConfig {
	pub out: PathBuf,
	pub max_io_threads: NonZeroUsize,
	pub max_io_memory_usage: NonZeroUsize,
}

#[cfg(feature = "binary")]
pub enum ExtractGmadIn {
	Stdin,
	File(PathBuf),
}

#[cfg(feature = "binary")]
pub struct PrintHelp(pub Option<&'static str>);

const DEFAULT_MEMORY: NonZeroUsize = NonZeroUsize::new(1 << 28).expect("256 MiB is non-zero");

#[cfg(feature = "binary")]
fn leak(message: String) -> &'static str {
	Box::leak(message.into_boxed_str())
}

fn default_threads_for(available: usize) -> NonZeroUsize {
	let threads = available.saturating_sub(2).min(available * 3 / 4).clamp(1, 32);
	NonZeroUsize::new(threads).expect("clamped value is guaranteed to be >= 1")
}

fn get_default_threads() -> NonZeroUsize {
	default_threads_for(std::thread::available_parallelism().map(NonZeroUsize::get).unwrap_or(1))
}

impl Default for ExtractGmaConfig {
	fn default() -> Self {
		Self {
			out: PathBuf::new(),
			max_io_threads: get_default_threads(),
			max_io_memory_usage: DEFAULT_MEMORY,
		}
	}
}

#[cfg(feature = "binary")]
impl ExtractGmaConfig {
	pub fn from_args(mut args: impl Iterator<Item = OsString>) -> Result<(Self, ExtractGmadIn), PrintHelp> {
		let mut config = Self::default();
		let mut input = None;

		while let Some(arg) = args.next() {
			let arg = arg.to_str().ok_or(PrintHelp(Some("Non-UTF-8 argument")))?;
			match arg {
				"-max-io-threads" => {
					config.max_io_threads = args
						.next()
						.and_then(|v| v.to_str().and_then(|s| s.parse().ok()))
						.ok_or(PrintHelp(Some("Expected integer greater than zero for -max-io-threads")))?;
				}
				"-max-io-memory-usage" => {
					config.max_io_memory_usage = args
						.next()
						.and_then(|v| v.to_str().and_then(|s| s.parse().ok()))
						.ok_or(PrintHelp(Some("Expected integer greater than zero for -max-io-memory-usage")))?;
				}
				"-out" => {
					config.out = PathBuf::from(
						args.next()
							.filter(|p| !p.is_empty())
							.ok_or(PrintHelp(Some("Expected a value after -out")))?,
					);
				}
				"-stdin" => input = Some(ExtractGmadIn::Stdin),
				"-file" => {
					input = Some(ExtractGmadIn::File(
						args.next()
							.filter(|p| !p.is_empty())
							.map(PathBuf::from)
							.ok_or(PrintHelp(Some("Expected a value after -file")))?,
					));
				}
				_ => return Err(PrintHelp(Some(leak(format!("Unknown argument '{arg}'"))))),
			}
		}

		let input = input.ok_or(PrintHelp(Some("Missing -file (the addon you want to extract)")))?;

		if config.out.as_os_str().is_empty() {
			if let ExtractGmadIn::File(path) = &input {
				let mut dir = path.to_owned();
				dir.set_extension("");
				if dir.exists() && !dir.is_dir() {
					return Err(PrintHelp(Some(
						"Default output path exists as a file. Please specify an output folder with -out",
					)));
				}
				config.out = dir;
			} else {
				return Err(PrintHelp(Some("Missing -out (the folder to extract to)")));
			}
		}

		Ok((config, input))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_thread_count_stays_conservative() {
		assert_eq!(default_threads_for(1).get(), 1);
		assert_eq!(default_threads_for(2).get(), 1);
		assert_eq!(default_threads_for(3).get(), 1);
		assert_eq!(default_threads_for(4).get(), 2);
		assert_eq!(default_threads_for(8).get(), 6);
		assert_eq!(default_threads_for(16).get(), 12);
		assert_eq!(default_threads_for(128).get(), 32);

		let mut prev = 0;
		for cores in 1..=512 {
			let threads = default_threads_for(cores).get();
			assert!(threads >= 1, "{cores} cores produced {threads} threads");
			assert!(threads <= 32, "{cores} cores produced {threads} threads");
			assert!(threads <= cores.saturating_sub(2).max(1), "{cores} cores produced {threads} threads");
			assert!(threads >= prev, "thread count dropped from {prev} to {threads} at {cores} cores");
			prev = threads;
		}
	}
}
