/*
 * SPDX-License-Identifier: BSD-3-Clause
 *
 * Standalone YM2149 core adapted from the MAME AY-3-8910 / YM2149
 * implementation lineage, primarily `src/devices/sound/ay8910.cpp`.
 *
 * Copyright (c) Couriersud and MAME contributors.
 * Additional Harmony32 adaptations Copyright (c) Harmony32 contributors.
 */

#include "ym2149_core_standalone.h"

#include <math.h>
#include <string.h>

typedef struct {
    double r_up;
    double r_down;
    int res_count;
    double res[32];
} AYParam;

#define BIT(v, b) (((v) >> (b)) & 1u)

enum {
    AY_AFINE = 0x00,
    AY_ACOARSE = 0x01,
    AY_BFINE = 0x02,
    AY_BCOARSE = 0x03,
    AY_CFINE = 0x04,
    AY_CCOARSE = 0x05,
    AY_NOISEPER = 0x06,
    AY_ENABLE = 0x07,
    AY_AVOL = 0x08,
    AY_BVOL = 0x09,
    AY_CVOL = 0x0A,
    AY_EAFINE = 0x0B,
    AY_EACOARSE = 0x0C,
    AY_EASHAPE = 0x0D,
    AY_PORTA = 0x0E
};

static uint8_t ym2149_normalize_backend(uint8_t backend) {
    return (backend == YM2149_BACKEND_FURNACE) ? YM2149_BACKEND_FURNACE : YM2149_BACKEND_MAME;
}

static const AYParam ym2149_param = {
    630, 801, 16,
    {73770, 37586, 27458, 21451, 15864, 12371, 8922, 6796,
     4763, 3521, 2403, 1737, 1123, 762, 438, 251}
};

static const AYParam ym2149_param_env = {
    630, 801, 32,
    {103350, 73770, 52657, 37586, 32125, 27458, 24269, 21451,
     18447, 15864, 14009, 12371, 10506, 8922, 7787, 6796,
     5689, 4763, 4095, 3521, 2909, 2403, 2043, 1737,
     1397, 1123, 925, 762, 578, 438, 332, 251}
};

static void build_single_table(double rl, const AYParam *par, int normalize, double *tab, int zero_is_off) {
    double temp[32];
    double min = 10.0;
    double max = 0.0;

    for (int j = 0; j < par->res_count; j++) {
        double rt = 1.0 / par->r_down + 1.0 / rl;
        double rw = 1.0 / par->res[j];
        rt += 1.0 / par->res[j];

        if (!(zero_is_off && j == 0)) {
            rw += 1.0 / par->r_up;
            rt += 1.0 / par->r_up;
        }

        temp[j] = rw / rt;
        if (temp[j] < min) min = temp[j];
        if (temp[j] > max) max = temp[j];
    }

    if (normalize) {
        for (int j = 0; j < par->res_count; j++) {
            tab[j] = (((temp[j] - min) / (max - min)) - 0.25) * 0.5;
        }
    } else {
        for (int j = 0; j < par->res_count; j++) {
            tab[j] = temp[j];
        }
    }
}

static void envelope_set_shape(YM2149Core *ym, uint8_t shape, uint8_t mask) {
    ym->envelope.attack = (shape & 0x04) ? mask : 0x00;
    if ((shape & 0x08) == 0) {
        ym->envelope.hold = 1;
        ym->envelope.alternate = ym->envelope.attack;
    } else {
        ym->envelope.hold = shape & 0x01;
        ym->envelope.alternate = (shape >> 1) & 0x01;
    }
    ym->envelope.step = (int8_t)mask;
    ym->envelope.holding = 0;
    ym->envelope.volume = (uint32_t)(ym->envelope.step ^ ym->envelope.attack);
}

