#include <errno.h>
#include <limits.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "harmony32_board.h"
#include "ym2149_core_standalone.h"

#ifndef YM2149_USE_MAME_CORE
#define YM2149_USE_MAME_CORE 1
#endif

#if !YM2149_USE_MAME_CORE
#error "Only the MAME-derived YM2149 core backend is currently implemented."
#endif

typedef struct {
    YM2149Core ym;
    uint8_t input_port_1;
} EmuUser;

typedef struct {
    uint64_t *slots;
    size_t cap;
    size_t used;
} HashSet64;

static uint8_t emu_in_port(void *user, uint8_t port) {
    EmuUser *u = (EmuUser *)user;
    if (port == 0x01) {
        return u->input_port_1;
    }
    return 0;
}

static void emu_out_port(void *user, uint8_t port, uint8_t value) {
    EmuUser *u = (EmuUser *)user;
    if (port == 0x03) {
        ym2149_write_address(&u->ym, value);
        return;
    }
    if (port == 0x02) {
        ym2149_write_data(&u->ym, value);
    }
}

static int16_t ym_next_pcm16(YM2149Core *ym) {
    double mix = ym2149_next_sample(ym);
    mix *= 0.8;
    if (mix > 1.0) {
        mix = 1.0;
    }
    if (mix < -1.0) {
        mix = -1.0;
    }
    return (int16_t)lrint(mix * 32767.0);
}

static void write_u16le(FILE *f, uint16_t v) {
    fputc((int)(v & 0xFFu), f);
    fputc((int)((v >> 8) & 0xFFu), f);
}

static void write_u32le(FILE *f, uint32_t v) {
    fputc((int)(v & 0xFFu), f);
    fputc((int)((v >> 8) & 0xFFu), f);
    fputc((int)((v >> 16) & 0xFFu), f);
    fputc((int)((v >> 24) & 0xFFu), f);
}

static int write_wav_header(FILE *f, uint32_t sample_rate, uint32_t sample_count) {
    uint32_t data_bytes = sample_count * 2u;
    uint32_t riff_size = 36u + data_bytes;

    if (fwrite("RIFF", 1, 4, f) != 4) return -1;
    write_u32le(f, riff_size);
    if (fwrite("WAVE", 1, 4, f) != 4) return -1;

    if (fwrite("fmt ", 1, 4, f) != 4) return -1;
    write_u32le(f, 16u);
    write_u16le(f, 1u);
    write_u16le(f, 1u);
    write_u32le(f, sample_rate);
    write_u32le(f, sample_rate * 2u);
    write_u16le(f, 2u);
    write_u16le(f, 16u);

    if (fwrite("data", 1, 4, f) != 4) return -1;
    write_u32le(f, data_bytes);
    return 0;
}

static uint64_t fnv1a64_update(uint64_t h, const void *data, size_t n) {
    const uint8_t *p = (const uint8_t *)data;
    size_t i;
    for (i = 0; i < n; i++) {
        h ^= (uint64_t)p[i];
        h *= 1099511628211ULL;
    }
    return h;
}

static uint64_t state_signature(const H32Board *board, const EmuUser *user) {
    uint64_t h = 1469598103934665603ULL;
    const uint8_t *regs = ym2149_regs(&user->ym);
    uint8_t selected = ym2149_selected_reg(&user->ym);
    h = fnv1a64_update(h, &board->ram[0x4000], 0x0D);
    h = fnv1a64_update(h, regs, 14);
    h = fnv1a64_update(h, &selected, 1);
    h = fnv1a64_update(h, &user->input_port_1, 1);
    if (h == 0) {
        h = 1;
    }
    return h;
}

static int hashset64_init(HashSet64 *hs, size_t cap_pow2) {
    size_t cap = (size_t)1u << cap_pow2;
    hs->slots = (uint64_t *)calloc(cap, sizeof(uint64_t));
    if (!hs->slots) {
        return -1;
    }
    hs->cap = cap;
    hs->used = 0;
    return 0;
}

static void hashset64_free(HashSet64 *hs) {
    free(hs->slots);
    hs->slots = NULL;
    hs->cap = 0;
    hs->used = 0;
}

/* Returns 1 if already present, 0 if inserted, -1 if table too full. */
static int hashset64_insert_or_seen(HashSet64 *hs, uint64_t key) {
    size_t mask;
    size_t idx;
    size_t i;
    if (key == 0) {
        key = 1;
    }
    if (hs->used * 10 >= hs->cap * 8) {
        return -1;
    }
    mask = hs->cap - 1;
    idx = (size_t)key & mask;
    for (i = 0; i < hs->cap; i++) {
        uint64_t cur = hs->slots[idx];
        if (cur == 0) {
            hs->slots[idx] = key;
            hs->used++;
            return 0;
        }
        if (cur == key) {
            return 1;
        }
        idx = (idx + 1u) & mask;
    }
    return -1;
}

