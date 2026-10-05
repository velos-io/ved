use std::io::{self, Read, Write};
use std::process::Command;
use std::time::Duration;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("sleep") => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
        Some("ready-sleep") => {
            std::fs::write(args.next().unwrap(), "ready").unwrap();
            std::thread::sleep(Duration::from_millis(700));
        }
        Some("mark") => {
            std::fs::write(args.next().unwrap(), "spawned").unwrap();
        }
        Some("signal-policy") => {
            // Run inside a dedicated child: a signal must retain its default
            // action even while Runner supervises an active subprocess.
            let result = ved_process::Runner::new(std::env::current_exe().unwrap())
                .args(["ready-sleep", &args.next().unwrap()])
                .timeout(Duration::from_secs(3))
                .run();
            assert!(result.unwrap().status.success());
        }
        Some("duplex") => {
            let mut input = Vec::new();
            io::stdin().read_to_end(&mut input).unwrap();
            let len: usize = args.next().unwrap().parse().unwrap();
            let bytes = vec![b'x'; len];
            io::stdout().write_all(&bytes).unwrap();
            io::stderr().write_all(&bytes).unwrap();
            io::stdout()
                .write_all(&(input.len() as u64).to_le_bytes())
                .unwrap();
        }
        Some("write") => {
            let len: usize = args.next().unwrap().parse().unwrap();
            io::stdout().write_all(&vec![b'o'; len]).unwrap();
            io::stderr().write_all(&vec![b'e'; len]).unwrap();
        }
        Some("exit") => std::process::exit(args.next().unwrap().parse().unwrap()),
        Some("exit-immediately") => std::process::exit(0),
        Some("descendant") => {
            #[cfg(unix)]
            {
                let pid_file = args.next().unwrap();
                let child = Command::new(std::env::current_exe().unwrap())
                    .arg("sleep")
                    .spawn()
                    .unwrap();
                std::fs::write(pid_file, child.id().to_string()).unwrap();
                std::mem::forget(child);
                loop {
                    std::thread::sleep(Duration::from_secs(60));
                }
            }
            #[cfg(not(unix))]
            {
                std::process::exit(77);
            }
        }
        Some("orphan-pipes") => {
            let child = Command::new(std::env::current_exe().unwrap())
                .arg("sleep")
                .spawn()
                .unwrap();
            std::mem::forget(child);
        }
        _ => std::process::exit(64),
    }
}