static double ym2149_mix_current_output(const YM2149Core *ym, double out_channels[3]) {
    double mix = 0.0;

    for (int chan = 0; chan < 3; chan++) {
        uint8_t vol = ym->tone[chan].volume;
        double s;

        if (vol & 0x10) {
            uint8_t idx = (uint8_t)(ym->vol_enabled[chan] ? (ym->envelope.volume & 0x1F) : 0u);
            s = ym->env_table[idx];
        } else {
            uint8_t idx = (uint8_t)(ym->vol_enabled[chan] ? (vol & 0x0F) : 0u);
            s = ym->vol_table[idx];
        }

        if (out_channels) {
            out_channels[chan] = s;
        }
        mix += s;
    }

    return mix;
}

static void ym2149_seed_resampler(YM2149Core *ym) {
    ym->resample_phase = 0.0;
    ym->resample_curr_output = ym2149_mix_current_output(ym, ym->resample_curr_channels);
    ym->resample_prev_output = ym->resample_curr_output;
    for (int chan = 0; chan < 3; chan++) {
        ym->resample_prev_channels[chan] = ym->resample_curr_channels[chan];
    }
    ym->last_output = ym->resample_curr_output;
    ym->resample_seeded = 1;
}

const char *ym2149_backend_name(uint8_t backend) {
    return ym2149_normalize_backend(backend) == YM2149_BACKEND_FURNACE ? "furnace" : "mame";
}

static void ym2149_write_reg(YM2149Core *ym, int reg, uint8_t value) {
    uint8_t coarse;

    ym->regs[reg & 0x0F] = value;

    switch (reg & 0x0F) {
    case AY_AFINE:
    case AY_ACOARSE:
        coarse = ym->regs[AY_ACOARSE] & 0x0F;
        ym->tone[0].period = (uint32_t)ym->regs[AY_AFINE] | ((uint32_t)coarse << 8);
        break;
    case AY_BFINE:
    case AY_BCOARSE:
        coarse = ym->regs[AY_BCOARSE] & 0x0F;
        ym->tone[1].period = (uint32_t)ym->regs[AY_BFINE] | ((uint32_t)coarse << 8);
        break;
    case AY_CFINE:
    case AY_CCOARSE:
        coarse = ym->regs[AY_CCOARSE] & 0x0F;
        ym->tone[2].period = (uint32_t)ym->regs[AY_CFINE] | ((uint32_t)coarse << 8);
        break;
    case AY_AVOL:
        ym->tone[0].volume = ym->regs[AY_AVOL];
        break;
    case AY_BVOL:
        ym->tone[1].volume = ym->regs[AY_BVOL];
        break;
    case AY_CVOL:
        ym->tone[2].volume = ym->regs[AY_CVOL];
        break;
    case AY_EAFINE:
    case AY_EACOARSE:
        ym->envelope.period = (uint32_t)ym->regs[AY_EAFINE] | ((uint32_t)ym->regs[AY_EACOARSE] << 8);
        break;
    case AY_EASHAPE:
        envelope_set_shape(ym, ym->regs[AY_EASHAPE], 0x1F);
        break;
    default:
        break;
    }
}

void ym2149_init(YM2149Core *ym, uint32_t clock_hz, uint32_t sample_rate) {
    ym2149_init_backend(ym, clock_hz, sample_rate, YM2149_BACKEND_MAME);
}

void ym2149_init_backend(YM2149Core *ym, uint32_t clock_hz, uint32_t sample_rate, uint8_t backend) {
    memset(ym, 0, sizeof(*ym));

    ym->chip_clock_hz = clock_hz;
    ym->output_sample_rate = sample_rate;
    ym->chip_sample_rate = (clock_hz > 7) ? (clock_hz / 8u) : 1u;
    ym->backend_kind = ym2149_normalize_backend(backend);
    ym->render_mode = YM2149_RENDER_RESAMPLED;

    build_single_table(1000.0, &ym2149_param, 1, ym->vol_table, 0);
    build_single_table(1000.0, &ym2149_param_env, 1, ym->env_table, 0);

    ym2149_reset(ym);
}

