//! Switch to the ASCII input source for a moment and back, printing the active source's
//! identifier at each step: `cargo run -p dupin-mac --example input_source`.

#[cfg(target_os = "macos")]
use dupin_mac::input_source::{Target, select};

#[cfg(target_os = "macos")]
fn main() {
    let before = current();
    let previous = select(&Target::Ascii);
    println!(
        "before: {before}\nswitched: {}\nduring: {}",
        previous.is_some(),
        current()
    );
    if let Some(previous) = previous {
        previous.restore();
    }
    println!("after: {}", current());
}

#[cfg(target_os = "macos")]
fn current() -> String {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.HIToolbox", "AppleSelectedInputSources"])
        .output()
        .map(|o| {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.lines()
                .filter(|l| l.contains("Input Mode") || l.contains("KeyboardLayout Name"))
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("this example runs on macOS only");
}
