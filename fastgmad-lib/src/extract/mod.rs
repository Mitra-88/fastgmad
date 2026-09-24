use crate::{
	GMA_MAGIC, GMA_VERSION,
	error::{FastGmadError, FastGmadErrorKind},
	util::{BufReadEx, ReadSkip},
};
use std::{
	borrow::Cow,
	collections::{HashSet, VecDeque},
	fs::File,
	io::{BufRead, BufReader, BufWriter, Read, Seek, Write},
	path::{Component, Path, PathBuf},
	sync::{
		Condvar, Mutex,
		atomic::{AtomicBool, AtomicUsize, Ordering},
	},
};

mod conf;
pub use conf::ExtractGmaConfig;
#[cfg(feature = "binary")]
pub use conf::{ExtractGmadIn, PrintHelp};

const COPY_CHUNK: u64 = 1 << 20;

fn gma_err(message: impl Into<String>) -> FastGmadError {
	FastGmadError {
		kind: FastGmadErrorKind::InvalidGma(message.into()),
		context: None,
	}
}

fn map_io<T>(res: std::io::Result<T>, context: &str) -> Result<T, FastGmadError> {
	res.map_err(|e| {
		if e.kind() == std::io::ErrorKind::UnexpectedEof {
			gma_err(format!("{context}: unexpected end of file, GMA is truncated"))
		} else {
			FastGmadError::io(e, context, None)
		}
	})
}

fn entry_truncated(e: std::io::Error, path: &Path) -> FastGmadError {
	if e.kind() == std::io::ErrorKind::UnexpectedEof {
		gma_err(format!("GMA is truncated, entry \"{}\" is cut short", path.display()))
	} else {
		FastGmadError::io(e, "reading GMA entry data", Some(path))
	}
}

fn finish_failed_writes(failed: usize) -> Result<(), FastGmadError> {
	if failed == 0 {
		return Ok(());
	}
	Err(FastGmadError::io(
		std::io::Error::other(format!("{failed} files failed to write")),
		"extracting GMA entries",
		None,
	))
}

fn human_bytes(bytes: u64) -> String {
	let (value, unit) = match bytes {
		b if b >= 1 << 30 => (b as f64 / (1 << 30) as f64, "GiB"),
		b if b >= 1 << 20 => (b as f64 / (1 << 20) as f64, "MiB"),
		b if b >= 1 << 10 => (b as f64 / (1 << 10) as f64, "KiB"),
		b => (b as f64, "bytes"),
	};
	format!("{value:.2} {unit}")
}

fn read_u32_le(r: &mut impl BufRead) -> std::io::Result<u32> {
	let mut buf = [0u8; 4];
	r.read_exact(&mut buf)?;
	Ok(u32::from_le_bytes(buf))
}

fn read_u64_le(r: &mut impl BufRead) -> std::io::Result<u64> {
	let mut buf = [0u8; 8];
	r.read_exact(&mut buf)?;
	Ok(u64::from_le_bytes(buf))
}

fn read_header(r: &mut impl BufRead) -> Result<(Vec<u8>, Vec<u8>), FastGmadError> {
	log::debug!("Reading metadata...");

	let mut magic = [0u8; 4];
	map_io(r.read_exact(&mut magic), "reading GMA magic")?;
	if magic != GMA_MAGIC {
		return Err(gma_err("bad magic bytes, this is not a GMA file"));
	}

	let mut version_buf = [0u8; 1];
	map_io(r.read_exact(&mut version_buf), "reading GMA version")?;
	let version = version_buf[0];
	if version > GMA_VERSION {
		return Err(gma_err(format!(
			"unsupported GMA version {version}, this tool supports up to version {GMA_VERSION}"
		)));
	}
	if version != GMA_VERSION {
		log::warn!("GMA is version {version} instead of {GMA_VERSION}, reading anyway");
	}

	map_io(r.skip(16), "reading SteamID and timestamp")?;

	let mut buf = Vec::new();
	if version > 1 {
		loop {
			let content = map_io(r.read_nul_str(&mut buf), "reading required content")?;
			if content.is_empty() {
				break;
			}
		}
	}

	let title = map_io(r.read_nul_str(&mut buf), "reading addon name")?.to_vec();
	let addon_json = map_io(r.read_nul_str(&mut buf), "reading addon description")?.to_vec();
	map_io(r.skip_nul_str(), "reading addon author")?;
	map_io(r.skip(4), "reading addon version")?;

	Ok((title, addon_json))
}