void ym2149_set_clock(YM2149Core *ym, uint32_t clock_hz) {
    if (!ym || clock_hz == 0) {
        return;
    }

    ym->chip_clock_hz = clock_hz;
    ym->chip_sample_rate = (clock_hz > 7) ? (clock_hz / 8u) : 1u;
}

void ym2149_set_backend(YM2149Core *ym, uint8_t backend) {
    if (!ym) {
        return;
    }

    ym->backend_kind = ym2149_normalize_backend(backend);
    ym2149_reset(ym);
}

void ym2149_set_render_mode(YM2149Core *ym, uint8_t mode) {
    if (!ym) {
        return;
    }

    ym->render_mode = (mode == YM2149_RENDER_DIRECT) ? YM2149_RENDER_DIRECT : YM2149_RENDER_RESAMPLED;
    ym->step_accum = 0;
    ym->resample_seeded = 0;
    ym2149_seed_resampler(ym);
}

void ym2149_reset(YM2149Core *ym) {
    ym->active = 0;
    ym->selected_reg = 0;
    ym->rng = 1;
    ym->count_noise = 0;
    ym->prescale_noise = 0;
    ym->step_accum = 0;
    ym->resample_phase = 0.0;
    ym->resample_seeded = 0;
    ym->resample_prev_output = 0.0;
    ym->resample_curr_output = 0.0;
    ym->last_output = 0.0;

    memset(ym->regs, 0, sizeof(ym->regs));
    memset(ym->vol_enabled, 0, sizeof(ym->vol_enabled));
    memset(ym->tone, 0, sizeof(ym->tone));
    memset(&ym->envelope, 0, sizeof(ym->envelope));
    memset(ym->resample_prev_channels, 0, sizeof(ym->resample_prev_channels));
    memset(ym->resample_curr_channels, 0, sizeof(ym->resample_curr_channels));

    for (int i = 0; i < AY_PORTA; i++) {
        ym2149_write_reg(ym, i, 0);
    }

    ym2149_seed_resampler(ym);
}

void ym2149_write_address(YM2149Core *ym, uint8_t reg) {
    ym->active = ((reg >> 4) == 0) ? 1 : 0;
    if (ym->active) {
        ym->selected_reg = reg & 0x0F;
    }
}

void ym2149_write_data(YM2149Core *ym, uint8_t value) {
    if (!ym->active) {
        return;
    }
    ym2149_write_reg(ym, ym->selected_reg, value);
}

static double ym2149_step_chip_sample(YM2149Core *ym, double out_channels[3]) {
    for (int chan = 0; chan < 3; chan++) {
        uint32_t period = ym->tone[chan].period;
        if (period < 1) {
            period = 1;
        }
        ym->tone[chan].count += 1;
        while ((uint32_t)ym->tone[chan].count >= period) {
            ym->tone[chan].duty_cycle = (uint8_t)((ym->tone[chan].duty_cycle - 1u) & 0x1F);
            ym->tone[chan].output = (uint8_t)BIT(ym->tone[chan].duty_cycle, 0);
            ym->tone[chan].count -= (int32_t)period;
        }
    }

    {
        uint8_t period_noise = (uint8_t)(ym->regs[AY_NOISEPER] & 0x1F);
        if (ym->backend_kind == YM2149_BACKEND_FURNACE && period_noise == 0) {
            period_noise = 1;
        }

        if ((++ym->count_noise) >= (int16_t)period_noise) {
            ym->count_noise = 0;
            ym->prescale_noise ^= 1u;
            if (!ym->prescale_noise) {
                ym->rng = (ym->rng >> 1) | ((uint32_t)(BIT(ym->rng, 0) ^ BIT(ym->rng, 3)) << 16);
            }
        }
    }

    for (int chan = 0; chan < 3; chan++) {
        uint8_t tone_enable = (uint8_t)BIT(ym->regs[AY_ENABLE], chan);
        uint8_t noise_enable = (uint8_t)BIT(ym->regs[AY_ENABLE], 3 + chan);
        uint8_t noise_out = (uint8_t)(ym->rng & 1u);
        ym->vol_enabled[chan] = (uint8_t)((ym->tone[chan].output | tone_enable) & (noise_out | noise_enable));
    }

    if (!ym->envelope.holding) {
        uint32_t period = ym->envelope.period;
        if ((++ym->envelope.count) >= (int32_t)period) {
            ym->envelope.count = 0;
            ym->envelope.step--;
            if (ym->envelope.step < 0) {
                if (ym->envelope.hold) {
                    if (ym->envelope.alternate) {
                        ym->envelope.attack ^= 0x1F;
                    }
                    ym->envelope.holding = 1;
                    ym->envelope.step = 0;
                } else {
                    if (ym->envelope.alternate && (ym->envelope.step & 0x20)) {
                        ym->envelope.attack ^= 0x1F;
                    }
                    ym->envelope.step &= 0x1F;
                }
            }
        }
    }
    ym->envelope.volume = (uint32_t)(ym->envelope.step ^ ym->envelope.attack);

    ym->last_output = ym2149_mix_current_output(ym, out_channels);

    return ym->last_output;
}

