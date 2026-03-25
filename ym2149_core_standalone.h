/*
 * SPDX-License-Identifier: BSD-3-Clause
 *
 * Standalone YM2149 core adapted from the MAME AY-3-8910 / YM2149
 * implementation lineage, primarily `src/devices/sound/ay8910.cpp`.
 *
 * Copyright (c) Couriersud and MAME contributors.
 * Additional Harmony32 adaptations Copyright (c) Harmony32 contributors.
 */

#ifndef YM2149_CORE_STANDALONE_H
#define YM2149_CORE_STANDALONE_H

#include <stdint.h>

typedef struct {
    uint8_t regs[16];
    uint8_t selected_reg;
    uint8_t active;

    struct {
        uint32_t period;
        uint8_t volume;
        int32_t count;
        uint8_t duty_cycle;
        uint8_t output;
    } tone[3];

    struct {
        uint32_t period;
        int32_t count;
        int8_t step;
        uint32_t volume;
        uint8_t hold;
        uint8_t alternate;
        uint8_t attack;
        uint8_t holding;
    } envelope;

    uint8_t vol_enabled[3];
    uint8_t prescale_noise;
    int16_t count_noise;
    uint32_t rng;

    double vol_table[16];
    double env_table[32];

    uint32_t chip_clock_hz;
    uint32_t output_sample_rate;
    uint32_t chip_sample_rate;
    uint64_t step_accum;
    double last_output;
} YM2149Core;

void ym2149_init(YM2149Core *ym, uint32_t clock_hz, uint32_t sample_rate);
void ym2149_set_clock(YM2149Core *ym, uint32_t clock_hz);
void ym2149_reset(YM2149Core *ym);
void ym2149_write_address(YM2149Core *ym, uint8_t reg);
void ym2149_write_data(YM2149Core *ym, uint8_t value);
double ym2149_next_sample(YM2149Core *ym);
double ym2149_next_sample_channels(YM2149Core *ym, double out_channels[3]);

static inline const uint8_t *ym2149_regs(const YM2149Core *ym) { return ym->regs; }
static inline uint8_t ym2149_selected_reg(const YM2149Core *ym) { return ym->selected_reg; }

#endif