static void usage(const char *argv0) {
    fprintf(stderr,
            "Usage: %s <rom.bin> <out.wav> [song_index=0] [bank_index=0] [speed=3] [drums_on=1] [sample_rate=44100] [cpu_hz=2000000] [tail_ms=0] [max_seconds=600]\n",
            argv0);
}

int main(int argc, char **argv) {
    const char *rom_path;
    const char *wav_path;
    int song_index = 0;
    uint32_t bank = 0;
    int speed = 3;
    int drums_on = 1;
    int sample_rate = 44100;
    int cpu_hz = 2000000;
    int tail_ms = 0;
    int max_seconds = 600;

    FILE *rf = NULL;
    FILE *wf = NULL;
    H32Board board;
    EmuUser user;
    HashSet64 loop_set;
    int loop_set_enabled = 1;
    long rom_size;
    size_t nread;
    uint8_t *rom_image = NULL;
    uint32_t total_samples_max;
    uint32_t produced = 0;
    double sample_cursor = 0.0;
    uint64_t steps = 0;
    int song_ended = 0;
    int loop_detected = 0;

    if (argc < 3) {
        usage(argv[0]);
        return 1;
    }

    rom_path = argv[1];
    wav_path = argv[2];

    if (argc >= 4) {
        song_index = atoi(argv[3]);
    }
    if (argc >= 5) {
        char *end = NULL;
        unsigned long parsed_bank;
        errno = 0;
        parsed_bank = strtoul(argv[4], &end, 10);
        if (errno != 0 || !end || *end != '\0' || parsed_bank > UINT32_MAX) {
            fprintf(stderr, "Invalid bank index: %s\n", argv[4]);
            usage(argv[0]);
            return 1;
        }
        bank = (uint32_t)parsed_bank;
    }
    if (argc >= 6) {
        speed = atoi(argv[5]);
    }
    if (argc >= 7) {
        drums_on = atoi(argv[6]);
    }
    if (argc >= 8) {
        sample_rate = atoi(argv[7]);
    }
    if (argc >= 9) {
        cpu_hz = atoi(argv[8]);
    }
    if (argc >= 10) {
        tail_ms = atoi(argv[9]);
    }
    if (argc >= 11) {
        max_seconds = atoi(argv[10]);
    }

    if (sample_rate <= 0 || cpu_hz <= 0 || song_index < 0 || song_index > 15 ||
        speed < 0 || speed > 3 || (drums_on != 0 && drums_on != 1) || tail_ms < 0 || max_seconds <= 0) {
        fprintf(stderr, "Invalid arguments.\n");
        usage(argv[0]);
        return 1;
    }

    memset(&board, 0, sizeof(board));
    memset(&user, 0, sizeof(user));
    memset(&loop_set, 0, sizeof(loop_set));

    ym2149_init(&user.ym, 2000000u, (uint32_t)sample_rate);
    /* Port 0x01 mapping from firmware:
     * bits 0..3 tune, bits 4..5 speed, bit 6 drums/noise mode.
     * drums_on=1 -> bit6=0, drums_on=0 -> bit6=1.
     */
    user.input_port_1 = (uint8_t)((song_index & 0x0F) | ((speed & 0x03) << 4) | (drums_on ? 0x00 : 0x40));

    h32_board_init(&board, emu_in_port, emu_out_port, &user);

    rf = fopen(rom_path, "rb");
    if (!rf) {
        fprintf(stderr, "Failed to open ROM '%s': %s\n", rom_path, strerror(errno));
        return 1;
    }

    if (fseek(rf, 0, SEEK_END) != 0) {
        fprintf(stderr, "Failed to seek ROM.\n");
        fclose(rf);
        return 1;
    }
    rom_size = ftell(rf);
    if (rom_size <= 0 || (unsigned long)rom_size > UINT32_MAX) {
        fprintf(stderr, "Unexpected ROM size: %ld\n", rom_size);
        fclose(rf);
        return 1;
    }
    if (fseek(rf, 0, SEEK_SET) != 0) {
        fprintf(stderr, "Failed to rewind ROM.\n");
        fclose(rf);
        return 1;
    }

    rom_image = (uint8_t *)malloc((size_t)rom_size);
    if (!rom_image) {
        fprintf(stderr, "Failed to allocate ROM buffer (%ld bytes).\n", rom_size);
        fclose(rf);
        return 1;
    }
    nread = fread(rom_image, 1, (size_t)rom_size, rf);
    fclose(rf);
    if (nread != (size_t)rom_size) {
        fprintf(stderr, "Failed to read ROM bytes.\n");
        free(rom_image);
        return 1;
    }

    if (h32_board_load_rom(&board, rom_image, (uint32_t)rom_size) != 0) {
        fprintf(stderr, "Failed to load ROM into board.\n");
        free(rom_image);
        h32_board_deinit(&board);
        return 1;
    }
    free(rom_image);
    rom_image = NULL;

    if (!h32_board_bank_available(&board, bank)) {
        fprintf(stderr, "Requested bank %u not present in ROM (size=%ld, banks=%u).\n", bank, rom_size,
                h32_board_bank_count(&board));
        h32_board_deinit(&board);
        return 1;
    }
    h32_board_set_bank_pin(&board, bank);

    total_samples_max = (uint32_t)max_seconds * (uint32_t)sample_rate;

    wf = fopen(wav_path, "wb+");
    if (!wf) {
        fprintf(stderr, "Failed to open WAV '%s': %s\n", wav_path, strerror(errno));
        h32_board_deinit(&board);
        return 1;
    }

    if (write_wav_header(wf, (uint32_t)sample_rate, 0) != 0) {
        fprintf(stderr, "Failed to write WAV header.\n");
        fclose(wf);
        h32_board_deinit(&board);
        return 1;
    }

    if (h32_board_reset_song(&board) != 0) {
        fprintf(stderr, "Music engine ended during init (song %d, bank %u).\n", song_index, bank);
        fclose(wf);
        h32_board_deinit(&board);
        return 1;
    }

    if (hashset64_init(&loop_set, 20) != 0) {
        loop_set_enabled = 0;
    } else {
        (void)hashset64_insert_or_seen(&loop_set, state_signature(&board, &user));
    }

    {
        int16_t last_sample = 0;
    while (produced < total_samples_max) {
        uint32_t tstates = 0;
        uint32_t target_samples;
        uint32_t frame_samples;
        int alive = h32_board_step_music_tick(&board, &tstates);

        steps++;

        if (alive <= 0) {
            song_ended = 1;
            break;
        }

        if (loop_set_enabled) {
            int seen = hashset64_insert_or_seen(&loop_set, state_signature(&board, &user));
            if (seen == 1 && steps > 2048) {
                loop_detected = 1;
                break;
            }
            if (seen < 0) {
                loop_set_enabled = 0;
            }
        }

        if (tstates == 0) {
            tstates = 1;
        }

        sample_cursor += ((double)tstates * (double)sample_rate) / (double)cpu_hz;
        target_samples = (uint32_t)sample_cursor;
        if (target_samples <= produced) {
            target_samples = produced + 1;
        }
        frame_samples = target_samples - produced;

        if (produced + frame_samples > total_samples_max) {
            frame_samples = total_samples_max - produced;
        }

        while (frame_samples > 0) {
            int16_t s = ym_next_pcm16(&user.ym);
            write_u16le(wf, (uint16_t)s);
            last_sample = s;
            frame_samples--;
        }

        produced = target_samples;
        if (produced > total_samples_max) {
            produced = total_samples_max;
        }
    }

    if ((song_ended || loop_detected) && produced < total_samples_max) {
        uint32_t tail_samples = (uint32_t)(((uint64_t)tail_ms * (uint64_t)sample_rate) / 1000ULL);
        if (tail_samples > total_samples_max - produced) {
            tail_samples = total_samples_max - produced;
        }
        while (tail_samples > 0) {
            /* Hold last rendered value; do not advance chip state after song end. */
            write_u16le(wf, (uint16_t)last_sample);
            tail_samples--;
            produced++;
        }
    }
    }

    if (fseek(wf, 0, SEEK_SET) != 0 || write_wav_header(wf, (uint32_t)sample_rate, produced) != 0) {
        fprintf(stderr, "Failed to finalize WAV header.\n");
        fclose(wf);
        hashset64_free(&loop_set);
        h32_board_deinit(&board);
        return 1;
    }

    fclose(wf);
    hashset64_free(&loop_set);
    h32_board_deinit(&board);

    fprintf(stderr,
            "Wrote %s (%u samples, %.3f s, song %d, bank %u, speed=%d, drums_on=%d, %d Hz, cpu=%d Hz, in1=0x%02X, reason=%s).\n",
            wav_path,
            produced,
            (double)produced / (double)sample_rate,
            song_index,
            bank,
            speed,
            drums_on,
            sample_rate,
            cpu_hz,
            user.input_port_1,
            song_ended ? "song_end" : (loop_detected ? "loop_detected" : "max_seconds"));

    return 0;
}