double ym2149_next_sample(YM2149Core *ym) {
    if (ym->render_mode == YM2149_RENDER_RESAMPLED) {
        double channels[3];
        return ym2149_next_sample_channels(ym, channels);
    }

    ym->step_accum += ym->chip_sample_rate;
    while (ym->step_accum >= ym->output_sample_rate) {
        ym->step_accum -= ym->output_sample_rate;
        (void)ym2149_step_chip_sample(ym, NULL);
    }

    if (ym->chip_sample_rate >= ym->output_sample_rate) {
        return ym->last_output;
    }

    /* For upsampling cases, keep chip state moving at least once per output sample. */
    (void)ym2149_step_chip_sample(ym, NULL);
    return ym->last_output;
}

double ym2149_next_sample_channels(YM2149Core *ym, double out_channels[3]) {
    if (ym->render_mode == YM2149_RENDER_RESAMPLED) {
        if (!ym->resample_seeded) {
            ym2149_seed_resampler(ym);
        }

        ym->resample_phase += (double)ym->chip_sample_rate / (double)ym->output_sample_rate;
        while (ym->resample_phase >= 1.0) {
            ym->resample_phase -= 1.0;
            ym->resample_prev_output = ym->resample_curr_output;
            for (int chan = 0; chan < 3; chan++) {
                ym->resample_prev_channels[chan] = ym->resample_curr_channels[chan];
            }
            ym->resample_curr_output = ym2149_step_chip_sample(ym, ym->resample_curr_channels);
        }

        if (out_channels) {
            for (int chan = 0; chan < 3; chan++) {
                out_channels[chan] =
                    ym->resample_prev_channels[chan] +
                    ((ym->resample_curr_channels[chan] - ym->resample_prev_channels[chan]) * ym->resample_phase);
            }
        }

        ym->last_output =
            ym->resample_prev_output +
            ((ym->resample_curr_output - ym->resample_prev_output) * ym->resample_phase);
        return ym->last_output;
    }

    if (out_channels) {
        out_channels[0] = 0.0;
        out_channels[1] = 0.0;
        out_channels[2] = 0.0;
    }

    ym->step_accum += ym->chip_sample_rate;
    while (ym->step_accum >= ym->output_sample_rate) {
        ym->step_accum -= ym->output_sample_rate;
        (void)ym2149_step_chip_sample(ym, out_channels);
    }

    if (ym->chip_sample_rate >= ym->output_sample_rate) {
        return ym->last_output;
    }

    /* For upsampling cases, keep chip state moving at least once per output sample. */
    (void)ym2149_step_chip_sample(ym, out_channels);
    return ym->last_output;
}
