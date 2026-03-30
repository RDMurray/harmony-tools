use crate::{PitchInterval, WarningCollector};
use anyhow::{Context, Result, anyhow, bail};
use flate2::read::ZlibDecoder;
use std::collections::HashMap;
use std::io::Read;

const MAGIC: &[u8; 16] = b"-Furnace module-";
const FORMAT_INF2_MIN_VERSION: u16 = 240;
const ELEMENT_TYPE_SUBSONG: u8 = 0x01;
const ELEMENT_TYPE_PATTERN: u8 = 0x07;
const CHIP_ID_AY_3_8910: u16 = 0x80;
const CHIP_ID_AY8930: u16 = 0x9A;
const HARMONY_TICKS_PER_SECOND: f64 = 64.0;

#[derive(Clone, Debug)]
pub(crate) struct ParsedFurnaceModule {
    pub subsongs: Vec<ParsedFurnaceSubsong>,
    pub pitch_bounds: Option<(f64, f64)>,
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedFurnaceSubsong {
    pub display_name: String,
    pub voices: [Vec<PitchInterval>; 3],
}

#[derive(Clone, Debug)]
struct FurnaceInfo {
    total_channels: usize,
    ay_channel_offset: usize,
    subsong_pointers: Vec<u32>,
    pattern_pointers: Vec<u32>,
}

#[derive(Clone, Debug)]
struct FurnaceSubsong {
    ticks_per_second: f32,
    pattern_length: usize,
    orders_length: usize,
    virtual_tempo_num: u16,
    virtual_tempo_den: u16,
    speed_pattern_len: usize,
    speed_pattern: [u16; 16],
    name: String,
    orders: Vec<u8>,
}

#[derive(Clone, Debug)]
struct FurnacePattern {
    subsong_index: usize,
    channel_index: usize,
    pattern_index: u16,
    rows: HashMap<usize, PatternRow>,
}

#[derive(Clone, Debug, Default)]
struct PatternRow {
    note: Option<u8>,
    volume: Option<u8>,
    effects: Vec<PatternEffect>,
}

#[derive(Clone, Debug)]
struct PatternEffect {
    command: u8,
    value: Option<u8>,
}

#[derive(Clone, Copy, Debug)]
enum FlowControl {
    ScheduledJump {
        target_order: Option<usize>,
        start_row: usize,
    },
    Stop,
}

pub(crate) fn parse_furnace_bytes(
    display_name: &str,
    bytes: &[u8],
    warnings: &mut WarningCollector,
) -> Result<ParsedFurnaceModule> {
    let decoded = decode_container(bytes).with_context(|| format!("decoding {display_name}"))?;
    let mut root = CursorReader::new(&decoded);
    root.expect_bytes(MAGIC)
        .with_context(|| format!("reading header from {display_name}"))?;
    let version = root.read_u16()?;
    if version < FORMAT_INF2_MIN_VERSION {
        bail!(
            "{} uses Furnace format version {} but only INF2-based modules (version >= {}) are supported",
            display_name,
            version,
            FORMAT_INF2_MIN_VERSION
        );
    }
    root.skip(2)?;
    let info_ptr = root.read_u32()? as usize;
    root.skip(8)?;

    let info = parse_info(&decoded, info_ptr)
        .with_context(|| format!("parsing INF2 from {display_name}"))?;
    let subsongs = parse_subsongs(&decoded, &info)
        .with_context(|| format!("parsing subsongs from {display_name}"))?;
    let patterns = parse_patterns(&decoded, version, &info.pattern_pointers)
        .with_context(|| format!("parsing patterns from {display_name}"))?;
    let pattern_map: HashMap<(usize, usize, u16), FurnacePattern> = patterns
        .into_iter()
        .map(|pattern| {
            (
                (
                    pattern.subsong_index,
                    pattern.channel_index,
                    pattern.pattern_index,
                ),
                pattern,
            )
        })
        .collect();

    let mut parsed_subsongs = Vec::with_capacity(subsongs.len());
    let mut min_pitch = f64::INFINITY;
    let mut max_pitch = f64::NEG_INFINITY;

    for (subsong_index, subsong) in subsongs.iter().enumerate() {
        let voices = build_subsong_voices(
            display_name,
            subsong_index,
            subsong,
            &pattern_map,
            info.ay_channel_offset,
            warnings,
        )?;
        for voice in &voices {
            for interval in voice {
                min_pitch = min_pitch.min(interval.pitch_value);
                max_pitch = max_pitch.max(interval.pitch_value);
            }
        }
        let label = if subsong.name.is_empty() {
            format!("{display_name} subsong {}", subsong_index)
        } else {
            format!(
                "{display_name} subsong {} ({})",
                subsong_index, subsong.name
            )
        };
        parsed_subsongs.push(ParsedFurnaceSubsong {
            display_name: label,
            voices,
        });
    }

    Ok(ParsedFurnaceModule {
        subsongs: parsed_subsongs,
        pitch_bounds: if min_pitch.is_finite() && max_pitch.is_finite() {
            Some((min_pitch, max_pitch))
        } else {
            None
        },
    })
}

fn decode_container(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.starts_with(MAGIC) {
        return Ok(bytes.to_vec());
    }

    let mut decoded = Vec::new();
    let mut decoder = ZlibDecoder::new(bytes);
    decoder
        .read_to_end(&mut decoded)
        .context("zlib decompression failed")?;
    if !decoded.starts_with(MAGIC) {
        bail!("not a Furnace module or zlib-compressed Furnace module");
    }
    Ok(decoded)
}

fn parse_info(bytes: &[u8], ptr: usize) -> Result<FurnaceInfo> {
    let mut reader = CursorReader::at(bytes, ptr)?;
    reader.expect_str("INF2")?;
    let block_len = reader.read_u32()? as usize;
    let block_end = reader
        .pos
        .checked_add(block_len)
        .ok_or_else(|| anyhow!("INF2 block length overflow"))?;
    reader.ensure_within(block_end)?;

    for _ in 0..8 {
        reader.read_c_string()?;
    }
    reader.read_f32()?;
    reader.read_u8()?;
    reader.read_f32()?;
    let total_channels = usize::from(reader.read_u16()?);
    let num_chips = usize::from(reader.read_u16()?);

    let mut ay_channel_offset = None;
    let mut running_channel_offset = 0usize;
    for _ in 0..num_chips {
        let chip_id = reader.read_u16()?;
        let chip_channels = usize::from(reader.read_u16()?);
        reader.read_f32()?;
        reader.read_f32()?;
        reader.read_f32()?;

        if is_legacy_compound_chip(chip_id) {
            bail!("legacy compound chip ID 0x{chip_id:04X} is not supported");
        }
        if matches!(chip_id, CHIP_ID_AY_3_8910 | CHIP_ID_AY8930) {
            if ay_channel_offset.is_some() {
                bail!("multiple AY-compatible chips are not supported");
            }
            if chip_channels != 3 {
                bail!(
                    "AY-compatible chip 0x{chip_id:04X} must expose 3 channels, got {chip_channels}"
                );
            }
            ay_channel_offset = Some(running_channel_offset);
        }
        running_channel_offset = running_channel_offset.saturating_add(chip_channels);
    }
    if running_channel_offset != total_channels {
        bail!(
            "INF2 channel count mismatch: header says {} channels but chip list sums to {}",
            total_channels,
            running_channel_offset
        );
    }

    let patchbay_connections = reader.read_u32()? as usize;
    reader.skip(patchbay_connections.saturating_mul(4))?;
    reader.read_u8()?;

    let mut subsong_pointers = Vec::new();
    let mut pattern_pointers = Vec::new();
    loop {
        let element_type = reader.read_u8()?;
        if element_type == 0 {
            break;
        }
        let count = reader.read_u32()? as usize;
        let mut pointers = Vec::with_capacity(count);
        for _ in 0..count {
            pointers.push(reader.read_u32()?);
        }
        match element_type {
            ELEMENT_TYPE_SUBSONG => subsong_pointers.extend(pointers),
            ELEMENT_TYPE_PATTERN => pattern_pointers.extend(pointers),
            _ => {}
        }
    }

    if reader.pos > block_end {
        bail!("INF2 block overflow");
    }
    if subsong_pointers.is_empty() {
        bail!("INF2 does not contain any SNG2 elements");
    }

    Ok(FurnaceInfo {
        total_channels,
        ay_channel_offset: ay_channel_offset
            .ok_or_else(|| anyhow!("no AY-compatible chip found"))?,
        subsong_pointers,
        pattern_pointers,
    })
}

fn parse_subsongs(bytes: &[u8], info: &FurnaceInfo) -> Result<Vec<FurnaceSubsong>> {
    let mut out = Vec::with_capacity(info.subsong_pointers.len());
    for &ptr in &info.subsong_pointers {
        let mut reader = CursorReader::at(bytes, ptr as usize)?;
        reader.expect_str("SNG2")?;
        let block_len = reader.read_u32()? as usize;
        let block_end = reader
            .pos
            .checked_add(block_len)
            .ok_or_else(|| anyhow!("SNG2 block length overflow"))?;
        reader.ensure_within(block_end)?;

        let ticks_per_second = reader.read_f32()?;
        let _initial_arpeggio_speed = reader.read_u8()?;
        let _effect_speed_divider = reader.read_u8()?;
        let pattern_length = usize::from(reader.read_u16()?);
        let orders_length = usize::from(reader.read_u16()?);
        let _highlight_a = reader.read_u8()?;
        let _highlight_b = reader.read_u8()?;
        let virtual_tempo_num = reader.read_u16()?;
        let virtual_tempo_den = reader.read_u16()?;
        let speed_pattern_len = usize::from(reader.read_u8()?);
        if !(1..=16).contains(&speed_pattern_len) {
            bail!("invalid SNG2 speed pattern length {}", speed_pattern_len);
        }
        let mut speed_pattern = [0u16; 16];
        for speed in &mut speed_pattern {
            *speed = reader.read_u16()?;
        }
        let name = reader.read_c_string()?;
        let _comment = reader.read_c_string()?;

        let orders_len = info
            .total_channels
            .checked_mul(orders_length)
            .ok_or_else(|| anyhow!("orders table length overflow"))?;
        let orders = reader.read_vec(orders_len)?;
        reader.skip(info.total_channels)?;
        reader.skip(info.total_channels)?;
        reader.skip(info.total_channels)?;
        for _ in 0..info.total_channels {
            reader.read_c_string()?;
        }
        for _ in 0..info.total_channels {
            reader.read_c_string()?;
        }
        reader.skip(info.total_channels.saturating_mul(4))?;

        if reader.pos > block_end {
            bail!("SNG2 block overflow");
        }
        if ticks_per_second <= 0.0 {
            bail!("SNG2 ticks per second must be positive");
        }
        if virtual_tempo_num == 0 || virtual_tempo_den == 0 {
            bail!("SNG2 virtual tempo values must be non-zero");
        }
        if pattern_length == 0 {
            bail!("SNG2 pattern length must be non-zero");
        }
        if orders_length == 0 {
            bail!("SNG2 orders length must be non-zero");
        }
        if speed_pattern[..speed_pattern_len]
            .iter()
            .any(|&speed| speed == 0)
        {
            bail!("SNG2 speed pattern entries must be non-zero");
        }

        out.push(FurnaceSubsong {
            ticks_per_second,
            pattern_length,
            orders_length,
            virtual_tempo_num,
            virtual_tempo_den,
            speed_pattern_len,
            speed_pattern,
            name,
            orders,
        });
    }
    Ok(out)
}

fn parse_patterns(bytes: &[u8], _version: u16, pointers: &[u32]) -> Result<Vec<FurnacePattern>> {
    let mut out = Vec::with_capacity(pointers.len());
    for &ptr in pointers {
        let mut reader = CursorReader::at(bytes, ptr as usize)?;
        reader.expect_str("PATN")?;
        let block_len = reader.read_u32()? as usize;
        let block_end = reader
            .pos
            .checked_add(block_len)
            .ok_or_else(|| anyhow!("PATN block length overflow"))?;
        reader.ensure_within(block_end)?;

        let subsong_index = usize::from(reader.read_u8()?);
        let channel_index = usize::from(reader.read_u8()?);
        let pattern_index = reader.read_u16()?;
        let _name = reader.read_c_string()?;

        let mut rows = HashMap::new();
        let mut row_index = 0usize;
        while reader.pos < block_end {
            let control = reader.read_u8()?;
            if control == 0xFF {
                break;
            }
            if (control & 0x80) != 0 {
                row_index = row_index.saturating_add(usize::from(control & 0x7F) + 2);
                continue;
            }

            let mut effect_mask_lo = 0u8;
            let mut effect_mask_hi = 0u8;
            if (control & 0x20) != 0 {
                effect_mask_lo = reader.read_u8()?;
            }
            if (control & 0x40) != 0 {
                effect_mask_hi = reader.read_u8()?;
            }

            let has_note = (control & 0x01) != 0;
            let has_ins = (control & 0x02) != 0;
            let has_volume = (control & 0x04) != 0;
            let effect_present = (control & 0x08) != 0
                || (control & 0x10) != 0
                || effect_mask_lo != 0
                || effect_mask_hi != 0;

            if !has_note && !has_ins && !has_volume && !effect_present {
                row_index = row_index.saturating_add(1);
                continue;
            }

            let note = if has_note {
                Some(reader.read_u8()?)
            } else {
                None
            };
            if has_ins {
                let _ = reader.read_u8()?;
            }
            let volume = if has_volume {
                Some(reader.read_u8()?)
            } else {
                None
            };
            let mut effects = Vec::new();
            for slot in 0..8 {
                let cmd_present = if slot == 0 {
                    (control & 0x08) != 0 || (effect_mask_lo & 0x01) != 0
                } else if slot < 4 {
                    (effect_mask_lo & (1 << (slot * 2))) != 0
                } else {
                    (effect_mask_hi & (1 << ((slot - 4) * 2))) != 0
                };
                let val_present = if slot == 0 {
                    (control & 0x10) != 0 || (effect_mask_lo & 0x02) != 0
                } else if slot < 4 {
                    (effect_mask_lo & (1 << (slot * 2 + 1))) != 0
                } else {
                    (effect_mask_hi & (1 << ((slot - 4) * 2 + 1))) != 0
                };
                let mut command = None;
                let mut value = None;
                if cmd_present {
                    command = Some(reader.read_u8()?);
                }
                if val_present {
                    value = Some(reader.read_u8()?);
                }
                if let Some(command) = command {
                    let _ = slot;
                    effects.push(PatternEffect { command, value });
                }
            }

            rows.insert(
                row_index,
                PatternRow {
                    note,
                    volume,
                    effects,
                },
            );
            row_index = row_index.saturating_add(1);
        }

        out.push(FurnacePattern {
            subsong_index,
            channel_index,
            pattern_index,
            rows,
        });
    }
    Ok(out)
}

fn build_subsong_voices(
    display_name: &str,
    subsong_index: usize,
    subsong: &FurnaceSubsong,
    patterns: &HashMap<(usize, usize, u16), FurnacePattern>,
    ay_channel_offset: usize,
    warnings: &mut WarningCollector,
) -> Result<[Vec<PitchInterval>; 3]> {
    let mut voices: [Vec<PitchInterval>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut active: [Option<(u32, f64)>; 3] = [None, None, None];
    let mut raw_time = 0.0f64;
    let mut speed_cursor = 0usize;
    let mut order_index = 0usize;
    let mut row = 0usize;

    while order_index < subsong.orders_length {
        while row < subsong.pattern_length {
            let row_start = quantize_time(raw_time, warnings);
            let mut flow_control = None;
            for voice_index in 0..3 {
                let channel_index = ay_channel_offset + voice_index;
                let order_offset = channel_index * subsong.orders_length + order_index;
                let order_value = subsong.orders[order_offset];
                let row_data = patterns
                    .get(&(subsong_index, channel_index, u16::from(order_value)))
                    .and_then(|pattern| pattern.rows.get(&row));
                if let Some(row_data) = row_data {
                    for effect in &row_data.effects {
                        match effect.command {
                            0x0D => {
                                let target_order = match flow_control {
                                    Some(FlowControl::ScheduledJump {
                                        target_order,
                                        start_row: _,
                                    }) => target_order,
                                    _ => None,
                                };
                                flow_control = Some(FlowControl::ScheduledJump {
                                    target_order,
                                    start_row: usize::from(effect.value.unwrap_or(0)),
                                });
                            }
                            0x0B => {
                                let target = usize::from(effect.value.unwrap_or(0));
                                if target <= order_index {
                                    bail!(
                                        "{} subsong {} channel {} uses backward order jump 0B{:02X}, which cannot be linearized safely",
                                        display_name,
                                        subsong_index,
                                        char::from(b'A' + voice_index as u8),
                                        effect.value.unwrap_or(0)
                                    );
                                }
                                let start_row = match flow_control {
                                    Some(FlowControl::ScheduledJump {
                                        target_order: _,
                                        start_row,
                                    }) => start_row,
                                    _ => 0,
                                };
                                flow_control = Some(FlowControl::ScheduledJump {
                                    target_order: Some(target),
                                    start_row,
                                });
                            }
                            0x20 => {}
                            0xFF => flow_control = Some(FlowControl::Stop),
                            _ => {
                                bail!(
                                    "{} subsong {} channel {} uses unsupported effect {:02X}",
                                    display_name,
                                    subsong_index,
                                    char::from(b'A' + voice_index as u8),
                                    effect.command
                                );
                            }
                        }
                    }
                    if row_data.volume == Some(0) && row_data.note.is_none() {
                        close_active_interval(
                            &mut voices[voice_index],
                            &mut active[voice_index],
                            row_start,
                        );
                    }
                    if let Some(note) = row_data.note {
                        match note {
                            0..=179 => {
                                close_active_interval(
                                    &mut voices[voice_index],
                                    &mut active[voice_index],
                                    row_start,
                                );
                                if row_data.volume != Some(0) {
                                    let midi_pitch = furnace_note_to_midi_pitch(note)?;
                                    active[voice_index] = Some((row_start, midi_pitch));
                                }
                            }
                            180..=182 => {
                                close_active_interval(
                                    &mut voices[voice_index],
                                    &mut active[voice_index],
                                    row_start,
                                );
                            }
                            _ => bail!("invalid Furnace note value {}", note),
                        }
                    }
                }
            }

            let speed = f64::from(subsong.speed_pattern[speed_cursor % subsong.speed_pattern_len]);
            raw_time += speed * f64::from(subsong.virtual_tempo_den)
                / (f64::from(subsong.ticks_per_second) * f64::from(subsong.virtual_tempo_num))
                * HARMONY_TICKS_PER_SECOND;
            speed_cursor += 1;
            row += 1;

            match flow_control {
                Some(FlowControl::ScheduledJump {
                    target_order,
                    start_row,
                }) => {
                    order_index = target_order.unwrap_or(order_index + 1);
                    row = start_row;
                    break;
                }
                Some(FlowControl::Stop) => {
                    order_index = subsong.orders_length;
                    break;
                }
                None => {}
            }
        }
        if order_index >= subsong.orders_length {
            break;
        }
        if row >= subsong.pattern_length {
            order_index += 1;
            row = 0;
        }
    }

    let song_end = quantize_time(raw_time, warnings);
    for voice_index in 0..3 {
        close_active_interval(&mut voices[voice_index], &mut active[voice_index], song_end);
    }

    Ok(voices)
}

fn close_active_interval(
    target: &mut Vec<PitchInterval>,
    active: &mut Option<(u32, f64)>,
    end_tick: u32,
) {
    let Some((start_tick, pitch_value)) = active.take() else {
        return;
    };
    if end_tick <= start_tick {
        return;
    }
    target.push(PitchInterval {
        start: start_tick,
        end: end_tick,
        pitch_value,
    });
}

fn quantize_time(value: f64, warnings: &mut WarningCollector) -> u32 {
    let rounded = value.round();
    if (value - rounded).abs() > 1e-9 {
        warnings.note_quantized_event();
    }
    rounded.max(0.0) as u32
}

fn furnace_note_to_midi_pitch(note: u8) -> Result<f64> {
    let midi = i16::from(note) - 48;
    if !(0..=127).contains(&midi) {
        bail!(
            "Furnace note {} maps outside the supported MIDI range after AY conversion",
            note
        );
    }
    Ok(f64::from(midi))
}

fn is_legacy_compound_chip(chip_id: u16) -> bool {
    (0x0200..=0x04FF).contains(&chip_id)
}

struct CursorReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> CursorReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn at(bytes: &'a [u8], pos: usize) -> Result<Self> {
        if pos > bytes.len() {
            bail!("pointer 0x{pos:08X} is outside the file");
        }
        Ok(Self { bytes, pos })
    }

    fn ensure_within(&self, end: usize) -> Result<()> {
        if end > self.bytes.len() {
            bail!("block extends past end of file");
        }
        Ok(())
    }

    fn expect_bytes(&mut self, expected: &[u8]) -> Result<()> {
        let actual = self.read_vec(expected.len())?;
        if actual.as_slice() != expected {
            bail!("unexpected header bytes");
        }
        Ok(())
    }

    fn expect_str(&mut self, expected: &str) -> Result<()> {
        self.expect_bytes(expected.as_bytes())
    }

    fn read_u8(&mut self) -> Result<u8> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| anyhow!("unexpected end of file"))?;
        self.pos += 1;
        Ok(byte)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_f32(&mut self) -> Result<f32> {
        let bytes = self.read_array::<4>()?;
        Ok(f32::from_le_bytes(bytes))
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let end = self
            .pos
            .checked_add(N)
            .ok_or_else(|| anyhow!("offset overflow"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("unexpected end of file"))?;
        self.pos = end;
        Ok(slice.try_into().unwrap())
    }

    fn read_vec(&mut self, len: usize) -> Result<Vec<u8>> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| anyhow!("offset overflow"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("unexpected end of file"))?;
        self.pos = end;
        Ok(slice.to_vec())
    }

    fn read_c_string(&mut self) -> Result<String> {
        let Some(end) = self.bytes[self.pos..].iter().position(|&b| b == 0) else {
            bail!("unterminated string");
        };
        let string_bytes = &self.bytes[self.pos..self.pos + end];
        self.pos += end + 1;
        let value = std::str::from_utf8(string_bytes).context("invalid UTF-8 string")?;
        Ok(value.to_string())
    }

    fn skip(&mut self, len: usize) -> Result<()> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| anyhow!("offset overflow"))?;
        self.bytes
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("unexpected end of file"))?;
        self.pos = end;
        Ok(())
    }
}
