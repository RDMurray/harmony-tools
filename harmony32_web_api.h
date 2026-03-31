#ifndef HARMONY32_WEB_API_H
#define HARMONY32_WEB_API_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct H32Engine H32Engine;

enum {
    H32_YM_BACKEND_MAME = 0,
    H32_YM_BACKEND_FURNACE = 1
};

typedef struct {
    uint8_t song_ended;
    uint8_t initialized;
    uint8_t running;
    uint8_t ym_render_mode;
    uint8_t song;
    uint8_t speed;
    uint8_t drums_on;
    uint8_t mix_mode;
    uint32_t bank;
    uint32_t bank_count;
    uint32_t sample_rate;
    uint32_t cpu_hz;
    uint32_t ym_hz;
    uint32_t steps;
    uint32_t ym_backend;
} H32Status;

H32Engine *h32_create(uint32_t sample_rate, uint32_t cpu_hz);
void h32_destroy(H32Engine *engine);

int h32_load_rom(H32Engine *engine, const uint8_t *rom, uint32_t rom_len);
int h32_set_controls(H32Engine *engine, uint8_t song, uint32_t bank, uint8_t speed, uint8_t drums_on, uint8_t running);
int h32_set_cpu_hz(H32Engine *engine, uint32_t cpu_hz);
int h32_set_ym_hz(H32Engine *engine, uint32_t ym_hz);
int h32_set_ym_backend(H32Engine *engine, uint8_t backend);
int h32_set_ym_render_mode(H32Engine *engine, uint8_t render_mode);
int h32_set_channel_mix(H32Engine *engine, uint8_t channel, int32_t level_pct, int32_t pan_pct);
int h32_set_mix_mode(H32Engine *engine, uint8_t mix_mode);
int h32_reset_song(H32Engine *engine, uint8_t song, uint32_t bank);
int h32_reset_cpu(H32Engine *engine);
int h32_reset_full(H32Engine *engine, uint8_t song, uint32_t bank);
uint32_t h32_get_bank_count(const H32Engine *engine);

uint32_t h32_render(H32Engine *engine, float *out, uint32_t frames);
uint32_t h32_render_stems(H32Engine *engine, float *out, uint32_t frames);
int h32_get_status(const H32Engine *engine, H32Status *out_status);

#ifdef __cplusplus
}
#endif

#endif
