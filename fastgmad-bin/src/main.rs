use fastgmad::error::FastGmadError;
use fastgmad::extract::{ExtractGmaConfig, ExtractGmadIn, ExtractSummary};
use std::{
	io::{BufReader, IsTerminal, Write},
	path::Path,
	time::Instant,
};

fn main() {
	init_logger();

	match bin() {
		Ok(summary) => {
			log::info!("Finished");
			let mut stdout = std::io::stdout();
			let piped = !stdout.is_terminal();
			print_summary(&summary, &mut stdout, piped);
			std::process::exit(0);
		}
		Err(FastGmadBinError::FastGmadError(err)) => {
			eprintln!();
			log::error!("{err}\n");
			std::process::exit(1);
		}
		Err(FastGmadBinError::PrintHelp(None)) => {
			eprintln!("{}", include_str!("usage.txt"));
			std::process::exit(2);
		}
		Err(FastGmadBinError::PrintHelp(Some(msg))) => {
			log::error!("{msg}\n");
			eprintln!("{}", include_str!("usage.txt"));
			std::process::exit(2);
		}
	}
}

fn print_summary(summary: &ExtractSummary, out: &mut impl Write, piped: bool) {
	if !piped {
		return;
	}
	let _ = writeln!(out, "{}", summary.to_json());
	let _ = out.flush();
}

fn init_logger() {
	struct Logger(Instant);
	impl log::Log for Logger {
		fn enabled(&self, metadata: &log::Metadata) -> bool {
			metadata.level() <= log::Level::Info
		}
		fn log(&self, record: &log::Record) {
			if record.level() == log::Level::Info {
				eprintln!("[+{:?}] {}", self.0.elapsed(), record.args());
			} else {
				eprintln!("{}{}", record.level().as_str().chars().next().unwrap_or(' '), record.args());
			}
		}
		fn flush(&self) {
			let _ = std::io::stderr().lock().flush();
		}
	}

	log::set_logger(Box::leak(Box::new(Logger(Instant::now())))).unwrap();
	log::set_max_level(log::LevelFilter::Info);
}

fn bin() -> Result<ExtractSummary, FastGmadBinError> {
	let mut args = std::env::args_os().skip(1);
	let cmd = args.next().ok_or(FastGmadBinError::PrintHelp(None))?;
	let path = Path::new(&cmd);

	if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("gma")) {
		let conf = ExtractGmaConfig {
			out: path.with_extension(""),
			..Default::default()
		};
		extract(conf, ExtractGmadIn::File(cmd.into()))
	} else if cmd.to_str() == Some("extract") {
		let (conf, input) = ExtractGmaConfig::from_args(args).map_err(|h| FastGmadBinError::PrintHelp(h.0))?;
		extract(conf, input)
	} else {
		Err(FastGmadBinError::PrintHelp(Some("Unknown command")))
	}
}

fn extract(conf: ExtractGmaConfig, input: ExtractGmadIn) -> Result<ExtractSummary, FastGmadBinError> {
	let summary = match input {
		ExtractGmadIn::File(path) => fastgmad::extract::extract_gma_file(&conf, &path)?,
		ExtractGmadIn::Stdin => {
			let mut r = BufReader::with_capacity(1 << 20, std::io::stdin().lock());
			fastgmad::extract::extract_gma(&conf, &mut r)?
		}
	};
	Ok(summary)
}

enum FastGmadBinError {
	FastGmadError(FastGmadError),
	PrintHelp(Option<&'static str>),
}

impl From<FastGmadError> for FastGmadBinError {
	fn from(e: FastGmadError) -> Self {
		Self::FastGmadError(e)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn summary() -> ExtractSummary {
		ExtractSummary {
			v: 1,
			entries: 2,
			bad_names: 0,
			crc_checked: 1,
			crc_mismatch: 0,
		}
	}

	#[test]
	fn status_line_is_printed_only_when_piped() {
		let mut piped = Vec::new();
		print_summary(&summary(), &mut piped, true);
		assert_eq!(
			String::from_utf8(piped).unwrap(),
			"{\"v\":1,\"entries\":2,\"bad_names\":0,\"crc_checked\":1,\"crc_mismatch\":0}\n"
		);

		let mut terminal = Vec::new();
		print_summary(&summary(), &mut terminal, false);
		assert!(terminal.is_empty());
	}
}
