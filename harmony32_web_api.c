#include "harmony32_web_api.h"

#include <stdlib.h>
#include <string.h>

#include "harmony32_board.h"
#include "ym2149_core_standalone.h"

#ifdef __EMSCRIPTEN__
#include <emscripten/emscripten.h>
#define H32_KEEPALIVE EMSCRIPTEN_KEEPALIVE
#else
#define H32_KEEPALIVE
#endif

struct H32Engine {
    H32Board board;
    YM2149Core ym;

    uint8_t song;
    uint32_t bank;
    uint8_t speed;
    uint8_t drums_on;
    uint8_t running;
    uint8_t mix_mode;

    uint8_t input_port_1;
    uint8_t initialized;
    uint8_t song_ended;

    uint32_t sample_rate;
    uint32_t cpu_hz;
    uint32_t steps;

    double sample_cursor;
    double channel_level[3];
    double channel_pan[3];
};

static int h32_reinitialize_song(H32Engine *e);

static int h32_advance_to_sample(H32Engine *engine) {
    while (engine->sample_cursor <= 0.0) {
        uint32_t tstates = 0;
        int alive = h32_board_step_music_tick(&engine->board, &tstates);

        if (alive <= 0) {
            engine->song_ended = 1;
            if (h32_reinitialize_song(engine) != 0) {
                return -1;
            }
            continue;
        }

        engine->steps++;
        if (tstates == 0) {
            tstates = 1;
        }
        engine->sample_cursor += ((double)tstates * (double)engine->sample_rate) / (double)engine->cpu_hz;
    }
    return 0;
}

static uint8_t h32_in_port(void *user, uint8_t port) {
    H32Engine *e = (H32Engine *)user;
    if (port == 0x01) {
        return e->input_port_1;
    }
    return 0;
}

static void h32_out_port(void *user, uint8_t port, uint8_t value) {
    H32Engine *e = (H32Engine *)user;
    if (port == 0x03) {
        ym2149_write_address(&e->ym, value);
        return;
    }
    if (port == 0x02) {
        ym2149_write_data(&e->ym, value);
    }
}

static void h32_update_input_port(H32Engine *e) {
    e->input_port_1 = (uint8_t)((e->song & 0x0Fu) | ((e->speed & 0x03u) << 4) | (e->drums_on ? 0x00u : 0x40u));
}

static int h32_reinitialize_song(H32Engine *e) {
    uint32_t bank_count;
    ym2149_reset(&e->ym);
    e->sample_cursor = 0.0;
    e->song_ended = 0;

    h32_update_input_port(e);
    bank_count = h32_board_bank_count(&e->board);
    if (bank_count == 0) {
        e->initialized = 0;
        e->song_ended = 1;
        return -1;
    }
    if (e->bank >= bank_count) {
        e->bank = 0;
    }
    h32_board_set_bank_pin(&e->board, e->bank);
    if (!h32_board_bank_available(&e->board, e->bank)) {
        e->initialized = 0;
        e->song_ended = 1;
        return -1;
    }

    if (h32_board_reset_full(&e->board) != 0) {
        e->initialized = 0;
        e->song_ended = 1;
        return -1;
    }

    e->initialized = 1;
    return 0;
}

H32_KEEPALIVE H32Engine *h32_create(uint32_t sample_rate, uint32_t cpu_hz) {
    H32Engine *e;
    if (sample_rate == 0 || cpu_hz == 0) {
        return NULL;
    }

    e = (H32Engine *)calloc(1, sizeof(*e));
    if (!e) {
        return NULL;
    }

    e->sample_rate = sample_rate;
    e->cpu_hz = cpu_hz;
    e->song = 0;
    e->bank = 0;
    e->speed = 3;
    e->drums_on = 1;
    e->running = 1;
    e->mix_mode = 1;
    e->channel_level[0] = 1.0;
    e->channel_level[1] = 1.0;
    e->channel_level[2] = 1.0;
    e->channel_pan[0] = 0.0;
    e->channel_pan[1] = 0.0;
    e->channel_pan[2] = 0.0;

    ym2149_init(&e->ym, 2000000u, sample_rate);

    h32_board_init(&e->board, h32_in_port, h32_out_port, e);
    h32_update_input_port(e);

    return e;
}

