use fastgmad::extract::{ExtractGmaConfig, extract_gma, extract_gma_file};
use std::{
	io::{Cursor, Read, Write},
	path::{Path, PathBuf},
};

fn push_nul(buf: &mut Vec<u8>, s: &str) {
	buf.extend_from_slice(s.as_bytes());
	buf.push(0);
}

fn build_gma_from(entries: &[(&str, &[u8])]) -> Vec<u8> {
	let mut b = Vec::new();
	b.extend_from_slice(b"GMAD");
	b.push(3);
	b.extend_from_slice(&0u64.to_le_bytes());
	b.extend_from_slice(&0u64.to_le_bytes());
	b.push(0);
	push_nul(&mut b, "Test Addon");
	push_nul(&mut b, "{}");
	push_nul(&mut b, "author");
	b.extend_from_slice(&0u32.to_le_bytes());

	for (i, (name, data)) in entries.iter().enumerate() {
		b.extend_from_slice(&(i as u32 + 1).to_le_bytes());
		push_nul(&mut b, name);
		b.extend_from_slice(&(data.len() as u64).to_le_bytes());
		b.extend_from_slice(&0u32.to_le_bytes());
	}
	b.extend_from_slice(&0u32.to_le_bytes());
	for (_, data) in entries {
		b.extend_from_slice(data);
	}
	b
}

fn build_gma() -> Vec<u8> {
	build_gma_from(&[
		("lua/test.lua", b"hello"),
		("sub/dir/bin.png", &[1, 2, 3, 4, 5]),
		("../evil.txt", b"evil"),
		("lua/test.lua", b"duplicate"),
	])
}

