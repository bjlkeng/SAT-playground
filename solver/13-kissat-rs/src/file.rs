// Port of src/file.c + src/file.h (kissat 4.0.4).
//
// PORT NOTE: The C `struct file` wraps a `FILE *` that is either a regular
// stream, `stdin`/`stdout`, or a popen(3) pipe to an external
// (de)compression tool. Rust has no FILE*, so `File` holds a `Stream` enum
// instead; the public field set (`close`, `reading`, `compressed`, `path`,
// `bytes`) matches the C struct exactly. popen(cmd, mode) is reproduced as
// `/bin/sh -c <cmd>` with the matching end piped and the other end
// inherited, so shell-metacharacter behaviour in paths is identical to C.
// PORT NOTE: kissat 4.0.4 has no zstd support in file.c; none is added here.
// PORT NOTE: `kissat_read`/`kissat_write` use fread/fwrite, which loop
// internally until the full count or EOF/error; `read`/`write` below
// replicate that (a single `Read::read` may legally return short).
// PORT NOTE: `kissat_read_already_open_file` / `kissat_write_already_open_file`
// take a `FILE *` argument, but the only call sites (application.c) pass
// `stdin` respectively `stdout`; the Rust versions hard-wire those handles
// and drop the argument. The locked handles substitute for
// KISSAT_HAS_UNLOCKEDIO (getc_unlocked etc.).
// PORT NOTE: `kissat_looks_like_a_compressed_file` is only compiled when
// KISSAT_HAS_COMPRESSION is absent; the reference build (POSIX) has
// compression, so it is not ported.
// PORT NOTE: C relies on exit-time FILE flushing for the non-`close`
// stdout case and on fclose() for buffered writers; Rust's
// std::process::exit does not flush BufWriter/StdoutLock, so `close_file`
// flushes write streams explicitly before dropping them.

use std::ffi::CString;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read as IoRead, Write as IoWrite};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub const EOF: i32 = -1;

// access(2) mode bits (POSIX, as on Linux).
const R_OK: i32 = 4;
const W_OK: i32 = 2;

// PORT NOTE: file.c uses access(2) for readability/writability checks
// (which honours effective uid/ACLs and never opens/blocks on the path,
// unlike opening the file). Declared directly against libc to avoid a
// crate dependency.
extern "C" {
    fn access(path: *const std::os::raw::c_char, amode: i32) -> i32;
}

fn c_access(path: &str, amode: i32) -> bool {
    match CString::new(path) {
        Ok(c) => unsafe { access(c.as_ptr(), amode) == 0 },
        Err(_) => false,
    }
}