H32_KEEPALIVE void h32_destroy(H32Engine *engine) {
    if (engine) {
        h32_board_deinit(&engine->board);
    }
    free(engine);
}

H32_KEEPALIVE int h32_load_rom(H32Engine *engine, const uint8_t *rom, uint32_t rom_len) {
    if (!engine || !rom || rom_len == 0) {
        return -1;
    }

    if (h32_board_load_rom(&engine->board, rom, rom_len) != 0) {
        return -1;
    }

    return h32_reinitialize_song(engine);
}

H32_KEEPALIVE int h32_set_controls(H32Engine *engine, uint8_t song, uint32_t bank, uint8_t speed, uint8_t drums_on,
                                   uint8_t running) {
    uint8_t song_changed;
    uint8_t bank_changed;
    uint32_t bank_count;

    if (!engine || song > 15 || speed > 3 || (drums_on != 0 && drums_on != 1) ||
        (running != 0 && running != 1)) {
        return -1;
    }
    bank_count = h32_board_bank_count(&engine->board);
    if (bank_count == 0 || bank >= bank_count) {
        return -1;
    }

    song_changed = (uint8_t)(engine->song != song);
    bank_changed = (uint8_t)(engine->bank != bank ? 1u : 0u);

    engine->song = song;
    engine->bank = bank;
    engine->speed = speed;
    engine->drums_on = drums_on;
    engine->running = running;
    h32_update_input_port(engine);

    if (bank_changed && !song_changed) {
        if (!h32_board_bank_available(&engine->board, bank)) {
            return -1;
        }
        h32_board_set_bank_pin(&engine->board, bank);
    }

    if (song_changed) {
        return h32_reinitialize_song(engine);
    }

    return 0;
}

H32_KEEPALIVE int h32_set_cpu_hz(H32Engine *engine, uint32_t cpu_hz) {
    double ratio;

    if (!engine || cpu_hz == 0) {
        return -1;
    }

    if (engine->cpu_hz == cpu_hz) {
        return 0;
    }

    ratio = (double)engine->cpu_hz / (double)cpu_hz;
    engine->cpu_hz = cpu_hz;
    engine->sample_cursor *= ratio;
    return 0;
}

H32_KEEPALIVE int h32_set_ym_hz(H32Engine *engine, uint32_t ym_hz) {
    if (!engine || ym_hz == 0) {
        return -1;
    }

    if (engine->ym.chip_clock_hz == ym_hz) {
        return 0;
    }

    ym2149_set_clock(&engine->ym, ym_hz);
    return 0;
}

H32_KEEPALIVE int h32_set_channel_mix(H32Engine *engine, uint8_t channel, int32_t level_pct, int32_t pan_pct) {
    if (!engine || channel > 2 || level_pct < 0 || level_pct > 100 || pan_pct < -100 || pan_pct > 100) {
        return -1;
    }

    engine->channel_level[channel] = (double)level_pct / 100.0;
    engine->channel_pan[channel] = (double)pan_pct / 100.0;
    return 0;
}

H32_KEEPALIVE int h32_set_mix_mode(H32Engine *engine, uint8_t mix_mode) {
    if (!engine || mix_mode > 1u) {
        return -1;
    }
    engine->mix_mode = mix_mode;
    return 0;
}

H32_KEEPALIVE int h32_reset_song(H32Engine *engine, uint8_t song, uint32_t bank) {
    if (!engine || song > 15 || !h32_board_bank_available(&engine->board, bank)) {
        return -1;
    }

    engine->song = song;
    engine->bank = bank;
    h32_update_input_port(engine);
    return h32_reinitialize_song(engine);
}

H32_KEEPALIVE int h32_reset_cpu(H32Engine *engine) {
    if (!engine || !h32_board_bank_available(&engine->board, engine->bank)) {
        return -1;
    }
    if (h32_board_reset_cpu(&engine->board) != 0) {
        return -1;
    }
    engine->song_ended = 0;
    engine->initialized = 1;
    return 0;
}

