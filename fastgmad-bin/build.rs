fn main() {
	println!("cargo:rerun-if-changed=build.rs");
	println!("cargo:rerun-if-changed=src/usage.txt");

	let usage = std::fs::read_to_string("src/usage.txt").expect("failed to read usage.txt");
	let readme = std::fs::read_to_string("../README.md").expect("failed to read README.md");

	const BEGIN: &str = "<!--BEGINUSAGE><!-->";
	const END: &str = "<!--ENDUSAGE><!-->";

	let usage_start = readme.find(BEGIN).expect("BEGIN marker not found in README") + BEGIN.len();
	let usage_end = readme[usage_start..]
		.find(END)
		.map(|i| usage_start + i)
		.expect("END marker not found after BEGIN in README");

	let readme = format!("{}\n```\n{usage}\n```\n{}", &readme[..usage_start], &readme[usage_end..]);
	std::fs::write("../README.md", readme).expect("failed to write README.md");
}
