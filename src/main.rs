use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "harmony-midi")]
#[command(about = "Convert Harmony firmware/song streams to and from MIDI", long_about = None)]
struct Cli {
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Extract {
        firmware: PathBuf,
        output_dir: PathBuf,
    },
    Build {
        #[arg(short = 'c', long, value_delimiter = ',', num_args = 3, value_parser = clap::value_parser!(u8).range(0..=15))]
        channel_volumes: Option<Vec<u8>>,
        #[arg(short = 't', long, default_value_t = harmony_midi::HARMONY_TICKS_PER_QUARTER, value_parser = clap::value_parser!(u16).range(1..))]
        ticks_per_quarter: u16,
        input_dir: PathBuf,
        output_firmware: PathBuf,
    },
    ToMidi {
        code_bin: PathBuf,
        song_bin: PathBuf,
        output_midi: PathBuf,
    },
    ToHarmony {
        #[arg(short = 't', long, default_value_t = harmony_midi::HARMONY_TICKS_PER_QUARTER, value_parser = clap::value_parser!(u16).range(1..))]
        ticks_per_quarter: u16,
        code_bin: PathBuf,
        input_midi: PathBuf,
        output_song: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let verbose = cli.verbose;
    match cli.command {
        Command::Extract {
            firmware,
            output_dir,
        } => harmony_midi::extract_firmware_to_dir(&firmware, &output_dir),
        Command::Build {
            channel_volumes,
            ticks_per_quarter,
            input_dir,
            output_firmware,
        } => {
            let channel_volumes = channel_volumes.map(|values| {
                let [a, b, c]: [u8; 3] = values.try_into().expect("clap enforces exactly 3 values");
                [a, b, c]
            });
            let report = harmony_midi::build_firmware_from_dir_with_options(
                &input_dir,
                &output_firmware,
                channel_volumes,
                ticks_per_quarter,
            )?;
            let total_used: usize = report.banks.iter().map(|bank| bank.used_bytes).sum();
            let total_capacity = report.bank_capacity_bytes * report.banks.len();
            println!(
                "built {} banks: {} / {} bytes used, {} bytes free",
                report.banks.len(),
                total_used,
                total_capacity,
                total_capacity.saturating_sub(total_used),
            );
            println!("code section per bank: {} bytes", report.code_section_bytes);
            println!(
                "MIDI import baseline: {} harmony ticks per quarter note at 120 BPM (.fur files use module timing)",
                ticks_per_quarter
            );
            for bank in &report.banks {
                println!(
                    "bank {:02}: used {} / {} bytes, free {}, song data {}, unique songs {}, aliased slots {}",
                    bank.bank_index + 1,
                    bank.used_bytes,
                    report.bank_capacity_bytes,
                    bank.free_bytes,
                    bank.song_data_bytes,
                    bank.unique_song_count,
                    bank.aliased_song_slots,
                );
            }
            for warning in render_warnings(report.warnings, verbose) {
                eprintln!("warning: {warning}");
            }
            Ok(())
        }
        Command::ToMidi {
            code_bin,
            song_bin,
            output_midi,
        } => harmony_midi::harmony_song_to_midi_file(&code_bin, &song_bin, &output_midi),
        Command::ToHarmony {
            ticks_per_quarter,
            code_bin,
            input_midi,
            output_song,
        } => {
            let warnings = harmony_midi::midi_song_to_harmony_file_with_ticks(
                &code_bin,
                &input_midi,
                &output_song,
                ticks_per_quarter,
            )?;
            for warning in render_warnings(warnings, verbose) {
                eprintln!("warning: {warning}");
            }
            Ok(())
        }
    }
}

fn render_warnings(warnings: Vec<String>, verbose: bool) -> Vec<String> {
    if verbose {
        return warnings;
    }

    let mut overlapping_by_context: BTreeMap<(Option<String>, u32), usize> = BTreeMap::new();
    let mut collapsed_by_context: BTreeMap<(Option<String>, u32), usize> = BTreeMap::new();
    let mut passthrough_counts: BTreeMap<String, usize> = BTreeMap::new();

    for warning in warnings {
        let (context, detail) = split_warning_context(&warning);
        if let Some(channel) = extract_channel(
            detail,
            "dropping note ",
            " on MIDI channel ",
            " because a later overlapping note quantised to the same harmony tick",
        ) {
            *overlapping_by_context
                .entry((context, channel))
                .or_default() += 1;
            continue;
        }
        if let Some(channel) = extract_channel(
            detail,
            "note ",
            " on MIDI channel ",
            " collapsed to zero length after quantisation; forcing duration 1 tick",
        ) {
            *collapsed_by_context.entry((context, channel)).or_default() += 1;
            continue;
        }
        *passthrough_counts.entry(warning).or_default() += 1;
    }

    let mut out = Vec::new();
    for ((context, channel), count) in overlapping_by_context {
        out.push(apply_warning_context(
            context.as_deref(),
            format!(
            "{} overlapping {} {} dropped on MIDI channel {} because a later note took precedence after quantisation",
            count,
            if count == 1 { "note" } else { "notes" },
            if count == 1 { "was" } else { "were" },
            channel
            ),
        ));
    }
    for ((context, channel), count) in collapsed_by_context {
        out.push(apply_warning_context(
            context.as_deref(),
            format!(
            "{} {} on MIDI channel {} collapsed to zero length after quantisation and {} forced to 1 tick",
            count,
            if count == 1 { "note" } else { "notes" },
            channel,
            if count == 1 { "was" } else { "were" }
            ),
        ));
    }
    for (warning, count) in passthrough_counts {
        if count == 1 {
            out.push(warning);
        } else {
            out.push(format!("{warning} ({count} times)"));
        }
    }
    out
}

fn extract_channel(warning: &str, prefix: &str, mid: &str, suffix: &str) -> Option<u32> {
    let body = warning.strip_prefix(prefix)?;
    let (_, rest) = body.split_once(mid)?;
    let channel_text = rest.strip_suffix(suffix)?;
    channel_text.parse().ok()
}

fn split_warning_context(warning: &str) -> (Option<String>, &str) {
    if let Some((context, detail)) = warning.split_once(": ") {
        if context.starts_with("bank ") && context.contains(" song ") {
            return (Some(context.to_string()), detail);
        }
    }
    (None, warning)
}

fn apply_warning_context(context: Option<&str>, detail: String) -> String {
    match context {
        Some(context) => format!("{context}: {detail}"),
        None => detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_repeated_note_warnings() {
        let warnings = vec![
            "dropping note 70 on MIDI channel 3 because a later overlapping note quantised to the same harmony tick".to_string(),
            "dropping note 78 on MIDI channel 3 because a later overlapping note quantised to the same harmony tick".to_string(),
            "note 64 on MIDI channel 2 collapsed to zero length after quantisation; forcing duration 1 tick".to_string(),
            "5 events were quantised to whole harmony ticks".to_string(),
        ];

        let rendered = render_warnings(warnings, false);
        assert!(rendered.iter().any(|line| line == "2 overlapping notes were dropped on MIDI channel 3 because a later note took precedence after quantisation"));
        assert!(rendered.iter().any(|line| line == "1 note on MIDI channel 2 collapsed to zero length after quantisation and was forced to 1 tick"));
        assert!(
            rendered
                .iter()
                .any(|line| line == "5 events were quantised to whole harmony ticks")
        );
    }

    #[test]
    fn summarises_repeated_note_warnings_with_bank_song_context() {
        let warnings = vec![
            "bank 02 song 08: dropping note 70 on MIDI channel 3 because a later overlapping note quantised to the same harmony tick".to_string(),
            "bank 02 song 08: dropping note 78 on MIDI channel 3 because a later overlapping note quantised to the same harmony tick".to_string(),
        ];

        let rendered = render_warnings(warnings, false);
        assert!(rendered.iter().any(|line| line == "bank 02 song 08: 2 overlapping notes were dropped on MIDI channel 3 because a later note took precedence after quantisation"));
    }

    #[test]
    fn parses_short_command_names_and_flags() {
        let cli = Cli::try_parse_from([
            "harmony-midi",
            "-v",
            "build",
            "-c",
            "15",
            "8",
            "3",
            "-t",
            "24",
            "input",
            "output.bin",
        ])
        .unwrap();

        assert!(cli.verbose);
        match cli.command {
            Command::Build {
                channel_volumes,
                ticks_per_quarter,
                input_dir,
                output_firmware,
            } => {
                assert_eq!(channel_volumes, Some(vec![15, 8, 3]));
                assert_eq!(ticks_per_quarter, 24);
                assert_eq!(input_dir, PathBuf::from("input"));
                assert_eq!(output_firmware, PathBuf::from("output.bin"));
            }
            other => panic!("expected build command, got {other:?}"),
        }
    }

    #[test]
    fn rejects_removed_long_command_names() {
        let err =
            Cli::try_parse_from(["harmony-midi", "extract-firmware", "fw.bin", "out"]).unwrap_err();
        let rendered = err.to_string();
        assert!(rendered.contains("unrecognized subcommand"));
        assert!(rendered.contains("extract"));
    }
}
