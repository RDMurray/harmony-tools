mod furnace;

use anyhow::{Context, Result, anyhow, bail};
use furnace::{ParsedFurnaceModule, parse_furnace_bytes};
use midly::{
    Arena, Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent,
    TrackEventKind,
    num::{u4, u7, u15, u24, u28},
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

pub const BANK_SIZE: usize = 0x4000;
pub const SONGS_PER_BANK: usize = 16;
pub const POINTER_TABLE_OFFSET: usize = 0x01E0;
pub const POINTER_TABLE_LEN: usize = SONGS_PER_BANK * 2;
pub const NOTE_TABLE_OFFSET: usize = 0x0200;
pub const NOTE_TABLE_LEN: usize = 0x0100;
pub const DEFAULT_CODE_SECTION_LEN: usize = 0x0300;
pub const HARMONY_TICKS_PER_QUARTER: u16 = 32;
const PITCH_BEND_RANGE_SEMITONES: f64 = 2.0;
const CHANNEL_VOLUME_IMMEDIATE_OFFSETS: [usize; 3] = [0x0091, 0x00C7, 0x00FD];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarmonyRecord {
    pub idx: u8,
    pub ctl: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarmonySegment {
    pub idx: u8,
    pub duration: u32,
    pub voiced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarmonySong {
    pub stream: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarmonyBank {
    pub songs: Vec<HarmonySong>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarmonyFirmware {
    pub code_section: Vec<u8>,
    pub banks: Vec<HarmonyBank>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BankBuildStats {
    pub bank_index: usize,
    pub song_data_bytes: usize,
    pub used_bytes: usize,
    pub free_bytes: usize,
    pub unique_song_count: usize,
    pub aliased_song_slots: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildFirmwareReport {
    pub warnings: Vec<String>,
    pub code_section_bytes: usize,
    pub bank_capacity_bytes: usize,
    pub banks: Vec<BankBuildStats>,
}

#[derive(Clone, Debug)]
struct MidiImportInput {
    display_name: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct FurnaceImportInput {
    display_name: String,
    module: ParsedFurnaceModule,
}

#[derive(Clone, Debug)]
enum ImportedSongSource {
    Midi { input: MidiImportInput },
    Furnace { input: FurnaceImportInput },
}

#[derive(Clone, Debug)]
struct PlacedSong {
    stream: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct WarningCollector {
    warnings: Vec<String>,
    quantized_events: usize,
}

impl WarningCollector {
    pub fn new() -> Self {
        Self {
            warnings: Vec::new(),
            quantized_events: 0,
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    pub fn note_quantized_event(&mut self) {
        self.quantized_events += 1;
    }

    pub fn into_vec(mut self) -> Vec<String> {
        if self.quantized_events > 0 {
            self.warnings.push(format!(
                "{} events were quantised to whole harmony ticks",
                self.quantized_events
            ));
        }
        self.warnings
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SongFileId {
    pub bank_index: usize,
    pub song_index: usize,
}

#[derive(Clone, Debug)]
struct NoteMapping {
    valid_indices: Vec<u8>,
    idx_to_pitch: HashMap<u8, EncodedPitch>,
    idx_to_semitone: HashMap<u8, f64>,
    pitch_to_idx: HashMap<(u8, i16), u8>,
}

#[derive(Clone, Debug)]
struct ActiveSegment {
    remaining: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct PitchInterval {
    start: u32,
    end: u32,
    pitch_value: f64,
}

#[derive(Clone, Debug)]
struct MidiTimingMap {
    ppqn: u16,
    tempo_segments: Vec<TempoSegment>,
}

#[derive(Clone, Debug)]
struct TempoSegment {
    start_tick: u32,
    elapsed_quarter_micros: u128,
    micros_per_quarter: u32,
}

#[derive(Clone, Debug)]
enum OwnedMetaKind {
    TrackName,
}

#[derive(Clone, Debug)]
struct TrackBuild {
    events: Vec<(u32, u8, OwnedEventKind)>,
}

#[derive(Clone, Debug)]
enum OwnedEventKind {
    Midi { channel: u8, message: MidiMessage },
    MetaBytes { kind: OwnedMetaKind, bytes: Vec<u8> },
    MetaTempo(u24),
    MetaTimeSignature(u8, u8, u8, u8),
}

impl TrackBuild {
    fn new() -> Self {
        Self { events: Vec::new() }
    }

    fn push_track_name(&mut self, tick: u32, order: u8, text: impl Into<Vec<u8>>) {
        self.events.push((
            tick,
            order,
            OwnedEventKind::MetaBytes {
                kind: OwnedMetaKind::TrackName,
                bytes: text.into(),
            },
        ));
    }

    fn push_midi(&mut self, tick: u32, order: u8, channel: u8, message: MidiMessage) {
        self.events
            .push((tick, order, OwnedEventKind::Midi { channel, message }));
    }

    fn push_tempo(&mut self, tick: u32, order: u8, tempo: u24) {
        self.events
            .push((tick, order, OwnedEventKind::MetaTempo(tempo)));
    }

    fn push_time_signature(&mut self, tick: u32, order: u8, a: u8, b: u8, c: u8, d: u8) {
        self.events
            .push((tick, order, OwnedEventKind::MetaTimeSignature(a, b, c, d)));
    }

    fn into_track<'a>(self, arena: &'a Arena) -> Vec<TrackEvent<'a>> {
        let mut events = self.events;
        events.sort_by(|a, b| match a.0.cmp(&b.0) {
            Ordering::Equal => a.1.cmp(&b.1),
            other => other,
        });
        let mut last_tick = 0u32;
        let mut track = Vec::with_capacity(events.len() + 1);
        for (tick, _, kind) in events {
            let delta = tick.saturating_sub(last_tick);
            last_tick = tick;
            track.push(TrackEvent {
                delta: u28::new(delta),
                kind: kind.into_track_event(arena),
            });
        }
        track.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        track
    }
}

impl OwnedEventKind {
    fn into_track_event<'a>(self, arena: &'a Arena) -> TrackEventKind<'a> {
        match self {
            OwnedEventKind::Midi { channel, message } => TrackEventKind::Midi {
                channel: u4::new(channel),
                message,
            },
            OwnedEventKind::MetaBytes { kind, bytes } => match kind {
                OwnedMetaKind::TrackName => {
                    TrackEventKind::Meta(MetaMessage::TrackName(arena.add(&bytes)))
                }
            },
            OwnedEventKind::MetaTempo(tempo) => TrackEventKind::Meta(MetaMessage::Tempo(tempo)),
            OwnedEventKind::MetaTimeSignature(a, b, c, d) => {
                TrackEventKind::Meta(MetaMessage::TimeSignature(a, b, c, d))
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct EncodedPitch {
    midi_key: u8,
    bend_int: i16,
}

pub fn extract_firmware_to_dir(input_firmware: &Path, output_dir: &Path) -> Result<()> {
    let firmware = parse_firmware(input_firmware)?;
    fs::create_dir_all(output_dir).with_context(|| format!("creating {}", output_dir.display()))?;
    fs::write(output_dir.join("code.bin"), &firmware.code_section)
        .with_context(|| format!("writing {}", output_dir.join("code.bin").display()))?;
    for (bank_index, bank) in firmware.banks.iter().enumerate() {
        for (song_index, song) in bank.songs.iter().enumerate() {
            let midi = harmony_song_to_midi_bytes(&firmware.code_section, &song.stream)?;
            let filename = format!("bank{:02}_song{:02}.mid", bank_index + 1, song_index + 1);
            fs::write(output_dir.join(filename), midi).with_context(|| {
                format!(
                    "writing extracted midi for bank {} song {}",
                    bank_index + 1,
                    song_index + 1
                )
            })?;
        }
    }
    Ok(())
}

pub fn build_firmware_from_dir(
    input_dir: &Path,
    output_path: &Path,
) -> Result<BuildFirmwareReport> {
    build_firmware_from_dir_with_options(input_dir, output_path, None, HARMONY_TICKS_PER_QUARTER)
}

pub fn build_firmware_from_dir_with_options(
    input_dir: &Path,
    output_path: &Path,
    channel_volumes: Option<[u8; 3]>,
    ticks_per_quarter: u16,
) -> Result<BuildFirmwareReport> {
    let code_path = input_dir.join("code.bin");
    let mut original_code_section =
        fs::read(&code_path).with_context(|| format!("reading {}", code_path.display()))?;
    validate_code_section(&original_code_section)?;

    let mut warnings = WarningCollector::new();
    let mut discovered: BTreeMap<(usize, usize), ImportedSongSource> = BTreeMap::new();
    let mut midi_inputs = Vec::new();
    let mut furnace_inputs = Vec::new();
    for entry in
        fs::read_dir(input_dir).with_context(|| format!("reading {}", input_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some((id, kind)) = parse_song_input_filename(name) {
            let key = (id.bank_index, id.song_index);
            if discovered.contains_key(&key) {
                bail!(
                    "multiple input files claim bank {:02} song {:02}",
                    id.bank_index + 1,
                    id.song_index + 1
                );
            }
            match kind {
                SongInputKind::Midi => {
                    let input = MidiImportInput {
                        display_name: path.display().to_string(),
                        bytes: fs::read(&path)
                            .with_context(|| format!("reading {}", path.display()))?,
                    };
                    midi_inputs.push(input.clone());
                    discovered.insert(key, ImportedSongSource::Midi { input });
                }
                SongInputKind::Furnace => {
                    let bytes =
                        fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                    let mut parse_warnings = WarningCollector::new();
                    let module = parse_furnace_bytes(
                        &path.display().to_string(),
                        &bytes,
                        &mut parse_warnings,
                    )
                    .with_context(|| format!("decoding {}", path.display()))?;
                    for warning in parse_warnings.into_vec() {
                        warnings.warn(format!("{}: {}", path.display(), warning));
                    }
                    let input = FurnaceImportInput {
                        display_name: path.display().to_string(),
                        module,
                    };
                    furnace_inputs.push(input.clone());
                    discovered.insert(key, ImportedSongSource::Furnace { input });
                }
            }
        }
    }

    if discovered.is_empty() {
        bail!(
            "no song input files found in {} (expected names like bank01_song01.mid or bank01_song01.fur)",
            input_dir.display()
        );
    }

    if let Some(volumes) = channel_volumes {
        write_channel_volumes(&mut original_code_section, volumes)?;
    }
    let import_code_section = maybe_upgrade_code_for_imports(
        &original_code_section,
        &midi_inputs,
        &furnace_inputs,
        &mut warnings,
    )?;
    let note_mapping = build_note_mapping(&import_code_section)?;
    let song_data_start = import_code_section.len().max(DEFAULT_CODE_SECTION_LEN);
    if song_data_start > BANK_SIZE {
        bail!(
            "song data start 0x{:04X} exceeds bank size",
            song_data_start
        );
    }

    let mut placed_songs: BTreeMap<usize, BTreeMap<usize, PlacedSong>> = BTreeMap::new();
    let mut bank_data_usage = Vec::new();
    let mut global_fallback_stream: Option<Vec<u8>> = None;

    for ((bank_index, song_index), source) in discovered {
        let anchor = SongFileId {
            bank_index,
            song_index,
        };
        match source {
            ImportedSongSource::Midi { input } => {
                let mut song_warnings = WarningCollector::new();
                let stream = midi_bytes_to_harmony_stream_with_ticks(
                    &import_code_section,
                    &input.bytes,
                    &mut song_warnings,
                    ticks_per_quarter,
                )
                .with_context(|| format!("decoding {}", input.display_name))?;
                for warning in song_warnings.into_vec() {
                    warnings.warn(format!(
                        "bank {:02} song {:02}: {}",
                        anchor.bank_index + 1,
                        anchor.song_index + 1,
                        warning
                    ));
                }
                if global_fallback_stream.is_none() {
                    global_fallback_stream = Some(stream.clone());
                }
                place_imported_song(
                    &mut placed_songs,
                    &mut bank_data_usage,
                    anchor,
                    PlacedSong { stream },
                    song_data_start,
                    false,
                    &input.display_name,
                )?;
            }
            ImportedSongSource::Furnace { input } => {
                let mut next_slot = anchor.clone();
                for (subsong_index, subsong) in input.module.subsongs.iter().enumerate() {
                    let segments =
                        build_voice_segments_from_intervals(&subsong.voices, &note_mapping);
                    let stream = encode_voice_segments_to_stream(&segments);
                    if global_fallback_stream.is_none() {
                        global_fallback_stream = Some(stream.clone());
                    }
                    if subsong_index > 0 {
                        next_slot = next_song_slot(&next_slot);
                    }
                    let actual_slot = place_imported_song(
                        &mut placed_songs,
                        &mut bank_data_usage,
                        next_slot,
                        PlacedSong { stream },
                        song_data_start,
                        subsong_index > 0,
                        &subsong.display_name,
                    )?;
                    next_slot = actual_slot;
                }
            }
        }
    }

    let max_bank = placed_songs.keys().next_back().copied().unwrap_or(0);
    let mut firmware = Vec::with_capacity((max_bank + 1) * BANK_SIZE);
    let mut bank_stats = Vec::with_capacity(max_bank + 1);
    let fallback_stream = global_fallback_stream.ok_or_else(|| {
        anyhow!(
            "no decodable song input files found in {}",
            input_dir.display()
        )
    })?;
    for bank_index in 0..=max_bank {
        let bank_songs = placed_songs.get(&bank_index);
        let mut bank = vec![0xFFu8; BANK_SIZE];
        bank[..import_code_section.len()].copy_from_slice(&import_code_section);
        let mut cursor = song_data_start;
        let representative_index = (0..SONGS_PER_BANK).rfind(|&song_index| {
            bank_songs
                .and_then(|songs| songs.get(&song_index))
                .is_some()
        });
        let representative_stream = representative_index
            .and_then(|idx| bank_songs.and_then(|songs| songs.get(&idx)))
            .map(|song| &song.stream)
            .unwrap_or(&fallback_stream);
        if representative_index.is_none() {
            warnings.warn(format!(
                "bank {} has no imported songs; all 16 pointers will alias a copied fallback song",
                bank_index + 1
            ));
        } else if (0..SONGS_PER_BANK).any(|song_index| {
            bank_songs
                .and_then(|songs| songs.get(&song_index))
                .is_none()
        }) {
            warnings.warn(format!(
                "bank {} has missing song slots; absent entries will alias song {}",
                bank_index + 1,
                representative_index.unwrap() + 1
            ));
        }

        let mut representative_ptr: Option<u16> = None;
        let mut unique_song_count = 0usize;
        for song_index in 0..SONGS_PER_BANK {
            let song_stream = bank_songs.and_then(|songs| songs.get(&song_index));
            let ptr = if song_stream.is_none() || representative_index == Some(song_index) {
                if representative_ptr.is_none() {
                    if cursor + representative_stream.len() > BANK_SIZE {
                        bail!(
                            "bank overflow while placing representative song {}: need {} more bytes with {} bytes free",
                            representative_index.map(|idx| idx + 1).unwrap_or(1),
                            representative_stream.len(),
                            BANK_SIZE.saturating_sub(cursor)
                        );
                    }
                    representative_ptr = Some(u16::try_from(cursor).unwrap());
                    bank[cursor..cursor + representative_stream.len()]
                        .copy_from_slice(representative_stream);
                    cursor += representative_stream.len();
                    unique_song_count += 1;
                }
                representative_ptr.unwrap()
            } else {
                let song_stream = &song_stream.unwrap().stream;
                if cursor + song_stream.len() > BANK_SIZE {
                    bail!(
                        "bank overflow while placing song {}: need {} more bytes with {} bytes free",
                        song_index + 1,
                        song_stream.len(),
                        BANK_SIZE.saturating_sub(cursor)
                    );
                }
                let ptr = u16::try_from(cursor).unwrap();
                bank[cursor..cursor + song_stream.len()].copy_from_slice(song_stream);
                cursor += song_stream.len();
                unique_song_count += 1;
                ptr
            };
            let table_offset = POINTER_TABLE_OFFSET + song_index * 2;
            bank[table_offset..table_offset + 2].copy_from_slice(&ptr.to_le_bytes());
        }
        bank_stats.push(BankBuildStats {
            bank_index,
            song_data_bytes: cursor - song_data_start,
            used_bytes: cursor,
            free_bytes: BANK_SIZE - cursor,
            unique_song_count,
            aliased_song_slots: SONGS_PER_BANK - unique_song_count,
        });
        firmware.extend_from_slice(&bank);
    }

    fs::write(output_path, firmware)
        .with_context(|| format!("writing {}", output_path.display()))?;
    Ok(BuildFirmwareReport {
        warnings: warnings.into_vec(),
        code_section_bytes: import_code_section.len().max(DEFAULT_CODE_SECTION_LEN),
        bank_capacity_bytes: BANK_SIZE,
        banks: bank_stats,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SongInputKind {
    Midi,
    Furnace,
}

fn next_song_slot(current: &SongFileId) -> SongFileId {
    if current.song_index + 1 >= SONGS_PER_BANK {
        SongFileId {
            bank_index: current.bank_index + 1,
            song_index: 0,
        }
    } else {
        SongFileId {
            bank_index: current.bank_index,
            song_index: current.song_index + 1,
        }
    }
}

fn ensure_bank_usage(bank_data_usage: &mut Vec<usize>, bank_index: usize, song_data_start: usize) {
    if bank_data_usage.len() <= bank_index {
        bank_data_usage.resize(bank_index + 1, song_data_start);
    }
}

fn place_imported_song(
    placed_songs: &mut BTreeMap<usize, BTreeMap<usize, PlacedSong>>,
    bank_data_usage: &mut Vec<usize>,
    requested_slot: SongFileId,
    placed_song: PlacedSong,
    song_data_start: usize,
    allow_bank_carry: bool,
    source_name: &str,
) -> Result<SongFileId> {
    if song_data_start + placed_song.stream.len() > BANK_SIZE {
        bail!(
            "{} needs {} bytes of song data but only {} bytes are available in an empty bank",
            source_name,
            placed_song.stream.len(),
            BANK_SIZE.saturating_sub(song_data_start)
        );
    }

    let mut slot = requested_slot;
    loop {
        ensure_bank_usage(bank_data_usage, slot.bank_index, song_data_start);

        if bank_data_usage[slot.bank_index] + placed_song.stream.len() > BANK_SIZE {
            if !allow_bank_carry {
                bail!(
                    "{} does not fit in bank {:02}; need {} more bytes with {} bytes free",
                    source_name,
                    slot.bank_index + 1,
                    placed_song.stream.len(),
                    BANK_SIZE.saturating_sub(bank_data_usage[slot.bank_index])
                );
            }
            slot = SongFileId {
                bank_index: slot.bank_index + 1,
                song_index: 0,
            };
            if placed_songs
                .get(&slot.bank_index)
                .and_then(|songs| songs.get(&slot.song_index))
                .is_some()
            {
                bail!(
                    "{} cannot carry into bank {:02} song {:02} because that slot is already occupied",
                    source_name,
                    slot.bank_index + 1,
                    slot.song_index + 1
                );
            }
            continue;
        }

        let bank = placed_songs.entry(slot.bank_index).or_default();
        if bank.contains_key(&slot.song_index) {
            bail!(
                "{} conflicts with an existing assignment at bank {:02} song {:02}",
                source_name,
                slot.bank_index + 1,
                slot.song_index + 1
            );
        }
        bank_data_usage[slot.bank_index] += placed_song.stream.len();
        bank.insert(slot.song_index, placed_song);
        return Ok(slot);
    }
}

pub fn harmony_song_to_midi_file(
    code_path: &Path,
    song_path: &Path,
    output_path: &Path,
) -> Result<()> {
    let code = fs::read(code_path).with_context(|| format!("reading {}", code_path.display()))?;
    let song = fs::read(song_path).with_context(|| format!("reading {}", song_path.display()))?;
    let midi = harmony_song_to_midi_bytes(&code, &song)?;
    fs::write(output_path, midi).with_context(|| format!("writing {}", output_path.display()))?;
    Ok(())
}

pub fn midi_song_to_harmony_file(
    code_path: &Path,
    midi_path: &Path,
    output_path: &Path,
) -> Result<Vec<String>> {
    midi_song_to_harmony_file_with_ticks(
        code_path,
        midi_path,
        output_path,
        HARMONY_TICKS_PER_QUARTER,
    )
}

pub fn midi_song_to_harmony_file_with_ticks(
    code_path: &Path,
    midi_path: &Path,
    output_path: &Path,
    ticks_per_quarter: u16,
) -> Result<Vec<String>> {
    let code = fs::read(code_path).with_context(|| format!("reading {}", code_path.display()))?;
    validate_code_section(&code)?;
    let midi_bytes =
        fs::read(midi_path).with_context(|| format!("reading {}", midi_path.display()))?;
    let mut warnings = WarningCollector::new();
    let import_code = maybe_upgrade_code_for_midi_import(
        &code,
        &[MidiImportInput {
            display_name: midi_path.display().to_string(),
            bytes: midi_bytes.clone(),
        }],
        &mut warnings,
    )?;
    if import_code != code {
        warnings.warn(
            "song stream was encoded against an auto-promoted full MIDI note table; build will rewrite the firmware table automatically",
        );
    }

    let stream = midi_bytes_to_harmony_stream_with_ticks(
        &import_code,
        &midi_bytes,
        &mut warnings,
        ticks_per_quarter,
    )?;
    fs::write(output_path, stream).with_context(|| format!("writing {}", output_path.display()))?;
    Ok(warnings.into_vec())
}

pub fn parse_firmware(path: &Path) -> Result<HarmonyFirmware> {
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() % BANK_SIZE != 0 {
        bail!(
            "firmware size {} is not a multiple of 16 KiB banks",
            bytes.len()
        );
    }
    let bank_count = bytes.len() / BANK_SIZE;
    if bank_count != 2 && bank_count != 4 {
        bail!(
            "extraction currently supports the sample Harmony32/Harmony64 firmwares (2 or 4 banks), got {} banks",
            bank_count
        );
    }

    let mut banks = Vec::with_capacity(bank_count);
    let mut common_code: Option<Vec<u8>> = None;
    for bank_index in 0..bank_count {
        let start = bank_index * BANK_SIZE;
        let bank = &bytes[start..start + BANK_SIZE];
        let pointers = parse_pointer_table(bank)?;
        let code_len = usize::from(*pointers.first().unwrap());
        let code_section = bank[..code_len].to_vec();
        if let Some(existing) = &common_code {
            if existing != &code_section {
                bail!(
                    "bank {} code section differs from bank 0; a single extracted code.bin would not be sufficient",
                    bank_index + 1
                );
            }
        } else {
            common_code = Some(code_section);
        }

        let mut songs = Vec::with_capacity(SONGS_PER_BANK);
        for &ptr in &pointers {
            let ptr = usize::from(ptr);
            let end = bank[ptr..]
                .iter()
                .position(|&b| b == 0xFF)
                .map(|idx| ptr + idx + 1)
                .ok_or_else(|| anyhow!("song at 0x{:04X} is missing terminator", ptr))?;
            songs.push(HarmonySong {
                stream: bank[ptr..end].to_vec(),
            });
        }
        banks.push(HarmonyBank { songs });
    }

    Ok(HarmonyFirmware {
        code_section: common_code.unwrap_or_default(),
        banks,
    })
}

fn parse_pointer_table(bank: &[u8]) -> Result<Vec<u16>> {
    if bank.len() != BANK_SIZE {
        bail!("expected a single 16 KiB bank, got {}", bank.len());
    }
    let mut pointers = Vec::with_capacity(SONGS_PER_BANK);
    for song_index in 0..SONGS_PER_BANK {
        let off = POINTER_TABLE_OFFSET + song_index * 2;
        let ptr = u16::from_le_bytes([bank[off], bank[off + 1]]);
        let ptr_usize = usize::from(ptr);
        if !(DEFAULT_CODE_SECTION_LEN..BANK_SIZE).contains(&ptr_usize) {
            bail!(
                "pointer {} has invalid address 0x{:04X}",
                song_index + 1,
                ptr
            );
        }
        pointers.push(ptr);
    }
    if pointers.windows(2).any(|w| w[0] > w[1]) {
        bail!("song pointers are not nondecreasing");
    }
    Ok(pointers)
}

fn parse_song_input_filename(name: &str) -> Option<(SongFileId, SongInputKind)> {
    let (stem, kind) = if let Some(stem) = name
        .strip_suffix(".mid")
        .or_else(|| name.strip_suffix(".MID"))
    {
        (stem, SongInputKind::Midi)
    } else if let Some(stem) = name
        .strip_suffix(".fur")
        .or_else(|| name.strip_suffix(".FUR"))
    {
        (stem, SongInputKind::Furnace)
    } else {
        return None;
    };
    let (bank_part, song_part) = stem.split_once('_')?;
    let bank = bank_part
        .strip_prefix("bank")
        .or_else(|| bank_part.strip_prefix("BANK"))?
        .parse::<usize>()
        .ok()?;
    let song = song_part
        .strip_prefix("song")
        .or_else(|| song_part.strip_prefix("SONG"))?
        .parse::<usize>()
        .ok()?;
    if bank == 0 || song == 0 {
        return None;
    }
    Some(SongFileId {
        bank_index: bank - 1,
        song_index: song - 1,
    })
    .map(|id| (id, kind))
}

#[cfg(test)]
fn parse_song_filename(name: &str) -> Option<SongFileId> {
    let (id, kind) = parse_song_input_filename(name)?;
    if kind == SongInputKind::Midi {
        Some(id)
    } else {
        None
    }
}

fn validate_code_section(code_section: &[u8]) -> Result<()> {
    if code_section.len() > BANK_SIZE {
        bail!(
            "code section is {} bytes, larger than 16 KiB bank size",
            code_section.len()
        );
    }
    if code_section.len() < NOTE_TABLE_OFFSET + NOTE_TABLE_LEN {
        bail!(
            "code section must include the note table through offset 0x{:04X}",
            NOTE_TABLE_OFFSET + NOTE_TABLE_LEN
        );
    }
    Ok(())
}

fn load_note_table(code: &[u8]) -> Result<[u16; 128]> {
    if code.len() < NOTE_TABLE_OFFSET + NOTE_TABLE_LEN {
        bail!(
            "code section must be at least {} bytes to include the note table",
            NOTE_TABLE_OFFSET + NOTE_TABLE_LEN
        );
    }
    let mut table = [0u16; 128];
    for (i, slot) in table.iter_mut().enumerate() {
        let off = NOTE_TABLE_OFFSET + i * 2;
        *slot = u16::from_le_bytes([code[off], code[off + 1]]);
    }
    Ok(table)
}

fn write_note_table(code: &mut [u8], table: &[u16; 128]) -> Result<()> {
    validate_code_section(code)?;
    for (i, value) in table.iter().enumerate() {
        let off = NOTE_TABLE_OFFSET + i * 2;
        code[off..off + 2].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

fn read_channel_volumes(code: &[u8]) -> Result<[u8; 3]> {
    validate_code_section(code)?;
    let mut volumes = [0u8; 3];
    for (slot, immediate_offset) in volumes.iter_mut().zip(CHANNEL_VOLUME_IMMEDIATE_OFFSETS) {
        let opcode_offset = immediate_offset
            .checked_sub(1)
            .ok_or_else(|| anyhow!("invalid channel volume offset 0x{immediate_offset:04X}"))?;
        if code[opcode_offset] != 0x3E {
            bail!(
                "expected LD A,n opcode before channel volume immediate at 0x{:04X}",
                immediate_offset
            );
        }
        *slot = code[immediate_offset] & 0x0F;
    }
    Ok(volumes)
}

fn write_channel_volumes(code: &mut [u8], volumes: [u8; 3]) -> Result<()> {
    validate_code_section(code)?;
    for (volume, immediate_offset) in volumes.into_iter().zip(CHANNEL_VOLUME_IMMEDIATE_OFFSETS) {
        if volume > 0x0F {
            bail!("channel volumes must be in the range 0..15");
        }
        let opcode_offset = immediate_offset
            .checked_sub(1)
            .ok_or_else(|| anyhow!("invalid channel volume offset 0x{immediate_offset:04X}"))?;
        if code[opcode_offset] != 0x3E {
            bail!(
                "expected LD A,n opcode before channel volume immediate at 0x{:04X}",
                immediate_offset
            );
        }
        code[immediate_offset] = volume;
    }
    Ok(())
}

fn build_complete_midi_note_table() -> [u16; 128] {
    let mut table = [0u16; 128];
    for (midi_note, slot) in table.iter_mut().enumerate() {
        let freq = 440.0 * 2f64.powf((midi_note as f64 - 69.0) / 12.0);
        let period = (2_000_000.0 / (16.0 * freq)).round();
        *slot = period.clamp(1.0, f64::from(u16::MAX)) as u16;
    }
    table
}

fn maybe_upgrade_code_for_imports(
    code: &[u8],
    midi_inputs: &[MidiImportInput],
    furnace_inputs: &[FurnaceImportInput],
    warnings: &mut WarningCollector,
) -> Result<Vec<u8>> {
    let note_mapping = build_note_mapping(code)?;
    let (min_pitch, max_pitch) = note_mapping_pitch_bounds(&note_mapping)?;
    let mut requires_full_table = false;

    for midi in midi_inputs {
        if midi_uses_pitch_outside_range(&midi.bytes, min_pitch, max_pitch)? {
            warnings.warn(format!(
                "rewriting note table to a full 128-note MIDI table because {} uses notes outside the current table range",
                midi.display_name
            ));
            requires_full_table = true;
            break;
        }
    }

    if !requires_full_table {
        for furnace in furnace_inputs {
            let Some((source_min, source_max)) = furnace.module.pitch_bounds else {
                continue;
            };
            if source_min < min_pitch || source_max > max_pitch {
                warnings.warn(format!(
                    "rewriting note table to a full 128-note MIDI table because {} uses notes outside the current table range",
                    furnace.display_name
                ));
                requires_full_table = true;
                break;
            }
        }
    }

    if !requires_full_table {
        return Ok(code.to_vec());
    }

    let mut upgraded = code.to_vec();
    let table = build_complete_midi_note_table();
    write_note_table(&mut upgraded, &table)?;
    Ok(upgraded)
}

fn maybe_upgrade_code_for_midi_import(
    code: &[u8],
    midi_inputs: &[MidiImportInput],
    warnings: &mut WarningCollector,
) -> Result<Vec<u8>> {
    maybe_upgrade_code_for_imports(code, midi_inputs, &[], warnings)
}

fn note_mapping_pitch_bounds(note_mapping: &NoteMapping) -> Result<(f64, f64)> {
    let mut values = note_mapping.idx_to_semitone.values().copied();
    let Some(first) = values.next() else {
        bail!("note table contains no usable notes");
    };
    let mut min_pitch = first;
    let mut max_pitch = first;
    for value in values {
        min_pitch = min_pitch.min(value);
        max_pitch = max_pitch.max(value);
    }
    Ok((min_pitch, max_pitch))
}

fn midi_uses_pitch_outside_range(
    midi_bytes: &[u8],
    min_pitch: f64,
    max_pitch: f64,
) -> Result<bool> {
    let smf = Smf::parse(midi_bytes).context("parsing MIDI file")?;
    let mut bend_by_channel = [0i16; 16];
    for track in &smf.tracks {
        for event in track {
            if let TrackEventKind::Midi { channel, message } = event.kind {
                let channel = channel.as_int() as usize;
                match message {
                    MidiMessage::PitchBend { bend } => bend_by_channel[channel] = bend.as_int(),
                    MidiMessage::NoteOn { key, vel } if vel > u7::new(0) => {
                        let pitch = f64::from(key.as_int())
                            + (f64::from(bend_by_channel[channel]) / 8192.0
                                * PITCH_BEND_RANGE_SEMITONES);
                        if pitch < min_pitch || pitch > max_pitch {
                            return Ok(true);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(false)
}

fn build_note_mapping(code: &[u8]) -> Result<NoteMapping> {
    let table = load_note_table(code)?;
    let mut valid_indices = Vec::new();
    let mut idx_to_pitch = HashMap::new();
    let mut idx_to_semitone = HashMap::new();
    let mut pitch_to_idx = HashMap::new();
    for idx in (0u8..=u8::MAX).step_by(2) {
        let period = table[(idx / 2) as usize];
        if period == 0 {
            continue;
        }
        let (semi, pitch) = period_to_encoded_pitch(period);
        pitch_to_idx
            .entry((pitch.midi_key, pitch.bend_int))
            .or_insert(idx);
        valid_indices.push(idx);
        idx_to_pitch.insert(idx, pitch);
        idx_to_semitone.insert(idx, semi);
    }
    if valid_indices.is_empty() {
        bail!("note table contains no usable pitch entries");
    }
    Ok(NoteMapping {
        valid_indices,
        idx_to_pitch,
        idx_to_semitone,
        pitch_to_idx,
    })
}

fn period_to_semitone(period: u16) -> f64 {
    let freq = 2_000_000.0f64 / (16.0 * f64::from(period));
    69.0 + 12.0 * (freq / 440.0).log2()
}

fn period_to_encoded_pitch(period: u16) -> (f64, EncodedPitch) {
    let semi = period_to_semitone(period);
    let midi_key = semi.round().clamp(0.0, 127.0) as u8;
    let diff = semi - f64::from(midi_key);
    let bend_int = ((diff / PITCH_BEND_RANGE_SEMITONES) * 8192.0)
        .round()
        .clamp(-8192.0, 8191.0) as i16;
    (semi, EncodedPitch { midi_key, bend_int })
}

fn nearest_idx_for_pitch(note_mapping: &NoteMapping, pitch_value: f64) -> u8 {
    note_mapping
        .valid_indices
        .iter()
        .copied()
        .min_by_key(|idx| {
            let mapped = *note_mapping.idx_to_semitone.get(idx).unwrap();
            ((mapped - pitch_value).abs() * 1_000_000.0) as i64
        })
        .unwrap()
}

pub fn harmony_song_to_midi_bytes(code: &[u8], song_stream: &[u8]) -> Result<Vec<u8>> {
    let note_mapping = build_note_mapping(code)?;
    let channel_volumes = read_channel_volumes(code)?;
    let voice_segments = decode_stream_to_voice_segments(song_stream)?;
    let arena = Arena::new();
    let mut meta_track = TrackBuild::new();
    meta_track.push_track_name(0, 0, b"Harmony Stream".to_vec());
    meta_track.push_tempo(0, 1, u24::new(500_000));
    meta_track.push_time_signature(0, 2, 4, 2, 24, 8);

    let mut tracks = Vec::with_capacity(4);
    tracks.push(meta_track.into_track(&arena));

    for (channel_index, segments) in voice_segments.iter().enumerate() {
        let mut track = TrackBuild::new();
        track.push_track_name(
            0,
            0,
            format!("Harmony {}", char::from(b'A' + channel_index as u8)).into_bytes(),
        );
        push_pitch_bend_range_setup(&mut track, channel_index as u8);
        let mut abs_tick = 0u32;
        for segment in segments {
            let pitch = *note_mapping
                .idx_to_pitch
                .get(&segment.idx)
                .ok_or_else(|| anyhow!("no MIDI pitch mapping for idx 0x{:02X}", segment.idx))?;
            let velocity = if segment.voiced {
                midi_velocity_for_volume(channel_volumes[channel_index])
            } else {
                0
            };
            track.push_midi(
                abs_tick,
                1,
                channel_index as u8,
                MidiMessage::PitchBend {
                    bend: PitchBend::from_int(pitch.bend_int),
                },
            );
            track.push_midi(
                abs_tick,
                2,
                channel_index as u8,
                MidiMessage::NoteOn {
                    key: u7::new(pitch.midi_key),
                    vel: u7::new(velocity),
                },
            );
            track.push_midi(
                abs_tick + segment.duration,
                0,
                channel_index as u8,
                MidiMessage::NoteOff {
                    key: u7::new(pitch.midi_key),
                    vel: u7::new(0),
                },
            );
            abs_tick += segment.duration;
        }
        tracks.push(track.into_track(&arena));
    }

    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(u15::new(HARMONY_TICKS_PER_QUARTER)),
        },
        tracks,
    };
    let mut out = Vec::new();
    smf.write_std(&mut out)?;
    Ok(out)
}

fn push_pitch_bend_range_setup(track: &mut TrackBuild, channel: u8) {
    let setup = [
        (0u32, 0u8, 101u8, 0u8),
        (0, 1, 100, 0),
        (0, 2, 6, PITCH_BEND_RANGE_SEMITONES as u8),
        (0, 3, 38, 0),
        (0, 4, 101, 127),
        (0, 5, 100, 127),
    ];
    for (tick, order, controller, value) in setup {
        track.push_midi(
            tick,
            order,
            channel,
            MidiMessage::Controller {
                controller: u7::new(controller),
                value: u7::new(value),
            },
        );
    }
}

fn midi_velocity_for_volume(volume: u8) -> u8 {
    (((u16::from(volume) * 127) + 7) / 15) as u8
}

pub fn midi_bytes_to_harmony_stream(
    code: &[u8],
    midi_bytes: &[u8],
    warnings: &mut WarningCollector,
) -> Result<Vec<u8>> {
    midi_bytes_to_harmony_stream_with_ticks(code, midi_bytes, warnings, HARMONY_TICKS_PER_QUARTER)
}

fn midi_bytes_to_harmony_stream_with_ticks(
    code: &[u8],
    midi_bytes: &[u8],
    warnings: &mut WarningCollector,
    ticks_per_quarter: u16,
) -> Result<Vec<u8>> {
    let smf = Smf::parse(midi_bytes).context("parsing MIDI file")?;
    let note_mapping = build_note_mapping(code)?;
    if let Some(segments) = decode_canonical_channel_segments(&smf, &note_mapping)? {
        return Ok(encode_voice_segments_to_stream(&segments));
    }

    let voices = collect_channel_intervals(&smf, warnings, ticks_per_quarter)?;
    let segments = build_voice_segments_from_intervals(&voices, &note_mapping);
    Ok(encode_voice_segments_to_stream(&segments))
}

#[derive(Clone, Debug)]
enum CanonicalChannelEvent {
    PitchBend(i16),
    NoteOn { key: u8, vel: u8 },
    NoteOff { key: u8 },
}

fn decode_canonical_channel_segments(
    smf: &Smf<'_>,
    note_mapping: &NoteMapping,
) -> Result<Option<[Vec<HarmonySegment>; 3]>> {
    let mut events: [Vec<(u32, u8, CanonicalChannelEvent)>; 3] =
        [Vec::new(), Vec::new(), Vec::new()];
    let mut saw_channel_note = false;
    let mut saw_noncanonical_note = false;

    for track in &smf.tracks {
        let mut abs_tick = 0u32;
        for event in track {
            abs_tick = abs_tick.saturating_add(event.delta.as_int());
            if let TrackEventKind::Midi { channel, message } = event.kind {
                let channel = channel.as_int();
                let order = match message {
                    MidiMessage::NoteOff { .. } => 0,
                    MidiMessage::PitchBend { .. } => 1,
                    MidiMessage::NoteOn { .. } => 2,
                    _ => 3,
                };
                match message {
                    MidiMessage::PitchBend { bend } if channel < 3 => {
                        events[channel as usize].push((
                            abs_tick,
                            order,
                            CanonicalChannelEvent::PitchBend(bend.as_int()),
                        ));
                    }
                    MidiMessage::NoteOn { key, vel } if channel < 3 => {
                        saw_channel_note = true;
                        events[channel as usize].push((
                            abs_tick,
                            order,
                            CanonicalChannelEvent::NoteOn {
                                key: key.as_int(),
                                vel: vel.as_int(),
                            },
                        ));
                    }
                    MidiMessage::NoteOff { key, .. } if channel < 3 => {
                        saw_channel_note = true;
                        events[channel as usize].push((
                            abs_tick,
                            order,
                            CanonicalChannelEvent::NoteOff { key: key.as_int() },
                        ));
                    }
                    MidiMessage::PitchBend { .. }
                    | MidiMessage::NoteOn { .. }
                    | MidiMessage::NoteOff { .. } => {
                        saw_noncanonical_note = true;
                    }
                    _ => {}
                }
            }
        }
    }

    if !saw_channel_note || saw_noncanonical_note {
        return Ok(None);
    }

    let mut voices: [Vec<HarmonySegment>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (voice_idx, channel_events) in events.iter_mut().enumerate() {
        channel_events.sort_by(|a, b| match a.0.cmp(&b.0) {
            Ordering::Equal => a.1.cmp(&b.1),
            other => other,
        });
        let mut current_bend = 0i16;
        let mut active: Option<(u32, u8, u8, i16)> = None;
        for (tick, _, event) in channel_events.iter() {
            match *event {
                CanonicalChannelEvent::PitchBend(bend) => current_bend = bend,
                CanonicalChannelEvent::NoteOn { key, vel } => {
                    if active.is_some() {
                        return Ok(None);
                    }
                    active = Some((*tick, key, vel, current_bend));
                }
                CanonicalChannelEvent::NoteOff { key } => {
                    let Some((start_tick, active_key, vel, bend_int)) = active.take() else {
                        return Ok(None);
                    };
                    if active_key != key || *tick <= start_tick {
                        return Ok(None);
                    }
                    let Some(idx) = note_mapping
                        .pitch_to_idx
                        .get(&(active_key, bend_int))
                        .copied()
                    else {
                        return Ok(None);
                    };
                    voices[voice_idx].push(HarmonySegment {
                        idx,
                        duration: tick - start_tick,
                        voiced: vel > 0,
                    });
                }
            }
        }
        if active.is_some() {
            return Ok(None);
        }
    }
    Ok(Some(voices))
}

fn collect_channel_intervals(
    smf: &Smf<'_>,
    warnings: &mut WarningCollector,
    ticks_per_quarter: u16,
) -> Result<[Vec<PitchInterval>; 3]> {
    let timing_map = MidiTimingMap::from_smf(smf)?;
    let mut events: [Vec<(u32, usize, CanonicalChannelEvent)>; 3] =
        [Vec::new(), Vec::new(), Vec::new()];
    let mut ignored_channel_warned = [false; 16];
    let mut max_tick = 0u32;

    for (track_index, track) in smf.tracks.iter().enumerate() {
        let mut abs_tick = 0u32;
        for (event_index, event) in track.iter().enumerate() {
            abs_tick = abs_tick.saturating_add(event.delta.as_int());
            max_tick = max_tick.max(abs_tick);
            if let TrackEventKind::Midi { channel, message } = event.kind {
                let channel = channel.as_int() as usize;
                match message {
                    MidiMessage::PitchBend { bend } if channel < 3 => events[channel].push((
                        abs_tick,
                        track_index * 1_000_000 + event_index,
                        CanonicalChannelEvent::PitchBend(bend.as_int()),
                    )),
                    MidiMessage::NoteOn { key, vel } if vel > u7::new(0) && channel < 3 => {
                        events[channel].push((
                            abs_tick,
                            track_index * 1_000_000 + event_index,
                            CanonicalChannelEvent::NoteOn {
                                key: key.as_int(),
                                vel: vel.as_int(),
                            },
                        ));
                    }
                    MidiMessage::NoteOff { key, .. } if channel < 3 => events[channel].push((
                        abs_tick,
                        track_index * 1_000_000 + event_index,
                        CanonicalChannelEvent::NoteOff { key: key.as_int() },
                    )),
                    MidiMessage::NoteOn { key, vel } if vel == u7::new(0) && channel < 3 => {
                        events[channel].push((
                            abs_tick,
                            track_index * 1_000_000 + event_index,
                            CanonicalChannelEvent::NoteOff { key: key.as_int() },
                        ));
                    }
                    MidiMessage::PitchBend { .. }
                    | MidiMessage::NoteOn { .. }
                    | MidiMessage::NoteOff { .. }
                        if channel >= 3 =>
                    {
                        if !ignored_channel_warned[channel] {
                            warnings.warn(format!(
                                "ignoring MIDI channel {} because only channels 1, 2, and 3 map to Harmony voices A, B, and C",
                                channel + 1
                            ));
                            ignored_channel_warned[channel] = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut voices: [Vec<PitchInterval>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (channel_index, channel_events) in events.iter_mut().enumerate() {
        channel_events.sort_by(|a, b| match a.0.cmp(&b.0) {
            Ordering::Equal => a.1.cmp(&b.1),
            other => other,
        });
        let mut current_bend = 0i16;
        let mut active: Option<(u8, u32, f64)> = None;
        for (tick, _, event) in channel_events.iter() {
            match *event {
                CanonicalChannelEvent::PitchBend(bend) => current_bend = bend,
                CanonicalChannelEvent::NoteOn { key, .. } => {
                    let start_q = timing_map.quantize_tick(*tick, warnings, ticks_per_quarter);
                    if let Some((previous_key, previous_start_tick, previous_pitch)) = active.take()
                    {
                        let previous_start_q = timing_map.quantize_tick(
                            previous_start_tick,
                            warnings,
                            ticks_per_quarter,
                        );
                        if start_q > previous_start_q {
                            voices[channel_index].push(PitchInterval {
                                start: previous_start_q,
                                end: start_q,
                                pitch_value: previous_pitch,
                            });
                        } else {
                            warnings.warn(format!(
                                "dropping note {} on MIDI channel {} because a later overlapping note quantised to the same harmony tick",
                                previous_key,
                                channel_index + 1
                            ));
                        }
                    }
                    let pitch_value = f64::from(key)
                        + (f64::from(current_bend) / 8192.0 * PITCH_BEND_RANGE_SEMITONES);
                    active = Some((key, *tick, pitch_value));
                }
                CanonicalChannelEvent::NoteOff { key } => {
                    let Some((active_key, start_tick, pitch_value)) = active else {
                        continue;
                    };
                    if active_key != key {
                        continue;
                    }
                    active = None;
                    let start_q = timing_map.quantize_tick(start_tick, warnings, ticks_per_quarter);
                    let mut end_q = timing_map.quantize_tick(*tick, warnings, ticks_per_quarter);
                    if end_q <= start_q {
                        warnings.warn(format!(
                            "note {} on MIDI channel {} collapsed to zero length after quantisation; forcing duration 1 tick",
                            key,
                            channel_index + 1
                        ));
                        end_q = start_q + 1;
                    }
                    voices[channel_index].push(PitchInterval {
                        start: start_q,
                        end: end_q,
                        pitch_value,
                    });
                }
            }
        }
        if let Some((_, start_tick, pitch_value)) = active {
            let start_q = timing_map.quantize_tick(start_tick, warnings, ticks_per_quarter);
            let mut end_q = timing_map.quantize_tick(max_tick, warnings, ticks_per_quarter);
            if end_q <= start_q {
                end_q = start_q + 1;
            }
            voices[channel_index].push(PitchInterval {
                start: start_q,
                end: end_q,
                pitch_value,
            });
        }
    }
    Ok(voices)
}

impl MidiTimingMap {
    fn from_smf(smf: &Smf<'_>) -> Result<Self> {
        let ppqn = match smf.header.timing {
            Timing::Metrical(ppqn) => ppqn.as_int(),
            Timing::Timecode(_, _) => bail!("SMPTE-timed MIDI files are not supported"),
        };

        let mut tempo_events = Vec::new();
        for (track_index, track) in smf.tracks.iter().enumerate() {
            let mut abs_tick = 0u32;
            for (event_index, event) in track.iter().enumerate() {
                abs_tick = abs_tick.saturating_add(event.delta.as_int());
                if let TrackEventKind::Meta(MetaMessage::Tempo(tempo)) = event.kind {
                    tempo_events.push((
                        abs_tick,
                        track_index * 1_000_000 + event_index,
                        tempo.as_int(),
                    ));
                }
            }
        }
        tempo_events.sort_by(|a, b| match a.0.cmp(&b.0) {
            Ordering::Equal => a.1.cmp(&b.1),
            other => other,
        });

        let mut tempo_segments = vec![TempoSegment {
            start_tick: 0,
            elapsed_quarter_micros: 0,
            micros_per_quarter: 500_000,
        }];
        for (tick, _, micros_per_quarter) in tempo_events {
            let last = tempo_segments.last_mut().unwrap();
            if tick == last.start_tick {
                last.micros_per_quarter = micros_per_quarter;
                continue;
            }
            let elapsed_quarter_micros = last.elapsed_quarter_micros
                + u128::from(tick - last.start_tick) * u128::from(last.micros_per_quarter);
            tempo_segments.push(TempoSegment {
                start_tick: tick,
                elapsed_quarter_micros,
                micros_per_quarter,
            });
        }

        Ok(Self {
            ppqn,
            tempo_segments,
        })
    }

    fn quantize_tick(
        &self,
        value: u32,
        warnings: &mut WarningCollector,
        ticks_per_quarter: u16,
    ) -> u32 {
        let segment_index = self
            .tempo_segments
            .partition_point(|segment| segment.start_tick <= value)
            .saturating_sub(1);
        let segment = &self.tempo_segments[segment_index];
        let elapsed_quarter_micros = segment.elapsed_quarter_micros
            + u128::from(value - segment.start_tick) * u128::from(segment.micros_per_quarter);
        let numerator = elapsed_quarter_micros * u128::from(ticks_per_quarter);
        let denominator = u128::from(self.ppqn) * 500_000u128;
        let rounded = ((numerator * 2 + denominator) / (2 * denominator)) as u32;
        if numerator % denominator != 0 {
            warnings.note_quantized_event();
        }
        rounded
    }
}

fn build_voice_segments_from_intervals(
    voices: &[Vec<PitchInterval>; 3],
    note_mapping: &NoteMapping,
) -> [Vec<HarmonySegment>; 3] {
    let song_end = voices
        .iter()
        .flat_map(|voice| voice.iter().map(|interval| interval.end))
        .max()
        .unwrap_or(0);

    let mut out: [Vec<HarmonySegment>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (voice_index, intervals) in voices.iter().enumerate() {
        let mut cursor = 0u32;
        if intervals.is_empty() {
            if song_end > 0 {
                push_segment_split(
                    &mut out[voice_index],
                    HarmonySegment {
                        idx: 0x00,
                        duration: song_end,
                        voiced: false,
                    },
                );
            }
            continue;
        }
        for interval in intervals {
            if interval.start > cursor {
                push_segment_split(
                    &mut out[voice_index],
                    HarmonySegment {
                        idx: 0x00,
                        duration: interval.start - cursor,
                        voiced: false,
                    },
                );
            }
            push_interval_segment(
                &mut out[voice_index],
                HarmonySegment {
                    idx: nearest_idx_for_pitch(note_mapping, interval.pitch_value),
                    duration: interval.end - interval.start,
                    voiced: true,
                },
            );
            cursor = interval.end;
        }
        if cursor < song_end {
            push_segment_split(
                &mut out[voice_index],
                HarmonySegment {
                    idx: 0x00,
                    duration: song_end - cursor,
                    voiced: false,
                },
            );
        }
    }
    out
}

fn push_interval_segment(target: &mut Vec<HarmonySegment>, segment: HarmonySegment) {
    if !segment.voiced || segment.duration <= 0x7F {
        push_segment_split(target, segment);
        return;
    }

    target.push(HarmonySegment {
        idx: segment.idx,
        duration: 0x7F,
        voiced: true,
    });
    push_segment_split(
        target,
        HarmonySegment {
            idx: 0x00,
            duration: segment.duration - 0x7F,
            voiced: false,
        },
    );
}

fn push_segment_split(target: &mut Vec<HarmonySegment>, segment: HarmonySegment) {
    if segment.duration == 0 {
        return;
    }
    let mut remaining = segment.duration;
    while remaining > 0 {
        let chunk = remaining.min(0x7F);
        target.push(HarmonySegment {
            idx: segment.idx,
            duration: chunk,
            voiced: segment.voiced,
        });
        remaining -= chunk;
    }
}

fn encode_voice_segments_to_stream(voices: &[Vec<HarmonySegment>; 3]) -> Vec<u8> {
    if voices.iter().all(|voice| voice.is_empty()) {
        return vec![0xFF];
    }

    let mut indices = [0usize; 3];
    let mut remaining = [0u32; 3];
    let mut out = Vec::new();

    for ch in 0..3 {
        if voices[ch].is_empty() {
            out.push(0xFF);
            return out;
        }
        let seg = &voices[ch][0];
        indices[ch] = 1;
        remaining[ch] = seg.duration;
        out.push(seg.idx);
        out.push(build_ctl(seg.duration as u8, seg.voiced));
    }

    loop {
        let step = *remaining
            .iter()
            .filter(|&&value| value > 0)
            .min()
            .unwrap_or(&0);
        if step == 0 {
            out.push(0xFF);
            return out;
        }
        for rem in &mut remaining {
            *rem -= step;
        }
        for ch in 0..3 {
            if remaining[ch] == 0 {
                if indices[ch] >= voices[ch].len() {
                    out.push(0xFF);
                    return out;
                }
                let seg = &voices[ch][indices[ch]];
                indices[ch] += 1;
                remaining[ch] = seg.duration;
                out.push(seg.idx);
                out.push(build_ctl(seg.duration as u8, seg.voiced));
            }
        }
    }
}

fn build_ctl(duration: u8, voiced: bool) -> u8 {
    let base = duration & 0x7F;
    if voiced { base | 0x80 } else { base }
}

fn parse_stream_records(song_stream: &[u8]) -> Result<Vec<HarmonyRecord>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < song_stream.len() {
        let idx = song_stream[i];
        if idx == 0xFF {
            return Ok(out);
        }
        if i + 1 >= song_stream.len() {
            bail!("stream ended mid-record at byte {}", i);
        }
        let ctl = song_stream[i + 1];
        out.push(HarmonyRecord { idx, ctl });
        i += 2;
    }
    bail!("stream is missing 0xFF terminator")
}

fn decode_stream_to_voice_segments(song_stream: &[u8]) -> Result<[Vec<HarmonySegment>; 3]> {
    let records = parse_stream_records(song_stream)?;
    let mut voices: [Vec<HarmonySegment>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut active: [ActiveSegment; 3] = [
        ActiveSegment { remaining: 0 },
        ActiveSegment { remaining: 0 },
        ActiveSegment { remaining: 0 },
    ];
    let mut cursor = 0usize;

    for channel in 0..3 {
        let Some(record) = records.get(cursor) else {
            return Ok(voices);
        };
        cursor += 1;
        let segment = record_to_segment(record);
        active[channel].remaining = segment.duration;
        voices[channel].push(segment);
    }

    loop {
        for channel in 0..3 {
            if active[channel].remaining == 0 {
                let Some(record) = records.get(cursor) else {
                    return Ok(voices);
                };
                cursor += 1;
                let segment = record_to_segment(record);
                active[channel].remaining = segment.duration;
                voices[channel].push(segment);
            }
        }
        let step = active
            .iter()
            .map(|seg| seg.remaining)
            .filter(|&dur| dur > 0)
            .min()
            .unwrap_or(0);
        if step == 0 {
            return Ok(voices);
        }
        for channel in 0..3 {
            active[channel].remaining -= step;
        }
    }
}

fn record_to_segment(record: &HarmonyRecord) -> HarmonySegment {
    HarmonySegment {
        idx: record.idx,
        duration: u32::from(record.ctl & 0x7F),
        voiced: (record.ctl & 0x80) != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::furnace::parse_furnace_bytes;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_code() -> Vec<u8> {
        let mut code = vec![0u8; DEFAULT_CODE_SECTION_LEN];
        for immediate_offset in CHANNEL_VOLUME_IMMEDIATE_OFFSETS {
            code[immediate_offset - 1] = 0x3E;
        }
        write_channel_volumes(&mut code, [15, 12, 9]).unwrap();

        let mut table = [0u16; 128];
        for (i, slot) in table.iter_mut().take(96).enumerate() {
            let midi_note = 12 + i as u8;
            let freq = 440.0 * 2f64.powf((f64::from(midi_note) - 69.0) / 12.0);
            *slot = (2_000_000.0 / (16.0 * freq)).round() as u16;
        }
        write_note_table(&mut code, &table).unwrap();
        code
    }

    fn sample_song(bank_index: usize, song_index: usize) -> Vec<u8> {
        let base = ((((bank_index * SONGS_PER_BANK) + song_index) % 43) * 2) as u8;
        let second_b_voiced = if song_index % 2 == 0 { 0x84 } else { 0x04 };
        vec![
            base,
            0x84,
            base + 2,
            0x84,
            base + 4,
            0x84,
            base + 6,
            0x84,
            base + 8,
            second_b_voiced,
            base + 10,
            0x84,
            0xFF,
        ]
    }

    fn sample_firmware_bytes(bank_count: usize) -> Vec<u8> {
        let code = sample_code();
        let mut firmware = Vec::with_capacity(bank_count * BANK_SIZE);
        for bank_index in 0..bank_count {
            let mut bank = vec![0xFFu8; BANK_SIZE];
            bank[..code.len()].copy_from_slice(&code);
            let mut cursor = DEFAULT_CODE_SECTION_LEN;
            for song_index in 0..SONGS_PER_BANK {
                let song = sample_song(bank_index, song_index);
                let ptr = u16::try_from(cursor).unwrap();
                let off = POINTER_TABLE_OFFSET + song_index * 2;
                bank[off..off + 2].copy_from_slice(&ptr.to_le_bytes());
                bank[cursor..cursor + song.len()].copy_from_slice(&song);
                cursor += song.len();
            }
            firmware.extend_from_slice(&bank);
        }
        firmware
    }

    fn uniform_bank_firmware_bytes(bank_count: usize) -> Vec<u8> {
        let code = sample_code();
        let mut firmware = Vec::with_capacity(bank_count * BANK_SIZE);
        for bank_index in 0..bank_count {
            let song = sample_song(bank_index, 0);
            let mut bank = vec![0xFFu8; BANK_SIZE];
            bank[..code.len()].copy_from_slice(&code);
            let ptr = u16::try_from(DEFAULT_CODE_SECTION_LEN).unwrap();
            bank[DEFAULT_CODE_SECTION_LEN..DEFAULT_CODE_SECTION_LEN + song.len()]
                .copy_from_slice(&song);
            for song_index in 0..SONGS_PER_BANK {
                let off = POINTER_TABLE_OFFSET + song_index * 2;
                bank[off..off + 2].copy_from_slice(&ptr.to_le_bytes());
            }
            firmware.extend_from_slice(&bank);
        }
        firmware
    }

    fn write_sample_firmware(path: &Path, bank_count: usize) {
        fs::write(path, sample_firmware_bytes(bank_count)).unwrap();
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "harmony-midi-{name}-{}-{unique}",
            std::process::id()
        ))
    }

    fn single_note_midi_bytes(note: u8, duration_ticks: u32, ppqn: u16) -> Vec<u8> {
        let tracks = vec![
            vec![TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            }],
            vec![
                TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Midi {
                        channel: u4::new(0),
                        message: MidiMessage::NoteOn {
                            key: u7::new(note),
                            vel: u7::new(100),
                        },
                    },
                },
                TrackEvent {
                    delta: u28::new(duration_ticks),
                    kind: TrackEventKind::Midi {
                        channel: u4::new(0),
                        message: MidiMessage::NoteOff {
                            key: u7::new(note),
                            vel: u7::new(0),
                        },
                    },
                },
                TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                },
            ],
        ];
        let smf = Smf {
            header: Header {
                format: Format::Parallel,
                timing: Timing::Metrical(u15::new(ppqn)),
            },
            tracks,
        };
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        bytes
    }

    fn single_note_midi_bytes_with_tempo(
        note: u8,
        duration_ticks: u32,
        ppqn: u16,
        micros_per_quarter: u32,
    ) -> Vec<u8> {
        midi_bytes_with_tracks(
            ppqn,
            vec![
                vec![
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(
                            micros_per_quarter,
                        ))),
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                    },
                ],
                vec![
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(note),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(duration_ticks),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOff {
                                key: u7::new(note),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                    },
                ],
            ],
        )
    }

    fn midi_bytes_with_tracks(ppqn: u16, tracks: Vec<Vec<TrackEvent<'static>>>) -> Vec<u8> {
        let smf = Smf {
            header: Header {
                format: Format::Parallel,
                timing: Timing::Metrical(u15::new(ppqn)),
            },
            tracks,
        };
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        bytes
    }

    fn note_on_only_midi_bytes(note: u8, ppqn: u16) -> Vec<u8> {
        midi_bytes_with_tracks(
            ppqn,
            vec![
                vec![TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                }],
                vec![
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(note),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                    },
                ],
            ],
        )
    }

    #[derive(Clone)]
    struct TestFurnaceSubsong {
        name: &'static str,
        ticks_per_second: f32,
        speed_pattern: Vec<u16>,
        pattern_length: u16,
        orders: Vec<u8>,
    }

    #[derive(Clone, Debug, Default)]
    struct TestFurnaceRow {
        note: Option<u8>,
        volume: Option<u8>,
        effect0: Option<(u8, u8)>,
    }

    #[derive(Clone)]
    struct TestFurnacePattern {
        subsong: u8,
        channel: u16,
        index: u16,
        rows: Vec<(usize, TestFurnaceRow)>,
    }

    fn push_c_string(target: &mut Vec<u8>, value: &str) {
        target.extend_from_slice(value.as_bytes());
        target.push(0);
    }

    fn build_test_block(id: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(id);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn build_test_pattern_block(pattern: &TestFurnacePattern) -> Vec<u8> {
        let mut body = Vec::new();
        body.push(pattern.subsong);
        body.push(pattern.channel as u8);
        body.extend_from_slice(&pattern.index.to_le_bytes());
        push_c_string(&mut body, "");

        let mut rows = pattern.rows.clone();
        rows.sort_by_key(|(row, _)| *row);
        let mut cursor = 0usize;
        for (row_index, row) in rows {
            while cursor < row_index {
                let gap = row_index - cursor;
                if gap >= 2 {
                    let skip = (gap - 2).min(0x7F);
                    body.push(0x80 | skip as u8);
                    cursor += skip + 2;
                } else {
                    body.push(0x00);
                    cursor += 1;
                }
            }

            let mut control = 0u8;
            if row.note.is_some() {
                control |= 0x01;
            }
            if row.volume.is_some() {
                control |= 0x04;
            }
            if row.effect0.is_some() {
                control |= 0x08 | 0x10;
            }
            body.push(control);
            if let Some(note) = row.note {
                body.push(note);
            }
            if let Some(volume) = row.volume {
                body.push(volume);
            }
            if let Some((fx, fx_val)) = row.effect0 {
                body.push(fx);
                body.push(fx_val);
            }
            cursor += 1;
        }
        body.push(0xFF);
        build_test_block(b"PATN", body)
    }

    fn build_test_subsong_block(total_channels: usize, subsong: &TestFurnaceSubsong) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&subsong.ticks_per_second.to_le_bytes());
        body.push(1);
        body.push(1);
        body.extend_from_slice(&subsong.pattern_length.to_le_bytes());
        body.extend_from_slice(
            &(subsong.orders.len() as u16 / total_channels as u16).to_le_bytes(),
        );
        body.push(4);
        body.push(16);
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.push(subsong.speed_pattern.len() as u8);
        for index in 0..16 {
            let speed = subsong.speed_pattern.get(index).copied().unwrap_or(1);
            body.extend_from_slice(&speed.to_le_bytes());
        }
        push_c_string(&mut body, subsong.name);
        push_c_string(&mut body, "");
        body.extend_from_slice(&subsong.orders);
        body.extend(std::iter::repeat_n(0u8, total_channels));
        body.extend(std::iter::repeat_n(0u8, total_channels));
        body.extend(std::iter::repeat_n(0u8, total_channels));
        for _ in 0..total_channels {
            push_c_string(&mut body, "");
        }
        for _ in 0..total_channels {
            push_c_string(&mut body, "");
        }
        body.extend(std::iter::repeat_n(0u8, total_channels * 4));
        build_test_block(b"SNG2", body)
    }

    fn build_test_inf2_block(
        total_channels: u16,
        chips: &[(u16, u16)],
        subsong_pointers: &[u32],
        pattern_pointers: &[u32],
    ) -> Vec<u8> {
        let mut body = Vec::new();
        for value in ["song", "author", "system", "album", "", "", "", ""] {
            push_c_string(&mut body, value);
        }
        body.extend_from_slice(&440.0f32.to_le_bytes());
        body.push(0);
        body.extend_from_slice(&1.0f32.to_le_bytes());
        body.extend_from_slice(&total_channels.to_le_bytes());
        body.extend_from_slice(&(chips.len() as u16).to_le_bytes());
        for &(chip_id, channel_count) in chips {
            body.extend_from_slice(&chip_id.to_le_bytes());
            body.extend_from_slice(&channel_count.to_le_bytes());
            body.extend_from_slice(&1.0f32.to_le_bytes());
            body.extend_from_slice(&0.0f32.to_le_bytes());
            body.extend_from_slice(&0.0f32.to_le_bytes());
        }
        body.extend_from_slice(&0u32.to_le_bytes());
        body.push(1);

        body.push(0x01);
        body.extend_from_slice(&(subsong_pointers.len() as u32).to_le_bytes());
        for pointer in subsong_pointers {
            body.extend_from_slice(&pointer.to_le_bytes());
        }
        body.push(0x07);
        body.extend_from_slice(&(pattern_pointers.len() as u32).to_le_bytes());
        for pointer in pattern_pointers {
            body.extend_from_slice(&pointer.to_le_bytes());
        }
        body.push(0);

        build_test_block(b"INF2", body)
    }

    fn build_test_furnace_module(
        chips: &[(u16, u16)],
        subsongs: &[TestFurnaceSubsong],
        patterns: &[TestFurnacePattern],
    ) -> Vec<u8> {
        let total_channels: u16 = chips.iter().map(|(_, channels)| *channels).sum();
        let subsong_blocks: Vec<Vec<u8>> = subsongs
            .iter()
            .map(|subsong| build_test_subsong_block(total_channels as usize, subsong))
            .collect();
        let pattern_blocks: Vec<Vec<u8>> = patterns.iter().map(build_test_pattern_block).collect();

        let placeholder_info = build_test_inf2_block(
            total_channels,
            chips,
            &vec![0; subsong_blocks.len()],
            &vec![0; pattern_blocks.len()],
        );
        let mut next_ptr = 32 + placeholder_info.len();
        let mut subsong_pointers = Vec::with_capacity(subsong_blocks.len());
        for block in &subsong_blocks {
            subsong_pointers.push(next_ptr as u32);
            next_ptr += block.len();
        }
        let mut pattern_pointers = Vec::with_capacity(pattern_blocks.len());
        for block in &pattern_blocks {
            pattern_pointers.push(next_ptr as u32);
            next_ptr += block.len();
        }

        let info_block =
            build_test_inf2_block(total_channels, chips, &subsong_pointers, &pattern_pointers);
        let mut out = Vec::new();
        out.extend_from_slice(b"-Furnace module-");
        out.extend_from_slice(&240u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&32u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 8]);
        out.extend_from_slice(&info_block);
        for block in subsong_blocks {
            out.extend_from_slice(&block);
        }
        for block in pattern_blocks {
            out.extend_from_slice(&block);
        }
        out
    }

    fn compress_zlib(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn simple_ay_furnace_subsong(
        name: &'static str,
        total_channels: usize,
        channel_offset: usize,
        note_a: u8,
        note_b: u8,
        note_c: u8,
    ) -> (TestFurnaceSubsong, Vec<TestFurnacePattern>) {
        let mut orders = vec![0u8; total_channels];
        orders[channel_offset] = 0;
        orders[channel_offset + 1] = 0;
        orders[channel_offset + 2] = 0;
        let subsong = TestFurnaceSubsong {
            name,
            ticks_per_second: 60.0,
            speed_pattern: vec![6],
            pattern_length: 2,
            orders,
        };
        let patterns = vec![
            TestFurnacePattern {
                subsong: 0,
                channel: channel_offset as u16,
                index: 0,
                rows: vec![(
                    0,
                    TestFurnaceRow {
                        note: Some(note_a),
                        ..Default::default()
                    },
                )],
            },
            TestFurnacePattern {
                subsong: 0,
                channel: (channel_offset + 1) as u16,
                index: 0,
                rows: vec![(
                    0,
                    TestFurnaceRow {
                        note: Some(note_b),
                        ..Default::default()
                    },
                )],
            },
            TestFurnacePattern {
                subsong: 0,
                channel: (channel_offset + 2) as u16,
                index: 0,
                rows: vec![(
                    0,
                    TestFurnaceRow {
                        note: Some(note_c),
                        ..Default::default()
                    },
                )],
            },
        ];
        (subsong, patterns)
    }

    #[test]
    fn furnace_parser_reads_channel_major_order_tables() {
        let subsong = TestFurnaceSubsong {
            name: "orders",
            ticks_per_second: 64.0,
            speed_pattern: vec![1],
            pattern_length: 1,
            orders: vec![0, 1, 2, 3, 4, 5],
        };
        let patterns = vec![
            TestFurnacePattern {
                subsong: 0,
                channel: 0,
                index: 0,
                rows: vec![(0, TestFurnaceRow { note: Some(108), ..Default::default() })],
            },
            TestFurnacePattern {
                subsong: 0,
                channel: 0,
                index: 1,
                rows: vec![(0, TestFurnaceRow { note: Some(109), ..Default::default() })],
            },
            TestFurnacePattern {
                subsong: 0,
                channel: 1,
                index: 2,
                rows: vec![(0, TestFurnaceRow { note: Some(110), ..Default::default() })],
            },
            TestFurnacePattern {
                subsong: 0,
                channel: 1,
                index: 3,
                rows: vec![(0, TestFurnaceRow { note: Some(111), ..Default::default() })],
            },
            TestFurnacePattern {
                subsong: 0,
                channel: 2,
                index: 4,
                rows: vec![(0, TestFurnaceRow { note: Some(112), ..Default::default() })],
            },
            TestFurnacePattern {
                subsong: 0,
                channel: 2,
                index: 5,
                rows: vec![(0, TestFurnaceRow { note: Some(113), ..Default::default() })],
            },
        ];
        let module = build_test_furnace_module(&[(0x80, 3)], &[subsong], &patterns);
        let parsed =
            parse_furnace_bytes("orders.fur", &module, &mut WarningCollector::new()).unwrap();

        assert_eq!(parsed.subsongs.len(), 1);
        assert_eq!(
            parsed.subsongs[0].voices[0]
                .iter()
                .map(|interval| interval.pitch_value)
                .collect::<Vec<_>>(),
            vec![60.0, 61.0]
        );
        assert_eq!(
            parsed.subsongs[0].voices[1]
                .iter()
                .map(|interval| interval.pitch_value)
                .collect::<Vec<_>>(),
            vec![62.0, 63.0]
        );
        assert_eq!(
            parsed.subsongs[0].voices[2]
                .iter()
                .map(|interval| interval.pitch_value)
                .collect::<Vec<_>>(),
            vec![64.0, 65.0]
        );
    }

    #[test]
    fn harmony_stream_roundtrips_through_pure_midi_encoding() {
        let code = sample_code();
        for bank_index in 0..2 {
            for song_index in 0..4 {
                let song = sample_song(bank_index, song_index);
                let midi = harmony_song_to_midi_bytes(&code, &song).unwrap();
                let rebuilt =
                    midi_bytes_to_harmony_stream(&code, &midi, &mut WarningCollector::new())
                        .unwrap();
                assert_eq!(rebuilt, song);
            }
        }
    }

    #[test]
    fn extracts_harmony64_with_four_banks() {
        let path = temp_test_dir("fixture-4bank").with_extension("bin");
        write_sample_firmware(&path, 4);
        let firmware = parse_firmware(&path).unwrap();
        assert_eq!(firmware.banks.len(), 4);
        assert_eq!(firmware.code_section.len(), DEFAULT_CODE_SECTION_LEN);
        assert!(
            firmware
                .banks
                .iter()
                .all(|bank| bank.songs.len() == SONGS_PER_BANK)
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn midi_without_metadata_quantizes_ticks() {
        let bytes = single_note_midi_bytes(60, 7, 10);

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &bytes, &mut warnings).unwrap();
        assert!(!warnings.into_vec().is_empty());
        assert_eq!(stream.last().copied(), Some(0xFF));
    }

    #[test]
    fn midi_import_respects_custom_ticks_per_quarter() {
        let bytes = single_note_midi_bytes(60, 10, 10);

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream =
            midi_bytes_to_harmony_stream_with_ticks(&code, &bytes, &mut warnings, 20).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();
        assert_eq!(voices[0][0].duration, 20);
    }

    #[test]
    fn midi_import_uses_tempo_to_shorten_faster_sections() {
        let bytes = single_note_midi_bytes_with_tempo(60, 10, 10, 250_000);

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &bytes, &mut warnings).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();
        assert_eq!(voices[0][0].duration, 16);
    }

    #[test]
    fn midi_import_uses_tempo_to_extend_slower_sections() {
        let bytes = single_note_midi_bytes_with_tempo(60, 10, 10, 1_000_000);

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &bytes, &mut warnings).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();
        assert_eq!(voices[0][0].duration, 64);
    }

    #[test]
    fn midi_import_applies_tempo_changes_mid_song() {
        let midi = midi_bytes_with_tracks(
            10,
            vec![
                vec![
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500_000))),
                    },
                    TrackEvent {
                        delta: u28::new(10),
                        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(250_000))),
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                    },
                ],
                vec![
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(60),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(10),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOff {
                                key: u7::new(60),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(62),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(10),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOff {
                                key: u7::new(62),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                    },
                ],
            ],
        );

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &midi, &mut warnings).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();
        assert_eq!(voices[0][0].duration, 32);
        assert_eq!(voices[0][1].duration, 16);
    }

    #[test]
    fn midi_import_scales_tempo_aware_durations_with_custom_ticks_per_quarter() {
        let bytes = single_note_midi_bytes_with_tempo(60, 10, 10, 250_000);

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream =
            midi_bytes_to_harmony_stream_with_ticks(&code, &bytes, &mut warnings, 20).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();
        assert_eq!(voices[0][0].duration, 10);
    }

    #[test]
    fn overly_long_midi_notes_are_trimmed_instead_of_retriggered() {
        let bytes = single_note_midi_bytes(60, 200, HARMONY_TICKS_PER_QUARTER);

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &bytes, &mut warnings).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();

        assert_eq!(voices[0].len(), 2);
        assert_eq!(voices[0][0].duration, 0x7F);
        assert!(voices[0][0].voiced);
        assert_eq!(voices[0][1].duration, 73);
        assert!(!voices[0][1].voiced);
    }

    #[test]
    fn trimmed_long_notes_preserve_later_note_start_times_with_silence() {
        let midi = midi_bytes_with_tracks(
            HARMONY_TICKS_PER_QUARTER,
            vec![
                vec![TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                }],
                vec![
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(60),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(200),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOff {
                                key: u7::new(60),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(62),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(8),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOff {
                                key: u7::new(62),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                    },
                ],
            ],
        );

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &midi, &mut warnings).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();

        assert_eq!(voices[0].len(), 3);
        assert_eq!(voices[0][0].duration, 0x7F);
        assert!(voices[0][0].voiced);
        assert_eq!(voices[0][1].duration, 73);
        assert!(!voices[0][1].voiced);
        assert_eq!(voices[0][2].duration, 8);
        assert!(voices[0][2].voiced);
    }

    #[test]
    fn pitch_encoding_is_unique_for_note_table() {
        let code = sample_code();
        let mapping = build_note_mapping(&code).unwrap();
        assert_eq!(mapping.valid_indices.len(), mapping.pitch_to_idx.len());
    }

    #[test]
    fn complete_midi_note_table_has_all_128_entries() {
        let table = build_complete_midi_note_table();
        assert!(table.iter().all(|&value| value != 0));
        let mut code = sample_code();
        write_note_table(&mut code, &table).unwrap();
        let mapping = build_note_mapping(&code).unwrap();
        assert_eq!(mapping.valid_indices.len(), 128);
        assert!(mapping.pitch_to_idx.len() < 128);
    }

    #[test]
    fn out_of_range_midi_promotes_note_table() {
        let midi = single_note_midi_bytes(10, 32, HARMONY_TICKS_PER_QUARTER);
        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let upgraded = maybe_upgrade_code_for_midi_import(
            &code,
            &[MidiImportInput {
                display_name: "test.mid".to_string(),
                bytes: midi.clone(),
            }],
            &mut warnings,
        )
        .unwrap();
        assert_ne!(upgraded, code);
        assert!(!warnings.into_vec().is_empty());

        let stream =
            midi_bytes_to_harmony_stream(&upgraded, &midi, &mut WarningCollector::new()).unwrap();
        assert_eq!(stream[0], 20);
        assert_eq!(stream[1] & 0x7F, 32);
    }

    #[test]
    fn extract_and_build_roundtrip_with_code_bin() {
        let extract_dir = temp_test_dir("extract");
        let rebuilt_path = temp_test_dir("rebuilt").with_extension("bin");
        let source_path = temp_test_dir("source").with_extension("bin");
        fs::write(&source_path, uniform_bank_firmware_bytes(2)).unwrap();
        extract_firmware_to_dir(&source_path, &extract_dir).unwrap();
        assert!(extract_dir.join("code.bin").exists());

        let report = build_firmware_from_dir(&extract_dir, &rebuilt_path).unwrap();
        assert_eq!(report.banks.len(), 2);
        let rebuilt = parse_firmware(&rebuilt_path).unwrap();
        assert_eq!(rebuilt.banks.len(), 2);
        assert!(
            rebuilt
                .banks
                .iter()
                .all(|bank| bank.songs.len() == SONGS_PER_BANK)
        );

        let _ = fs::remove_dir_all(&extract_dir);
        let _ = fs::remove_file(&rebuilt_path);
        let _ = fs::remove_file(&source_path);
    }

    #[test]
    fn arbitrary_midi_maps_channels_directly_and_new_note_wins_on_overlap() {
        let midi = midi_bytes_with_tracks(
            HARMONY_TICKS_PER_QUARTER,
            vec![
                vec![TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                }],
                vec![
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(60),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(1),
                            message: MidiMessage::NoteOn {
                                key: u7::new(65),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(4),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOn {
                                key: u7::new(62),
                                vel: u7::new(100),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(1),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOff {
                                key: u7::new(60),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(3),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(0),
                            message: MidiMessage::NoteOff {
                                key: u7::new(62),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Midi {
                            channel: u4::new(1),
                            message: MidiMessage::NoteOff {
                                key: u7::new(65),
                                vel: u7::new(0),
                            },
                        },
                    },
                    TrackEvent {
                        delta: u28::new(0),
                        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
                    },
                ],
            ],
        );

        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &midi, &mut warnings).unwrap();
        let voices = decode_stream_to_voice_segments(&stream).unwrap();
        let mapping = build_note_mapping(&code).unwrap();

        assert_eq!(
            voices[0],
            vec![
                HarmonySegment {
                    idx: nearest_idx_for_pitch(&mapping, 60.0),
                    duration: 4,
                    voiced: true,
                },
                HarmonySegment {
                    idx: nearest_idx_for_pitch(&mapping, 62.0),
                    duration: 4,
                    voiced: true,
                },
            ]
        );
        assert_eq!(
            voices[1],
            vec![HarmonySegment {
                idx: nearest_idx_for_pitch(&mapping, 65.0),
                duration: 8,
                voiced: true,
            }]
        );
        assert_eq!(
            voices[2],
            vec![HarmonySegment {
                idx: 0x00,
                duration: 8,
                voiced: false,
            }]
        );
        assert!(warnings.into_vec().is_empty());
    }

    #[test]
    fn build_firmware_applies_channel_volume_overrides() {
        let input_dir = temp_test_dir("volumes-input");
        let output_path = temp_test_dir("volumes-output").with_extension("bin");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(input_dir.join("code.bin"), sample_code()).unwrap();
        fs::write(
            input_dir.join("bank01_song01.mid"),
            single_note_midi_bytes(60, 8, HARMONY_TICKS_PER_QUARTER),
        )
        .unwrap();

        let report = build_firmware_from_dir_with_options(
            &input_dir,
            &output_path,
            Some([15, 8, 3]),
            HARMONY_TICKS_PER_QUARTER,
        )
        .unwrap();
        assert_eq!(report.banks.len(), 1);

        let firmware = fs::read(&output_path).unwrap();
        assert_eq!(
            read_channel_volumes(&firmware[..DEFAULT_CODE_SECTION_LEN]).unwrap(),
            [15, 8, 3]
        );

        let _ = fs::remove_dir_all(&input_dir);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn build_firmware_uses_tempo_aware_midi_import() {
        let input_dir = temp_test_dir("tempo-build-input");
        let output_path = temp_test_dir("tempo-build-output").with_extension("bin");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(input_dir.join("code.bin"), sample_code()).unwrap();
        fs::write(
            input_dir.join("bank01_song01.mid"),
            single_note_midi_bytes_with_tempo(60, 10, 10, 250_000),
        )
        .unwrap();

        let report = build_firmware_from_dir(&input_dir, &output_path).unwrap();
        assert_eq!(report.banks.len(), 1);

        let firmware = fs::read(&output_path).unwrap();
        let bank = &firmware[..BANK_SIZE];
        let pointers = parse_pointer_table(bank).unwrap();
        let start = usize::from(pointers[0]);
        let end = bank[start..]
            .iter()
            .position(|&b| b == 0xFF)
            .map(|idx| start + idx + 1)
            .unwrap();
        let voices = decode_stream_to_voice_segments(&bank[start..end]).unwrap();
        assert_eq!(voices[0][0].duration, 16);

        let _ = fs::remove_dir_all(&input_dir);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn open_notes_at_end_of_file_close_silently() {
        let midi = note_on_only_midi_bytes(60, HARMONY_TICKS_PER_QUARTER);
        let mut warnings = WarningCollector::new();
        let code = sample_code();
        let stream = midi_bytes_to_harmony_stream(&code, &midi, &mut warnings).unwrap();
        assert_eq!(stream.last().copied(), Some(0xFF));
        assert!(warnings.into_vec().is_empty());
    }

    #[test]
    fn missing_song_slots_alias_an_existing_pointer() {
        let input_dir = temp_test_dir("alias-input");
        let output_path = temp_test_dir("alias-output").with_extension("bin");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(input_dir.join("code.bin"), sample_code()).unwrap();
        fs::write(
            input_dir.join("bank01_song03.mid"),
            single_note_midi_bytes(60, 8, HARMONY_TICKS_PER_QUARTER),
        )
        .unwrap();

        let report = build_firmware_from_dir(&input_dir, &output_path).unwrap();
        assert_eq!(report.banks[0].unique_song_count, 1);
        assert_eq!(report.banks[0].aliased_song_slots, 15);

        let firmware = fs::read(&output_path).unwrap();
        let first_bank = &firmware[..BANK_SIZE];
        let pointers = parse_pointer_table(first_bank).unwrap();
        assert!(pointers.iter().all(|&ptr| ptr == pointers[2]));

        let _ = fs::remove_dir_all(&input_dir);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn furnace_parser_accepts_zlib_and_maps_ay_offset() {
        let (subsong, patterns) = simple_ay_furnace_subsong("offset", 7, 4, 108, 112, 115);
        let module = build_test_furnace_module(&[(0x03, 4), (0x80, 3)], &[subsong], &patterns);
        let compressed = compress_zlib(&module);

        let parsed_raw =
            parse_furnace_bytes("offset.fur", &module, &mut WarningCollector::new()).unwrap();
        let parsed_zlib =
            parse_furnace_bytes("offset.fur", &compressed, &mut WarningCollector::new()).unwrap();

        assert_eq!(parsed_raw.subsongs.len(), 1);
        assert_eq!(parsed_zlib.subsongs.len(), 1);
        assert_eq!(parsed_raw.subsongs[0].voices[0][0].pitch_value, 60.0);
        assert_eq!(parsed_raw.subsongs[0].voices[1][0].pitch_value, 64.0);
        assert_eq!(parsed_raw.subsongs[0].voices[2][0].pitch_value, 67.0);
        assert_eq!(parsed_zlib.pitch_bounds, Some((60.0, 67.0)));
    }

    #[test]
    fn build_firmware_accepts_mixed_mid_and_fur_inputs() {
        let input_dir = temp_test_dir("mixed-build-input");
        let output_path = temp_test_dir("mixed-build-output").with_extension("bin");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(input_dir.join("code.bin"), sample_code()).unwrap();

        let (subsong, patterns) = simple_ay_furnace_subsong("mix", 3, 0, 108, 112, 115);
        let fur = build_test_furnace_module(&[(0x80, 3)], &[subsong], &patterns);
        fs::write(input_dir.join("bank01_song01.fur"), fur).unwrap();
        fs::write(
            input_dir.join("bank01_song02.mid"),
            single_note_midi_bytes(60, 8, HARMONY_TICKS_PER_QUARTER),
        )
        .unwrap();

        let report = build_firmware_from_dir(&input_dir, &output_path).unwrap();
        assert_eq!(report.banks.len(), 1);
        assert_eq!(report.banks[0].unique_song_count, 2);

        let firmware = fs::read(&output_path).unwrap();
        let pointers = parse_pointer_table(&firmware[..BANK_SIZE]).unwrap();
        assert_ne!(pointers[0], pointers[1]);

        let _ = fs::remove_dir_all(&input_dir);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn furnace_multisong_conflicts_with_explicit_next_slot() {
        let input_dir = temp_test_dir("fur-conflict-input");
        let output_path = temp_test_dir("fur-conflict-output").with_extension("bin");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(input_dir.join("code.bin"), sample_code()).unwrap();

        let total_channels = 3usize;
        let mut patterns = Vec::new();
        let mut subsongs = Vec::new();
        for subsong_index in 0..2u8 {
            let mut orders = vec![0u8; total_channels];
            orders[0] = subsong_index;
            orders[1] = subsong_index;
            orders[2] = subsong_index;
            subsongs.push(TestFurnaceSubsong {
                name: if subsong_index == 0 {
                    "first"
                } else {
                    "second"
                },
                ticks_per_second: 60.0,
                speed_pattern: vec![6],
                pattern_length: 2,
                orders,
            });
            for channel in 0..3u16 {
                patterns.push(TestFurnacePattern {
                    subsong: subsong_index,
                    channel,
                    index: subsong_index as u16,
                    rows: vec![(
                        0,
                        TestFurnaceRow {
                            note: Some(108 + channel as u8),
                            ..Default::default()
                        },
                    )],
                });
            }
        }
        let fur = build_test_furnace_module(&[(0x80, 3)], &subsongs, &patterns);
        fs::write(input_dir.join("bank01_song01.fur"), fur).unwrap();
        fs::write(
            input_dir.join("bank01_song02.mid"),
            single_note_midi_bytes(60, 8, HARMONY_TICKS_PER_QUARTER),
        )
        .unwrap();

        let err = build_firmware_from_dir(&input_dir, &output_path).unwrap_err();
        assert!(err.to_string().contains("bank 01 song 02"));

        let _ = fs::remove_dir_all(&input_dir);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn furnace_subsongs_carry_across_song_16() {
        let input_dir = temp_test_dir("fur-carry-slot-input");
        let output_path = temp_test_dir("fur-carry-slot-output").with_extension("bin");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(input_dir.join("code.bin"), sample_code()).unwrap();

        let total_channels = 3usize;
        let mut patterns = Vec::new();
        let mut subsongs = Vec::new();
        for subsong_index in 0..2u8 {
            let mut orders = vec![0u8; total_channels];
            orders[0] = subsong_index;
            orders[1] = subsong_index;
            orders[2] = subsong_index;
            subsongs.push(TestFurnaceSubsong {
                name: if subsong_index == 0 {
                    "tail-a"
                } else {
                    "tail-b"
                },
                ticks_per_second: 60.0,
                speed_pattern: vec![6],
                pattern_length: 2,
                orders,
            });
            for channel in 0..3u16 {
                patterns.push(TestFurnacePattern {
                    subsong: subsong_index,
                    channel,
                    index: subsong_index as u16,
                    rows: vec![(
                        0,
                        TestFurnaceRow {
                            note: Some(108 + channel as u8 + subsong_index),
                            ..Default::default()
                        },
                    )],
                });
            }
        }
        let fur = build_test_furnace_module(&[(0x80, 3)], &subsongs, &patterns);
        fs::write(input_dir.join("bank01_song16.fur"), fur).unwrap();

        let report = build_firmware_from_dir(&input_dir, &output_path).unwrap();
        assert_eq!(report.banks.len(), 2);
        assert_eq!(report.banks[0].unique_song_count, 1);
        assert_eq!(report.banks[1].unique_song_count, 1);

        let firmware = fs::read(&output_path).unwrap();
        let bank0 = &firmware[..BANK_SIZE];
        let bank1 = &firmware[BANK_SIZE..2 * BANK_SIZE];
        let pointers0 = parse_pointer_table(bank0).unwrap();
        let pointers1 = parse_pointer_table(bank1).unwrap();
        let song0 = &bank0[usize::from(pointers0[15])..];
        let song1 = &bank1[usize::from(pointers1[0])..];
        let voices0 = decode_stream_to_voice_segments(
            &song0[..song0.iter().position(|&b| b == 0xFF).unwrap() + 1],
        )
        .unwrap();
        let voices1 = decode_stream_to_voice_segments(
            &song1[..song1.iter().position(|&b| b == 0xFF).unwrap() + 1],
        )
        .unwrap();
        assert_ne!(voices0[0][0].idx, voices1[0][0].idx);

        let _ = fs::remove_dir_all(&input_dir);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn furnace_subsongs_carry_when_bank_data_fills() {
        let input_dir = temp_test_dir("fur-carry-bytes-input");
        let output_path = temp_test_dir("fur-carry-bytes-output").with_extension("bin");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(input_dir.join("code.bin"), sample_code()).unwrap();

        let mut subsongs = Vec::new();
        let mut patterns = Vec::new();
        for subsong_index in 0..12u8 {
            let mut orders = vec![0u8; 3];
            orders[0] = subsong_index;
            orders[1] = subsong_index;
            orders[2] = subsong_index;
            subsongs.push(TestFurnaceSubsong {
                name: "long",
                ticks_per_second: 60.0,
                speed_pattern: vec![1],
                pattern_length: 256,
                orders,
            });
            for channel in 0..3u16 {
                let mut rows = Vec::new();
                for row in 0..256usize {
                    rows.push((
                        row,
                        TestFurnaceRow {
                            note: Some(108 + ((row + channel as usize) % 12) as u8),
                            ..Default::default()
                        },
                    ));
                }
                patterns.push(TestFurnacePattern {
                    subsong: subsong_index,
                    channel,
                    index: subsong_index as u16,
                    rows,
                });
            }
        }
        let fur = build_test_furnace_module(&[(0x80, 3)], &subsongs, &patterns);
        fs::write(input_dir.join("bank01_song01.fur"), fur).unwrap();

        let report = build_firmware_from_dir(&input_dir, &output_path).unwrap();
        assert!(report.banks.len() >= 2);
        assert!(report.banks[0].unique_song_count < 12);

        let _ = fs::remove_dir_all(&input_dir);
        let _ = fs::remove_file(&output_path);
    }

    #[test]
    fn parser_accepts_mixed_case_and_no_leading_zero_filenames() {
        assert_eq!(
            parse_song_filename("BANK2_SONG8.mid"),
            Some(SongFileId {
                bank_index: 1,
                song_index: 7
            })
        );
        assert_eq!(
            parse_song_filename("bank02_song08.MID"),
            Some(SongFileId {
                bank_index: 1,
                song_index: 7
            })
        );
        assert_eq!(
            parse_song_filename("bank2_song8.mid"),
            Some(SongFileId {
                bank_index: 1,
                song_index: 7
            })
        );
        assert_eq!(
            parse_song_input_filename("bank2_song8.fur"),
            Some((
                SongFileId {
                    bank_index: 1,
                    song_index: 7
                },
                SongInputKind::Furnace
            ))
        );
    }
}
