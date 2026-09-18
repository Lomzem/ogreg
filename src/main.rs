mod cli;
mod output;
mod protocol;
mod session;

use clap::Parser;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

fn main() -> ExitCode {
    let args = cli::Cli::parse();
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&interrupted);
    if let Err(error) = ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed)) {
        eprintln!("error: could not install interrupt handler: {error}");
        return ExitCode::FAILURE;
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker_signal = Arc::clone(&interrupted);
    // Keep the main thread responsive even during a blocked OS network call.
    std::thread::spawn(move || {
        let mut stdout = io::stdout().lock();
        let result =
            session::run(&args, &worker_signal, &mut stdout).map_err(|error| error.to_string());
        let _ = sender.send(result);
    });
    loop {
        if interrupted.load(Ordering::Relaxed) {
            return ExitCode::from(130);
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(())) => return ExitCode::SUCCESS,
            Ok(Err(_)) if interrupted.load(Ordering::Relaxed) => return ExitCode::from(130),
            Ok(Err(error)) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("error: command worker stopped unexpectedly");
                return ExitCode::FAILURE;
            }
        }
    }
}