fn write_addon_json(conf: &ExtractGmaConfig, title: &[u8], addon_json: &[u8]) -> Result<(), FastGmadError> {
	log::debug!("Writing addon.json...");
	let path = conf.out.join("addon.json");
	let mut f = BufWriter::new(File::create(&path).map_err(|e| FastGmadError::io(e, "creating addon.json", Some(&path)))?);

	let res = if let Ok(mut kv) = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(addon_json) {
		kv.entry("title".to_string())
			.or_insert_with(|| serde_json::Value::String(String::from_utf8_lossy(title).into_owned()));
		serde_json::to_writer_pretty(&mut f, &kv)
	} else {
		serde_json::to_writer_pretty(
			&mut f,
			&StubAddonJson {
				title: String::from_utf8_lossy(title),
				description: String::from_utf8_lossy(addon_json),
			},
		)
	};
	res.map_err(|e| FastGmadError::io(std::io::Error::from(e), "writing addon.json", Some(&path)))?;
	f.flush().map_err(|e| FastGmadError::io(e, "writing addon.json", Some(&path)))?;
	Ok(())
}

#[derive(serde::Serialize)]
struct StubAddonJson<'a> {
	title: Cow<'a, str>,
	description: Cow<'a, str>,
}

fn read_index(conf: &ExtractGmaConfig, r: &mut impl BufRead) -> Result<Vec<GmaEntry>, FastGmadError> {
	log::debug!("Reading file list...");

	let mut entries = Vec::new();
	let mut buf = Vec::new();
	let mut offset = 0u64;
	let mut bad_names = 0u64;
	let mut seen: HashSet<String> = HashSet::new();

	loop {
		let file_id = map_io(read_u32_le(r), "reading file index")?;
		if file_id == 0 {
			break;
		}
		let name = map_io(r.read_nul_str(&mut buf), "reading file index")?.to_vec();
		let size = map_io(read_u64_le(r), "reading file index")?;
		map_io(r.skip(4), "reading file index")?;

		let entry_offset = offset;
		offset = offset.checked_add(size).ok_or_else(|| gma_err("file index is too large"))?;

		entries.push(GmaEntry {
			path: entry_path(&conf.out, &name, &mut bad_names, &mut seen),
			offset: entry_offset,
			size,
		});
	}

	Ok(entries)
}

fn entry_path(out: &Path, raw: &[u8], bad_names: &mut u64, seen: &mut HashSet<String>) -> PathBuf {
	let Ok(name) = std::str::from_utf8(raw) else {
		return bad_path(out, bad_names, "is not valid UTF-8", raw);
	};
	if name.is_empty() {
		return bad_path(out, bad_names, "is empty", raw);
	}
	let path = Path::new(name);
	if path
		.components()
		.any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
	{
		return bad_path(out, bad_names, "escapes the output folder", raw);
	}
	#[cfg(windows)]
	if is_windows_device_name(path.file_name().and_then(|n| n.to_str())) {
		return bad_path(out, bad_names, "is a reserved Windows device name", raw);
	}
	let key = if cfg!(any(windows, target_os = "macos")) {
		name.to_lowercase()
	} else {
		name.to_string()
	};
	if !seen.insert(key) {
		return bad_path(out, bad_names, "is a duplicate entry", raw);
	}
	out.join(path)
}

