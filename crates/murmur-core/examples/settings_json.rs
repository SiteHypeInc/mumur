//! `settings_json dump` prints default settings as JSON (what the UI receives);
//! `settings_json check FILE` verifies JSON the UI produced deserializes.
use murmur_core::config::Settings;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("check") => {
            let raw = std::fs::read_to_string(&args[2]).expect("read file");
            let s: Settings = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("UI payload rejected: {e}"));
            println!("ok: binding={} mode={:?} modes={} terms={:?} reps={} styles={:?}",
                s.hotkey.binding, s.hotkey.mode, s.modes.len(), s.dictionary.terms, s.dictionary.replacements.len(), s.styles);
            for m in &s.modes { println!("mode: {m:?}"); }
        }
        _ => println!("{}", serde_json::to_string(&Settings::default()).unwrap()),
    }
}