enum Stream {
    None,
    Stdin(std::io::StdinLock<'static>),
    Stdout(std::io::StdoutLock<'static>),
    ReadFile(BufReader<fs::File>),
    WriteFile(BufWriter<fs::File>),
    ReadPipe {
        child: Child,
        reader: BufReader<ChildStdout>,
    },
    WritePipe {
        child: Child,
        writer: BufWriter<ChildStdin>,
    },
}

pub struct File {
    stream: Stream,
    pub close: bool,
    pub reading: bool,
    pub compressed: bool,
    pub path: String,
    pub bytes: u64,
}

impl File {
    pub fn new() -> File {
        File {
            stream: Stream::None,
            close: false,
            reading: false,
            compressed: false,
            path: String::new(),
            bytes: 0,
        }
    }
}

impl Default for File {
    fn default() -> File {
        File::new()
    }
}

pub fn file_exists(path: &str) -> bool {
    // C: stat (path, &buf)
    fs::metadata(path).is_ok()
}

pub fn file_readable(path: &str) -> bool {
    if fs::metadata(path).is_err() {
        return false;
    }
    if !c_access(path, R_OK) {
        return false;
    }
    true
}

pub fn file_writable(path: &str) -> bool {
    // Faithful transliteration of kissat_file_writable, including its
    // quirks (e.g. a path like "/foo" yields dirname "" -> stat fails ->
    // reported unwritable; "/dev/null" is always writable).
    let res: i32;
    if path == "/dev/null" {
        res = 0;
    } else if path.is_empty() {
        res = 2;
    } else {
        match path.rfind('/') {
            None => match fs::metadata(path) {
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        res = 0;
                    } else {
                        res = -2;
                    }
                }
                Ok(buf) => {
                    if buf.is_dir() {
                        res = 3;
                    } else if !c_access(path, W_OK) {
                        res = 4;
                    } else {
                        res = 0;
                    }
                }
            },
            Some(p) if p + 1 == path.len() => res = 5,
            Some(p) => {
                let dirname = &path[..p];
                match fs::metadata(dirname) {
                    Err(_) => res = 6,
                    Ok(buf) if !buf.is_dir() => res = 7,
                    Ok(_) => {
                        if !c_access(dirname, W_OK) {
                            res = 8;
                        } else {
                            match fs::metadata(path) {
                                Err(e) => {
                                    if e.kind() == std::io::ErrorKind::NotFound {
                                        res = 0;
                                    } else {
                                        res = -3;
                                    }
                                }
                                Ok(_) => {
                                    if !c_access(path, W_OK) {
                                        res = 9;
                                    } else {
                                        res = 0;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    res == 0
}

pub fn file_size(path: &str) -> u64 {
    match fs::metadata(path) {
        Err(_) => 0,
        Ok(buf) => buf.len(),
    }
}

pub fn find_executable(name: &str) -> bool {
    // C iterates the ':'-separated PATH, including empty components
    // (an empty dir yields the path "/<name>"), and only checks
    // readability (R_OK), not executability — port that quirk.
    let environment = match std::env::var("PATH") {
        Ok(e) => e,
        Err(_) => return false,
    };
    for dir in environment.split(':') {
        let path = format!("{}/{}", dir, name);
        if file_readable(&path) {
            return true;
        }
    }
    false
}

static BZ2SIG: &[u8] = &[0x42, 0x5A, 0x68];
static GZSIG: &[u8] = &[0x1F, 0x8B];
static LZMASIG: &[u8] = &[0x5D, 0x00, 0x00, 0x80, 0x00];
static SIG7Z: &[u8] = &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
static XZSIG: &[u8] = &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, 0x00];
static ZSIG: &[u8] = &[0x1F, 0x9D, 0x90];

fn match_signature(path: &str, sig: &[u8]) -> bool {
    let mut tmp = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = vec![0u8; sig.len()];
    let mut have = 0usize;
    while have < sig.len() {
        match tmp.read(&mut buf[have..]) {
            Ok(0) => break,
            Ok(n) => have += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    have == sig.len() && buf == sig
}

fn open_pipe(fmt: &str, path: &str, reading: bool) -> Option<Child> {
    // C extracts the tool name as the prefix of the format string up to
    // the first space and checks it exists on PATH before popen'ing.
    let name_len = fmt.find(' ').unwrap_or(fmt.len());
    let name = &fmt[..name_len];
    if !find_executable(name) {
        return None;
    }
    let cmd = fmt.replacen("%s", path, 1);
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(cmd);
    if reading {
        command.stdout(Stdio::piped());
    } else {
        command.stdin(Stdio::piped());
    }
    command.spawn().ok()
}

fn read_pipe(fmt: &str, sig: Option<&[u8]>, path: &str) -> Option<Child> {
    if !file_readable(path) {
        return None;
    }
    if let Some(sig) = sig {
        if !match_signature(path, sig) {
            return None;
        }
    }
    open_pipe(fmt, path, true)
}

fn write_pipe(fmt: &str, path: &str) -> Option<Child> {
    open_pipe(fmt, path, false)
}

pub fn read_already_open_file(file: &mut File, path: &str) {
    // C: kissat_read_already_open_file (file, stdin, path)
    file.stream = Stream::Stdin(std::io::stdin().lock());
    file.close = false;
    file.reading = true;
    file.compressed = false;
    file.path = path.to_string();
    file.bytes = 0;
}

pub fn write_already_open_file(file: &mut File, path: &str) {
    // C: kissat_write_already_open_file (file, stdout, path)
    file.stream = Stream::Stdout(std::io::stdout().lock());
    file.close = false;
    file.reading = false;
    file.compressed = false;
    file.path = path.to_string();
    file.bytes = 0;
}

// READ_PIPE macro body: on suffix match try the pipe; on pipe failure fall
// through (in C via `break` out of the do-while) towards plain fopen.
fn try_read_pipe(file: &mut File, path: &str, suffix: &str, cmd: &str, sig: &[u8]) -> bool {
    if !crate::utilities::has_suffix(path, suffix) {
        return false;
    }
    let mut child = match read_pipe(cmd, Some(sig), path) {
        Some(child) => child,
        None => return false,
    };
    let stdout = child.stdout.take().expect("piped stdout");
    file.stream = Stream::ReadPipe {
        child,
        reader: BufReader::new(stdout),
    };
    file.close = true;
    file.reading = true;
    file.compressed = true;
    file.path = path.to_string();
    file.bytes = 0;
    true
}

pub fn open_to_read_file(file: &mut File, path: &str) -> bool {
    if try_read_pipe(file, path, ".bz2", "bzip2 -c -d %s", BZ2SIG) {
        return true;
    }
    if try_read_pipe(file, path, ".gz", "gzip -c -d %s", GZSIG) {
        return true;
    }
    if try_read_pipe(file, path, ".lzma", "lzma -c -d %s", LZMASIG) {
        return true;
    }
    if try_read_pipe(file, path, ".7z", "7z x -so %s 2>/dev/null", SIG7Z) {
        return true;
    }
    if try_read_pipe(file, path, ".xz", "xz -c -d %s", XZSIG) {
        return true;
    }
    if try_read_pipe(file, path, ".Z", "gzip -c -d %s", ZSIG) {
        return true;
    }
    let f = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    file.stream = Stream::ReadFile(BufReader::new(f));
    file.close = true;
    file.reading = true;
    file.compressed = false;
    file.path = path.to_string();
    file.bytes = 0;
    true
}

// WRITE_PIPE macro body. NOTE the C asymmetry: on a matching suffix a
// failed pipe open returns `false` immediately (no plain-file fallback,
// unlike the read side). Returns Some(success) when the suffix matched.
fn try_write_pipe(file: &mut File, path: &str, suffix: &str, cmd: &str) -> Option<bool> {
    if !crate::utilities::has_suffix(path, suffix) {
        return None;
    }
    // C: if (SUFFIX[1] == '7' && kissat_file_readable (path) && unlink (path))
    //      return false;
    // ('7z a' appends to an existing archive, so it is removed first.)
    if suffix.as_bytes()[1] == b'7' && file_readable(path) && fs::remove_file(path).is_err() {
        return Some(false);
    }
    let mut child = match write_pipe(cmd, path) {
        Some(child) => child,
        None => return Some(false),
    };
    let stdin = child.stdin.take().expect("piped stdin");
    file.stream = Stream::WritePipe {
        child,
        writer: BufWriter::new(stdin),
    };
    file.close = true;
    file.reading = false;
    file.compressed = true;
    file.path = path.to_string();
    file.bytes = 0;
    Some(true)
}

pub fn open_to_write_file(file: &mut File, path: &str) -> bool {
    if let Some(res) = try_write_pipe(file, path, ".bz2", "bzip2 -c > %s") {
        return res;
    }
    if let Some(res) = try_write_pipe(file, path, ".gz", "gzip -c > %s") {
        return res;
    }
    if let Some(res) = try_write_pipe(file, path, ".lzma", "lzma -c > %s") {
        return res;
    }
    if let Some(res) = try_write_pipe(file, path, ".7z", "7z a -si %s 2>/dev/null") {
        return res;
    }
    if let Some(res) = try_write_pipe(file, path, ".xz", "xz -c > %s") {
        return res;
    }
    let f = match fs::File::create(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    file.stream = Stream::WriteFile(BufWriter::new(f));
    file.close = true;
    file.reading = false;
    file.compressed = false;
    file.path = path.to_string();
    file.bytes = 0;
    true
}

pub fn close_file(file: &mut File) {
    let stream = std::mem::replace(&mut file.stream, Stream::None);
    match stream {
        Stream::ReadPipe { mut child, reader } => {
            // pclose: closes our end of the pipe and waits for the child
            // (an early close SIGPIPEs a still-writing decompressor,
            // exactly as with pclose).
            debug_assert!(file.close && file.compressed);
            drop(reader);
            let _ = child.wait();
        }
        Stream::WritePipe { mut child, mut writer } => {
            debug_assert!(file.close && file.compressed);
            let _ = writer.flush();
            drop(writer); // closes the compressor's stdin -> it finishes
            let _ = child.wait();
        }
        Stream::WriteFile(mut w) => {
            // fclose flushes.
            let _ = w.flush();
            drop(w);
        }
        Stream::ReadFile(r) => {
            drop(r);
        }
        Stream::Stdout(mut out) => {
            // C leaves stdout open (close == false) and relies on exit-time
            // flushing; Rust's process::exit does not flush, so do it here.
            debug_assert!(!file.close);
            let _ = out.flush();
        }
        Stream::Stdin(_) | Stream::None => {}
    }
}

fn read_full(r: &mut impl IoRead, buf: &mut [u8]) -> usize {
    // fread semantics: loop until the requested count or EOF/error.
    let mut total = 0usize;
    while total < buf.len() {
        match r.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    total
}

fn write_full(w: &mut impl IoWrite, buf: &[u8]) -> usize {
    let mut total = 0usize;
    while total < buf.len() {
        match w.write(&buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    total
}

// kissat_read (file.h inline)
pub fn read(file: &mut File, bytes: &mut [u8]) -> usize {
    debug_assert!(file.reading);
    let res = match &mut file.stream {
        Stream::Stdin(s) => read_full(s, bytes),
        Stream::ReadFile(s) => read_full(s, bytes),
        Stream::ReadPipe { reader, .. } => read_full(reader, bytes),
        _ => 0,
    };
    file.bytes += res as u64;
    res
}

// kissat_write (file.h inline)
pub fn write(file: &mut File, bytes: &[u8]) -> usize {
    debug_assert!(!file.reading);
    let res = match &mut file.stream {
        Stream::Stdout(s) => write_full(s, bytes),
        Stream::WriteFile(s) => write_full(s, bytes),
        Stream::WritePipe { writer, .. } => write_full(writer, bytes),
        _ => 0,
    };
    file.bytes += res as u64;
    res
}

#[inline]
fn getc_from(r: &mut impl BufRead) -> i32 {
    match r.fill_buf() {
        Ok(buf) if !buf.is_empty() => {
            let ch = buf[0] as i32;
            r.consume(1);
            ch
        }
        _ => EOF,
    }
}

// kissat_getc (file.h inline): byte-at-a-time read off the internal
// buffer (getc_unlocked equivalent); returns EOF (-1) at end of input.
#[inline]
pub fn getc(file: &mut File) -> i32 {
    debug_assert!(file.reading);
    let res = match &mut file.stream {
        Stream::Stdin(s) => getc_from(s),
        Stream::ReadFile(s) => getc_from(s),
        Stream::ReadPipe { reader, .. } => getc_from(reader),
        _ => EOF,
    };
    if res != EOF {
        file.bytes += 1;
    }
    res
}

// kissat_putc (file.h inline). PORT NOTE: like the C original this
// returns `ch` unconditionally (not the putc result) and only counts the
// byte when the write succeeded.
#[inline]
pub fn putc(file: &mut File, ch: i32) -> i32 {
    debug_assert!(!file.reading);
    let byte = [ch as u8];
    let ok = match &mut file.stream {
        Stream::Stdout(s) => s.write_all(&byte).is_ok(),
        Stream::WriteFile(s) => s.write_all(&byte).is_ok(),
        Stream::WritePipe { writer, .. } => writer.write_all(&byte).is_ok(),
        _ => false,
    };
    if ok {
        file.bytes += 1;
    }
    ch
}

// kissat_flush (file.h inline)
pub fn flush(file: &mut File) {
    debug_assert!(!file.reading);
    match &mut file.stream {
        Stream::Stdout(s) => {
            let _ = s.flush();
        }
        Stream::WriteFile(s) => {
            let _ = s.flush();
        }
        Stream::WritePipe { writer, .. } => {
            let _ = writer.flush();
        }
        _ => {}
    }
}