fn bad_path(out: &Path, bad_names: &mut u64, reason: &str, raw: &[u8]) -> PathBuf {
	let path = out.join("badnames").join(format!("{bad_names}.unk"));
	log::warn!(
		"Entry name '{}' {reason}, writing to badnames/{bad_names}.unk",
		String::from_utf8_lossy(raw)
	);
	*bad_names += 1;
	path
}

#[cfg(windows)]
fn is_windows_device_name(name: Option<&str>) -> bool {
	let Some(name) = name else { return false };
	let stem = name.split('.').next().unwrap_or(name);
	matches!(
		stem.to_ascii_uppercase().as_str(),
		"CON"
			| "PRN" | "AUX"
			| "NUL" | "COM1"
			| "COM2" | "COM3"
			| "COM4" | "COM5"
			| "COM6" | "COM7"
			| "COM8" | "COM9"
			| "LPT1" | "LPT2"
			| "LPT3" | "LPT4"
			| "LPT5" | "LPT6"
			| "LPT7" | "LPT8"
			| "LPT9"
	)
}

fn create_output_dirs(entries: &[GmaEntry]) -> Result<(), FastGmadError> {
	let mut dirs: HashSet<&Path> = HashSet::new();
	for entry in entries {
		if let Some(parent) = entry.path.parent() {
			dirs.insert(parent);
		}
	}
	for dir in dirs {
		std::fs::create_dir_all(dir).map_err(|e| FastGmadError::io(e, "creating directory", Some(dir)))?;
	}
	Ok(())
}

struct GmaEntry {
	path: PathBuf,
	offset: u64,
	size: u64,
}

pub fn extract_gma(conf: &ExtractGmaConfig, r: &mut impl BufRead) -> Result<(), FastGmadError> {
	let (title, addon_json) = read_header(r)?;
	std::fs::create_dir_all(&conf.out).map_err(|e| FastGmadError::io(e, "creating output directory", Some(&conf.out)))?;
	write_addon_json(conf, &title, &addon_json)?;
	let entries = read_index(conf, r)?;
	create_output_dirs(&entries)?;

	if conf.max_io_threads.get() == 1 {
		write_entries_sequential(r, &entries)?;
	} else {
		write_entries_parallel(conf, r, &entries)?;
	}

	log::info!("Extracted {} entries ({})", entries.len(), human_bytes(total_size(&entries)));
	Ok(())
}

fn total_size(entries: &[GmaEntry]) -> u64 {
	entries.iter().map(|e| e.size).sum()
}

pub fn extract_gma_file(conf: &ExtractGmaConfig, path: &Path) -> Result<(), FastGmadError> {
	let file = File::open(path).map_err(|e| FastGmadError::io(e, "opening GMA file", Some(path)))?;
	let mut r = BufReader::with_capacity(1 << 20, file);

	let (title, addon_json) = read_header(&mut r)?;
	std::fs::create_dir_all(&conf.out).map_err(|e| FastGmadError::io(e, "creating output directory", Some(&conf.out)))?;
	write_addon_json(conf, &title, &addon_json)?;
	let mut entries = read_index(conf, &mut r)?;
	let data_start = r
		.stream_position()
		.map_err(|e| FastGmadError::io(e, "locating GMA data block", Some(path)))?;
	drop(r);

	for entry in &mut entries {
		entry.offset = entry
			.offset
			.checked_add(data_start)
			.ok_or_else(|| gma_err("GMA file index is too large"))?;
	}
	create_output_dirs(&entries)?;
	extract_entries_parallel(conf, path, &entries)?;

	log::info!("Extracted {} entries ({})", entries.len(), human_bytes(total_size(&entries)));
	Ok(())
}

fn write_entries_sequential(r: &mut impl BufRead, entries: &[GmaEntry]) -> Result<(), FastGmadError> {
	let mut failed_writes = 0usize;
	let mut scratch = Vec::new();
	for entry in entries {
		write_entry_streaming(r, &entry.path, entry.size, &mut scratch, &mut failed_writes)?;
	}
	finish_failed_writes(failed_writes)
}

