use std::{
	path::PathBuf,
	process::{Command, Stdio},
};

const HELLO_CRC: u32 = 0x3610_a686;

fn gma(entry_crc: u32) -> Vec<u8> {
	let mut b = b"GMAD".to_vec();
	b.push(3);
	b.extend_from_slice(&[0u8; 16]);
	b.push(0);
	b.extend_from_slice(b"Test\0{}\0author\0");
	b.extend_from_slice(&0u32.to_le_bytes());
	b.extend_from_slice(&1u32.to_le_bytes());
	b.extend_from_slice(b"lua/a.lua\0");
	b.extend_from_slice(&5u64.to_le_bytes());
	b.extend_from_slice(&entry_crc.to_le_bytes());
	b.extend_from_slice(&0u32.to_le_bytes());
	b.extend_from_slice(b"hello");
	b
}

struct Run {
	code: i32,
	stdout: String,
	stderr: String,
	dir: PathBuf,
}

fn run_case(tag: &str, bytes: &[u8], extra: &[&str]) -> Run {
	let dir: PathBuf = std::env::temp_dir().join(format!("fastgmad-bin-test-{}-{tag}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let gma_path = dir.join("addon.gma");
	std::fs::write(&gma_path, bytes).unwrap();

	let output = Command::new(env!("CARGO_BIN_EXE_fastgmad"))
		.arg("extract")
		.args(extra)
		.arg("-file")
		.arg(&gma_path)
		.arg("-out")
		.arg(dir.join("out"))
		.stdin(Stdio::null())
		.output()
		.unwrap();

	Run {
		code: output.status.code().unwrap(),
		stdout: String::from_utf8(output.stdout).unwrap(),
		stderr: String::from_utf8(output.stderr).unwrap(),
		dir,
	}
}

#[test]
fn success_prints_one_json_line_on_piped_stdout() {
	let run = run_case("ok", &gma(HELLO_CRC), &[]);
	assert_eq!(run.code, 0);
	assert_eq!(
		run.stdout,
		"{\"v\":1,\"entries\":1,\"bad_names\":0,\"crc_checked\":1,\"crc_mismatch\":0}\n"
	);
	let _ = std::fs::remove_dir_all(&run.dir);
}

#[test]
fn crc_mismatch_warns_but_exits_zero() {
	let run = run_case("crc", &gma(HELLO_CRC ^ 1), &[]);
	assert_eq!(run.code, 0);
	assert!(run.stdout.contains("\"crc_mismatch\":1"), "unexpected stdout: {}", run.stdout);
	assert!(run.stderr.contains("CRC mismatch"), "unexpected stderr: {}", run.stderr);
	assert_eq!(std::fs::read(run.dir.join("out/lua/a.lua")).unwrap(), b"hello");
	let _ = std::fs::remove_dir_all(&run.dir);
}

#[test]
fn strict_crc_mismatch_exits_one_and_names_the_file() {
	let run = run_case("strict", &gma(HELLO_CRC ^ 1), &["--strict"]);
	assert_eq!(run.code, 1);
	assert_eq!(run.stdout, "");
	assert!(run.stderr.contains("lua/a.lua"), "unexpected stderr: {}", run.stderr);
	assert_eq!(std::fs::read(run.dir.join("out/lua/a.lua")).unwrap(), b"hello");
	let _ = std::fs::remove_dir_all(&run.dir);
}

#[test]
fn truncated_input_exits_one_with_empty_stdout() {
	let mut bytes = gma(HELLO_CRC);
	bytes.truncate(bytes.len() - 2);
	let run = run_case("truncated", &bytes, &[]);
	assert_eq!(run.code, 1);
	assert_eq!(run.stdout, "");
	let _ = std::fs::remove_dir_all(&run.dir);
}

#[test]
fn no_arguments_exits_two_with_usage_on_stderr() {
	let output = Command::new(env!("CARGO_BIN_EXE_fastgmad")).stdin(Stdio::null()).output().unwrap();
	assert_eq!(output.status.code(), Some(2));
	assert!(output.stdout.is_empty());
	assert!(String::from_utf8(output.stderr).unwrap().contains("FastGMAD"));
}
