use clap::Parser;
use gethostname::gethostname;
use std::io;
use std::io::Write;
use std::process::exit;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DATA_MAX_LEN: usize = 64;
const HOSTNAME_MAX_LEN: usize = 16;

/// stdout to collectd notifications
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// collectd service: <plugin>/<type>
    #[clap(short, long, default_value = "stdout/prv")]
    service: String,

    /// system hostname
    #[clap(short = 'H', long, default_value = "")]
    hostname: String,

    /// message rate limit
    #[clap(short, long, default_value_t = 0)]
    limit: usize,

    /// message rate window
    #[clap(short, long, default_value_t = 1)]
    window: u64,

    /// max message fragment length
    #[clap(short = 'M', long = "max-event-length", default_value_t = 245)]
    max_event_length: usize, // 255 - 10

    /// max message fragment header id
    #[clap(short = 'I', long = "max-event-id", default_value_t = 99)]
    max_event_id: u64,

    /// behaviour if write buffer is full
    #[clap(short = 'W', long = "write-buffer", default_value = "block")]
    write_buffer: String,

    /// verbose mode
    #[clap(short, long)]
    verbose: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = Args::parse();

    if args.hostname.len() >= HOSTNAME_MAX_LEN {
        eprintln!("invalid hostname: {}", args.hostname);
        exit(1)
    }

    if args.hostname.is_empty() {
        args.hostname = gethostname().into_string().unwrap();
    }

    let service: Vec<&str> = args.service.split('/').collect();

    if service.len() != 2 {
        eprintln!("invalid service: {}", args.service);
        exit(1)
    }

    for s in &service {
        if s.len() >= DATA_MAX_LEN {
            eprintln!("invalid service: {}", *s);
            exit(1)
        }
    }

    event_loop(&args, service)
}

fn event_loop(args: &Args, service: Vec<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let (plugin, ctype) = (service[0], service[1]);

    let mut stdout = io::stdout();
    let stdin = io::stdin();

    let mut t0 = Instant::now();

    let mut count = 0;
    let mut id = 1;

    loop {
        let mut buf = String::new();
        let buflen = match stdin.read_line(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) => return Err(Box::new(err)),
        };

        let len = match buf.find('\0') {
            Some(n) => n,
            None => buflen - if buf.ends_with('\n') { 1 } else { 0 },
        };

        let t1 = Instant::now();

        if t1.duration_since(t0).as_secs() >= args.window {
            count = 0;
            t0 = t1;
        }

        let chunks = len / args.max_event_length;
        let rem = len % args.max_event_length;
        let total = chunks + if rem == 0 { 0 } else { 1 };

        count += total;

        if args.limit > 0 && count > args.limit {
            if args.verbose {
                eprint!("DISCARD:{}/{}:{}", count, args.limit, buf);
            }
            continue;
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;

        let mut start = 0;

        for n in 0..total {
            stdout.write_all(
                format!(
                    "PUTNOTIF host={} severity=okay time={} plugin={} type={} message=\"",
                    args.hostname,
                    now.as_secs(),
                    plugin,
                    ctype,
                )
                .as_bytes(),
            )?;
            if total > 1 {
                stdout.write_all(format!("@{}:{}:{}@", id, n + 1, total).as_bytes())?;
            }
            let mut eol = false;
            let remainder = len - start;
            let end = if remainder > args.max_event_length {
                start + args.max_event_length
            } else {
                len
            };
            for c in buf[start..end].bytes() {
                match c as char {
                    '\\' => stdout.write_all(b"\\\\"),
                    '"' => stdout.write_all(b"\\\""),
                    '\r' | '\n' => {
                        eol = true;
                        Ok(())
                    }
                    _ => stdout.write_all(&[c]),
                }?;
                if eol {
                    break;
                }
            }
            stdout.write_all(b"\"\n")?;
            stdout.flush()?;

            start = end;
        }

        if total > 1 {
            id = (id % args.max_event_id) + 1;
        }
    }
}