fn write_entry_streaming(
	r: &mut impl BufRead,
	path: &Path,
	size: u64,
	scratch: &mut Vec<u8>,
	failed_writes: &mut usize,
) -> Result<(), FastGmadError> {
	let mut file = match File::create(path) {
		Ok(f) => BufWriter::with_capacity(size.min(COPY_CHUNK) as usize, f),
		Err(e) => {
			log::warn!("Failed to create \"{}\": {}", path.display(), e);
			*failed_writes += 1;
			r.skip(size).map_err(|e| entry_truncated(e, path))?;
			return Ok(());
		}
	};

	let mut remaining = size;
	while remaining > 0 {
		let chunk = remaining.min(COPY_CHUNK) as usize;
		if scratch.len() < chunk {
			scratch.resize(chunk, 0);
		}
		r.read_exact(&mut scratch[..chunk]).map_err(|e| entry_truncated(e, path))?;
		remaining -= chunk as u64;
		if let Err(e) = file.write_all(&scratch[..chunk]) {
			log::warn!("Failed to write \"{}\": {}", path.display(), e);
			*failed_writes += 1;
			r.skip(remaining).map_err(|e| entry_truncated(e, path))?;
			return Ok(());
		}
	}

	if let Err(e) = file.flush() {
		log::warn!("Failed to write \"{}\": {}", path.display(), e);
		*failed_writes += 1;
	}
	Ok(())
}

fn write_entries_parallel(conf: &ExtractGmaConfig, r: &mut impl BufRead, entries: &[GmaEntry]) -> Result<(), FastGmadError> {
	struct State {
		queue: VecDeque<(PathBuf, Vec<u8>)>,
		mem_used: usize,
		failed_writes: usize,
		producer_done: bool,
	}

	let state = Mutex::new(State {
		queue: VecDeque::new(),
		mem_used: 0,
		failed_writes: 0,
		producer_done: false,
	});
	let queue_cv = Condvar::new();
	let mem_cv = Condvar::new();

	struct ProducerGuard<'a> {
		state: &'a Mutex<State>,
		queue_cv: &'a Condvar,
		mem_cv: &'a Condvar,
	}
	impl Drop for ProducerGuard<'_> {
		fn drop(&mut self) {
			let mut s = self.state.lock().unwrap();
			s.producer_done = true;
			drop(s);
			self.queue_cv.notify_all();
			self.mem_cv.notify_all();
		}
	}

	let mut inline_failed_writes = 0usize;
	let mut scratch = Vec::new();

	std::thread::scope(|s| {
		for _ in 0..conf.max_io_threads.get() {
			s.spawn(|| {
				loop {
					let (path, buf) = {
						let mut s = state.lock().unwrap();
						loop {
							if s.producer_done && s.queue.is_empty() {
								return;
							}
							if let Some(item) = s.queue.pop_front() {
								break item;
							}
							s = queue_cv.wait(s).unwrap();
						}
					};

					let res = std::fs::write(&path, &buf);
					let mut s = state.lock().unwrap();
					s.mem_used -= buf.len();
					if let Err(e) = res {
						s.failed_writes += 1;
						log::warn!("Failed to write \"{}\": {}", path.display(), e);
					}
					drop(s);
					mem_cv.notify_one();
				}
			});
		}

		let _guard = ProducerGuard {
			state: &state,
			queue_cv: &queue_cv,
			mem_cv: &mem_cv,
		};

		for entry in entries {
			if entry.size > conf.max_io_memory_usage.get() as u64 {
				write_entry_streaming(r, &entry.path, entry.size, &mut scratch, &mut inline_failed_writes)?;
				continue;
			}

			{
				let mut s = state.lock().unwrap();
				while s.mem_used.saturating_add(entry.size as usize) > conf.max_io_memory_usage.get() {
					s = mem_cv.wait(s).unwrap();
				}
				s.mem_used += entry.size as usize;
			}

			let mut buf = Vec::new();
			(&mut *r)
				.take(entry.size)
				.read_to_end(&mut buf)
				.map_err(|e| entry_truncated(e, &entry.path))?;
			if buf.len() as u64 != entry.size {
				return Err(gma_err(format!("GMA is truncated, entry \"{}\" is cut short", entry.path.display())));
			}

			let mut s = state.lock().unwrap();
			s.queue.push_back((entry.path.clone(), buf));
			drop(s);
			queue_cv.notify_one();
		}

		Ok::<_, FastGmadError>(())
	})?;

	let mut s = state.lock().unwrap();
	s.failed_writes += inline_failed_writes;
	finish_failed_writes(s.failed_writes)
}

