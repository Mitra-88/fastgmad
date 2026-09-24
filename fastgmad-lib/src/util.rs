use std::io::{BufRead, Read, Result};

pub trait BufReadEx: BufRead {
	fn read_nul_str<'a>(&mut self, buf: &'a mut Vec<u8>) -> Result<&'a [u8]> {
		buf.clear();
		self.read_until(0, buf)?;
		if buf.last() == Some(&0) {
			buf.pop();
		}
		Ok(buf)
	}

	fn skip_nul_str(&mut self) -> Result<()> {
		loop {
			let (done, used) = {
				let available = match self.fill_buf() {
					Ok(n) => n,
					Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
					Err(e) => return Err(e),
				};
				match memchr::memchr(0, available) {
					Some(i) => (true, i + 1),
					None => (false, available.len()),
				}
			};
			self.consume(used);
			if done || used == 0 {
				return Ok(());
			}
		}
	}
}

impl<R: BufRead + ?Sized> BufReadEx for R {}

pub trait ReadSkip: Read {
	fn skip(&mut self, bytes: u64) -> Result<()> {
		let n = std::io::copy(&mut self.take(bytes), &mut std::io::sink())?;
		if n != bytes {
			return Err(std::io::Error::new(
				std::io::ErrorKind::UnexpectedEof,
				format!("expected to skip {bytes} bytes, only got {n}"),
			));
		}
		Ok(())
	}
}

impl<R: Read + ?Sized> ReadSkip for R {}