H32_KEEPALIVE int h32_reset_full(H32Engine *engine, uint8_t song, uint32_t bank) {
    return h32_reset_song(engine, song, bank);
}

H32_KEEPALIVE uint32_t h32_render(H32Engine *engine, float *out, uint32_t frames) {
    uint32_t produced = 0;

    if (!engine || !out || frames == 0) {
        return 0;
    }

    if (!engine->initialized || !engine->running || engine->song_ended) {
        memset(out, 0, (size_t)frames * 2u * sizeof(float));
        return frames;
    }

    while (produced < frames) {
        if (h32_advance_to_sample(engine) != 0) {
            while (produced < frames) {
                const size_t o = (size_t)produced * 2u;
                out[o] = 0.0f;
                out[o + 1u] = 0.0f;
                produced++;
            }
            return produced;
        }

        {
            double ch[3];
            double left = 0.0;
            double right = 0.0;
            size_t o;
            (void)ym2149_next_sample_channels(&engine->ym, ch);

            for (uint8_t i = 0; i < 3; i++) {
                double level = engine->channel_level[i];
                double pan = engine->channel_pan[i];
                double l_gain = (1.0 - pan) * 0.5;
                double r_gain = (1.0 + pan) * 0.5;
                double s = ch[i] * level;
                left += s * l_gain;
                right += s * r_gain;
            }

            left *= 0.8;
            right *= 0.8;

            if (left > 1.0) {
                left = 1.0;
            }
            if (left < -1.0) {
                left = -1.0;
            }
            if (right > 1.0) {
                right = 1.0;
            }
            if (right < -1.0) {
                right = -1.0;
            }

            o = (size_t)produced * 2u;
            out[o] = (float)left;
            out[o + 1u] = (float)right;
            produced++;
            engine->sample_cursor -= 1.0;
        }
    }

    return produced;
}

H32_KEEPALIVE uint32_t h32_render_stems(H32Engine *engine, float *out, uint32_t frames) {
    uint32_t produced = 0;

    if (!engine || !out || frames == 0) {
        return 0;
    }

    if (!engine->initialized || !engine->running || engine->song_ended) {
        memset(out, 0, (size_t)frames * 3u * sizeof(float));
        return frames;
    }

    while (produced < frames) {
        double ch[3];
        size_t o;
        if (h32_advance_to_sample(engine) != 0) {
            while (produced < frames) {
                o = (size_t)produced * 3u;
                out[o] = 0.0f;
                out[o + 1u] = 0.0f;
                out[o + 2u] = 0.0f;
                produced++;
            }
            return produced;
        }

        (void)ym2149_next_sample_channels(&engine->ym, ch);
        o = (size_t)produced * 3u;
        out[o] = (float)ch[0];
        out[o + 1u] = (float)ch[1];
        out[o + 2u] = (float)ch[2];
        produced++;
        engine->sample_cursor -= 1.0;
    }

    return produced;
}

H32_KEEPALIVE int h32_get_status(const H32Engine *engine, H32Status *out_status) {
    if (!engine || !out_status) {
        return -1;
    }

    memset(out_status, 0, sizeof(*out_status));
    out_status->song_ended = engine->song_ended;
    out_status->initialized = engine->initialized;
    out_status->running = engine->running;
    out_status->song = engine->song;
    out_status->mix_mode = engine->mix_mode;
    out_status->bank = engine->bank;
    out_status->bank_count = h32_board_bank_count(&engine->board);
    out_status->speed = engine->speed;
    out_status->drums_on = engine->drums_on;
    out_status->sample_rate = engine->sample_rate;
    out_status->cpu_hz = engine->cpu_hz;
    out_status->ym_hz = engine->ym.chip_clock_hz;
    out_status->steps = engine->steps;

    return 0;
}

H32_KEEPALIVE uint32_t h32_get_bank_count(const H32Engine *engine) {
    if (!engine) {
        return 0;
    }
    return h32_board_bank_count(&engine->board);
}