fn extract_entries_parallel(conf: &ExtractGmaConfig, gma_path: &Path, entries: &[GmaEntry]) -> Result<(), FastGmadError> {
	let next_entry = AtomicUsize::new(0);
	let failed_writes = AtomicUsize::new(0);
	let stop = AtomicBool::new(false);
	let read_error: Mutex<Option<FastGmadError>> = Mutex::new(None);

	std::thread::scope(|s| {
		for _ in 0..conf.max_io_threads.get() {
			s.spawn(|| {
				let file = match File::open(gma_path) {
					Ok(f) => f,
					Err(e) => {
						read_error
							.lock()
							.unwrap()
							.get_or_insert(FastGmadError::io(e, "opening GMA file", Some(gma_path)));
						stop.store(true, Ordering::Relaxed);
						return;
					}
				};

				let mut scratch: Vec<u8> = Vec::new();
				loop {
					if stop.load(Ordering::Relaxed) {
						return;
					}
					let index = next_entry.fetch_add(1, Ordering::Relaxed);
					let Some(entry) = entries.get(index) else {
						return;
					};
					if let Err(e) = write_entry_at(&file, &entry.path, entry.offset, entry.size, &mut scratch, &failed_writes) {
						read_error.lock().unwrap().get_or_insert(e);
						stop.store(true, Ordering::Relaxed);
						return;
					}
				}
			});
		}
	});

	if let Some(e) = read_error.into_inner().unwrap() {
		return Err(e);
	}
	finish_failed_writes(failed_writes.load(Ordering::Relaxed))
}

fn write_entry_at(file: &File, path: &Path, offset: u64, size: u64, scratch: &mut Vec<u8>, failed_writes: &AtomicUsize) -> Result<(), FastGmadError> {
	let mut out = match File::create(path) {
		Ok(f) => f,
		Err(e) => {
			log::warn!("Failed to create \"{}\": {}", path.display(), e);
			failed_writes.fetch_add(1, Ordering::Relaxed);
			return Ok(());
		}
	};

	let mut remaining = size;
	let mut position = offset;
	while remaining > 0 {
		let chunk = remaining.min(COPY_CHUNK) as usize;
		if scratch.len() < chunk {
			scratch.resize(chunk, 0);
		}
		read_exact_at(file, &mut scratch[..chunk], position).map_err(|e| entry_truncated(e, path))?;
		if let Err(e) = out.write_all(&scratch[..chunk]) {
			log::warn!("Failed to write \"{}\": {}", path.display(), e);
			failed_writes.fetch_add(1, Ordering::Relaxed);
			return Ok(());
		}
		position += chunk as u64;
		remaining -= chunk as u64;
	}
	Ok(())
}

#[cfg(unix)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut position: u64) -> std::io::Result<()> {
	use std::os::unix::fs::FileExt;
	while !buf.is_empty() {
		match file.read_at(buf, position) {
			Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected end of file")),
			Ok(n) => {
				buf = &mut buf[n..];
				position += n as u64;
			}
			Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
			Err(e) => return Err(e),
		}
	}
	Ok(())
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut position: u64) -> std::io::Result<()> {
	use std::os::windows::fs::FileExt;
	while !buf.is_empty() {
		match file.seek_read(buf, position) {
			Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "unexpected end of file")),
			Ok(n) => {
				buf = &mut buf[n..];
				position += n as u64;
			}
			Err(e) => return Err(e),
		}
	}
	Ok(())
}