fn temp_out(tag: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("fastgmad-test-{}-{tag}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	dir
}

fn test_conf(out: &Path, threads: usize) -> ExtractGmaConfig {
	ExtractGmaConfig {
		out: out.to_path_buf(),
		max_io_threads: threads.try_into().unwrap(),
		..Default::default()
	}
}

fn assert_extracted(out: &Path) {
	assert_eq!(std::fs::read(out.join("lua/test.lua")).unwrap(), b"hello");
	assert_eq!(std::fs::read(out.join("sub/dir/bin.png")).unwrap(), &[1, 2, 3, 4, 5][..]);
	assert_eq!(std::fs::read(out.join("badnames/0.unk")).unwrap(), b"evil");
	assert_eq!(std::fs::read(out.join("badnames/1.unk")).unwrap(), b"duplicate");
	let json = std::fs::read_to_string(out.join("addon.json")).unwrap();
	assert!(json.contains("Test Addon"));
}

#[test]
fn extracts_from_stream() {
	let out = temp_out("stream");
	extract_gma(&test_conf(&out, 4), &mut Cursor::new(build_gma())).unwrap();
	assert_extracted(&out);
	let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn extracts_from_file() {
	let out = temp_out("file");
	let gma_path = temp_out("gma").with_extension("gma");
	std::fs::create_dir_all(&out).unwrap();
	std::fs::write(&gma_path, build_gma()).unwrap();

	extract_gma_file(&test_conf(&out, 4), &gma_path).unwrap();
	assert_extracted(&out);
	let _ = std::fs::remove_dir_all(&out);
	let _ = std::fs::remove_file(&gma_path);
}

#[test]
fn rejects_truncated_gma() {
	let mut gma = build_gma();
	gma.truncate(gma.len() - 2);
	let out = temp_out("truncated");
	let err = extract_gma(&test_conf(&out, 4), &mut Cursor::new(gma)).unwrap_err();
	assert!(format!("{err}").contains("truncated"), "unexpected error: {err}");
}

#[test]
#[cfg(windows)]
fn windows_device_names_go_to_badnames() {
	let out = temp_out("devices");
	let gma = build_gma_from(&[("CON", b"c"), ("settings/nul.txt", b"n"), ("lua/ok.lua", b"ok")]);
	extract_gma(&test_conf(&out, 2), &mut Cursor::new(gma)).unwrap();
	assert_eq!(std::fs::read(out.join("badnames/0.unk")).unwrap(), b"c");
	assert_eq!(std::fs::read(out.join("badnames/1.unk")).unwrap(), b"n");
	assert_eq!(std::fs::read(out.join("lua/ok.lua")).unwrap(), b"ok");
	let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn unwritable_entry_keeps_stream_aligned() {
	let out = temp_out("blocked");
	std::fs::create_dir_all(out.join("blocker/file.bin")).unwrap();
	let gma = build_gma_from(&[("blocker/file.bin", b"BBBB"), ("after/ok.txt", b"good")]);
	let err = extract_gma(&test_conf(&out, 1), &mut Cursor::new(gma)).unwrap_err();
	assert!(format!("{err}").contains("failed to write"), "unexpected error: {err}");
	assert_eq!(std::fs::read(out.join("after/ok.txt")).unwrap(), b"good");
	let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn zero_size_entries_in_sequential_mode() {
	let out = temp_out("zero");
	let gma = build_gma_from(&[("a/empty.bin", b""), ("b/data.bin", b"payload"), ("c/empty2", b"")]);
	extract_gma(&test_conf(&out, 1), &mut Cursor::new(gma)).unwrap();
	assert_eq!(out.join("a/empty.bin").metadata().unwrap().len(), 0);
	assert_eq!(std::fs::read(out.join("b/data.bin")).unwrap(), b"payload");
	assert_eq!(out.join("c/empty2").metadata().unwrap().len(), 0);
	let _ = std::fs::remove_dir_all(&out);
}

struct XorShift(u64);

impl XorShift {
	fn next(&mut self) -> u64 {
		let mut x = self.0;
		x ^= x << 13;
		x ^= x >> 7;
		x ^= x << 17;
		self.0 = x;
		x
	}

	fn below(&mut self, n: u64) -> u64 {
		self.next() % n
	}
}

fn fnv1a(bytes: &[u8]) -> u64 {
	let mut hash = 0xcbf29ce484222325;
	for &byte in bytes {
		hash ^= byte as u64;
		hash = hash.wrapping_mul(0x100000001b3);
	}
	hash
}

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

#[test]
fn randomized_roundtrip_of_60_gmas() {
	let root = temp_out("fuzz");
	std::fs::create_dir_all(&root).unwrap();
	let mut rng = XorShift(0x9E3779B97F4A7C15);
	let mut total_gma_bytes = 0u64;

	for gma_index in 0..60u64 {
		let unit = (rng.below(10_000)) as f64 / 10_000.0;
		let total_size = (400.0 * KIB as f64 + (90.0 * MIB as f64 - 400.0 * KIB as f64) * unit * unit * unit * unit).round() as u64;
		let entry_count = 1 + rng.below(8) as usize;
		let base = total_size / entry_count as u64;

		let mut lens: Vec<u64> = Vec::with_capacity(entry_count);
		let mut remaining = total_size;
		for i in 0..entry_count {
			let len = if i + 1 == entry_count {
				remaining
			} else if rng.below(10) == 0 {
				0
			} else {
				base / 2 + rng.below(base / 2 + 1)
			};
			lens.push(len);
			remaining -= len;
		}

		let mut names: Vec<String> = Vec::with_capacity(entry_count);
		for i in 0..entry_count {
			let (dir, ext) = [
				("maps", "bsp"),
				("lua/autorun", "lua"),
				("materials/models", "vtf"),
				("sound/weapons", "wav"),
				("data", "json"),
			][rng.below(5) as usize];
			names.push(format!("{dir}/fuzz_{gma_index}_{i}.{ext}"));
		}
		let has_evil = rng.below(6) == 0;

		let mut data = vec![0u8; total_size as usize];
		{
			let (chunks, tail) = data.as_chunks_mut::<8>();
			for chunk in chunks {
				chunk.copy_from_slice(&rng.next().to_le_bytes());
			}
			if !tail.is_empty() {
				let bytes = rng.next().to_le_bytes();
				tail.copy_from_slice(&bytes[..tail.len()]);
			}
		}

		let mut prefix = Vec::new();
		prefix.extend_from_slice(b"GMAD");
		prefix.push(3);
		prefix.extend_from_slice(&0u64.to_le_bytes());
		prefix.extend_from_slice(&0u64.to_le_bytes());
		prefix.push(0);
		push_nul(&mut prefix, "Fuzz Addon");
		push_nul(&mut prefix, "{}");
		push_nul(&mut prefix, "fuzzer");
		prefix.extend_from_slice(&0u32.to_le_bytes());
		for (i, name) in names.iter().enumerate() {
			prefix.extend_from_slice(&(i as u32 + 1).to_le_bytes());
			push_nul(&mut prefix, name);
			prefix.extend_from_slice(&lens[i].to_le_bytes());
			prefix.extend_from_slice(&0u32.to_le_bytes());
		}
		if has_evil {
			prefix.extend_from_slice(&(names.len() as u32 + 1).to_le_bytes());
			push_nul(&mut prefix, "../fuzz_evil.bin");
			prefix.extend_from_slice(&7u64.to_le_bytes());
			prefix.extend_from_slice(&0u32.to_le_bytes());
		}
		prefix.extend_from_slice(&0u32.to_le_bytes());

		let gma_path = root.join(format!("fuzz_{gma_index}.gma"));
		{
			let mut file = std::fs::File::create(&gma_path).unwrap();
			file.write_all(&prefix).unwrap();
			file.write_all(&data).unwrap();
			if has_evil {
				file.write_all(b"badname").unwrap();
			}
		}
		total_gma_bytes += gma_path.metadata().unwrap().len();

		let out = root.join(format!("out_{gma_index}"));
		let conf = test_conf(&out, 4);
		if gma_index % 2 == 0 {
			extract_gma_file(&conf, &gma_path).unwrap();
		} else {
			let evil: &[u8] = if has_evil { b"badname" } else { &[] };
			extract_gma(&conf, &mut Cursor::new(prefix).chain(Cursor::new(&data)).chain(Cursor::new(evil))).unwrap();
		}

		let mut offset = 0u64;
		for (i, name) in names.iter().enumerate() {
			let bytes = std::fs::read(out.join(name)).unwrap();
			assert_eq!(bytes.len() as u64, lens[i], "wrong size for {name} in GMA {gma_index}");
			let expected = fnv1a(&data[offset as usize..(offset + lens[i]) as usize]);
			assert_eq!(fnv1a(&bytes), expected, "wrong content for {name} in GMA {gma_index}");
			offset += lens[i];
		}
		if has_evil {
			let bytes = std::fs::read(out.join("badnames/0.unk")).unwrap();
			assert_eq!(bytes, b"badname", "wrong badnames content in GMA {gma_index}");
		}

		let _ = std::fs::remove_file(&gma_path);
		let _ = std::fs::remove_dir_all(&out);
	}

	println!("randomized roundtrip: 60 GMAs, {total_gma_bytes} GMA bytes total");
	let _ = std::fs::remove_dir_all(&root);
}
