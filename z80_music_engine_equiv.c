#include "z80_music_engine_equiv.h"
#include <stddef.h>

/*
 * C-equivalent model of Z80 code at ROM 0x0000-0x01DF from M27C256B@DIP28.BIN.
 *
 * This is a behavioral translation for analysis, not cycle-exact emulation.
 */

static inline uint8_t rd8(Z80MusicCtx *ctx, uint16_t addr) {
    return ctx->mem[addr];
}

static inline void wr8(Z80MusicCtx *ctx, uint16_t addr, uint8_t v) {
    ctx->mem[addr] = v;
}

static inline uint16_t rd16(Z80MusicCtx *ctx, uint16_t addr) {
    return (uint16_t)ctx->mem[addr] | ((uint16_t)ctx->mem[(uint16_t)(addr + 1)] << 8);
}

static inline void wr16(Z80MusicCtx *ctx, uint16_t addr, uint16_t v) {
    ctx->mem[addr] = (uint8_t)(v & 0xFFu);
    ctx->mem[(uint16_t)(addr + 1)] = (uint8_t)((v >> 8) & 0xFFu);
}

static inline uint8_t in_port(Z80MusicCtx *ctx, uint8_t port) {
    return ctx->in_port ? ctx->in_port(ctx->user, port) : 0;
}

static inline void out_port(Z80MusicCtx *ctx, uint8_t port, uint8_t value) {
    if (ctx->out_port) {
        ctx->out_port(ctx->user, port, value);
    }
}

/* 0x003B: returns false when end marker (0xFF) encountered. */
static bool sub_next_record_timed(Z80MusicCtx *ctx, uint32_t *cycles_out) {
    uint32_t cycles = 0;
    uint16_t hl = rd16(ctx, 0x400B);
    cycles += 16; /* LD HL,(nn) */
    uint8_t a = rd8(ctx, hl);
    cycles += 7; /* LD A,(HL) */
    cycles += 7; /* CP n */
    if (a == 0xFF) {
        cycles += 11; /* RET Z taken */
        if (cycles_out) {
            *cycles_out += cycles;
        }
        return false;
    }
    cycles += 5; /* RET Z not taken */

    uint16_t de = (uint16_t)(0x0200u | rd8(ctx, hl));
    cycles += 10; /* LD DE,nn */
    cycles += 7;  /* LD E,(HL) */
    wr8(ctx, 0x4008, rd8(ctx, de));
    cycles += 7;  /* LD A,(DE) */
    cycles += 13; /* LD (nn),A */
    de = (uint16_t)(de + 1);
    cycles += 6; /* INC DE */
    wr8(ctx, 0x4009, rd8(ctx, de));
    cycles += 7;  /* LD A,(DE) */
    cycles += 13; /* LD (nn),A */

    hl = (uint16_t)(hl + 1);
    cycles += 6; /* INC HL */
    a = (uint8_t)(rd8(ctx, hl) & 0x7Fu);
    cycles += 7; /* LD A,(HL) */
    cycles += 7; /* AND n */
    wr8(ctx, 0x4006, a);
    cycles += 13; /* LD (nn),A */

    /* Equivalent result of: A=(HL)&0x80; RLA; JR NC,+1; RRA */
    a = (uint8_t)(rd8(ctx, hl) & 0x80u);
    cycles += 7; /* LD A,(HL) */
    cycles += 7; /* AND n */
    cycles += 4; /* RLA */
    if (a == 0) {
        cycles += 12; /* JR NC taken */
    } else {
        cycles += 7; /* JR NC not taken */
        cycles += 4; /* RRA */
    }
    wr8(ctx, 0x4007, a);
    cycles += 13; /* LD (nn),A */

    hl = (uint16_t)(hl + 1);
    cycles += 6; /* INC HL */
    wr16(ctx, 0x400B, hl);
    cycles += 16; /* LD (nn),HL */
    cycles += 4;  /* INC A */
    cycles += 10; /* RET */
    if (cycles_out) {
        *cycles_out += cycles;
    }
    return true;
}

static bool sub_next_record(Z80MusicCtx *ctx) {
    return sub_next_record_timed(ctx, NULL);
}

/* 0x0066 */
static void sub_build0_timed(Z80MusicCtx *ctx, uint32_t *cycles_out) {
    uint32_t cycles = 243;
    uint8_t a;

    out_port(ctx, 0x03, 0x08);
    out_port(ctx, 0x02, 0x00);

    wr8(ctx, 0x4000, rd8(ctx, 0x4006));

    out_port(ctx, 0x03, 0x00);
    out_port(ctx, 0x02, rd8(ctx, 0x4008));

    out_port(ctx, 0x03, 0x01);
    out_port(ctx, 0x02, rd8(ctx, 0x4009));

    out_port(ctx, 0x03, 0x08);
    a = (rd8(ctx, 0x4007) == 0) ? 0x00 : 0x0F;
    out_port(ctx, 0x02, a);

    wr8(ctx, 0x4003, (uint8_t)(a << 2));
    if (cycles_out) {
        *cycles_out += cycles;
    }
}

static void sub_build0(Z80MusicCtx *ctx) {
    sub_build0_timed(ctx, NULL);
}

/* 0x009C */
static void sub_build1_timed(Z80MusicCtx *ctx, uint32_t *cycles_out) {
    uint32_t cycles = 243;
    uint8_t a;

    out_port(ctx, 0x03, 0x09);
    out_port(ctx, 0x02, 0x00);

    wr8(ctx, 0x4001, rd8(ctx, 0x4006));

    out_port(ctx, 0x03, 0x02);
    out_port(ctx, 0x02, rd8(ctx, 0x4008));

    out_port(ctx, 0x03, 0x03);
    out_port(ctx, 0x02, rd8(ctx, 0x4009));

    out_port(ctx, 0x03, 0x09);
    a = (rd8(ctx, 0x4007) == 0) ? 0x00 : 0x0C;
    out_port(ctx, 0x02, a);

    wr8(ctx, 0x4004, (uint8_t)(a << 2));
    if (cycles_out) {
        *cycles_out += cycles;
    }
}

static void sub_build1(Z80MusicCtx *ctx) {
    sub_build1_timed(ctx, NULL);
}

/* 0x00D2 */
static void sub_build2_timed(Z80MusicCtx *ctx, uint32_t *cycles_out) {
    uint32_t cycles = 243;
    uint8_t a;

    out_port(ctx, 0x03, 0x0A);
    out_port(ctx, 0x02, 0x00);

    wr8(ctx, 0x4002, rd8(ctx, 0x4006));

    out_port(ctx, 0x03, 0x04);
    out_port(ctx, 0x02, rd8(ctx, 0x4008));

    out_port(ctx, 0x03, 0x05);
    out_port(ctx, 0x02, rd8(ctx, 0x4009));

    out_port(ctx, 0x03, 0x0A);
    a = (rd8(ctx, 0x4007) == 0) ? 0x00 : 0x09;
    out_port(ctx, 0x02, a);

    wr8(ctx, 0x4005, (uint8_t)(a << 2));
    if (cycles_out) {
        *cycles_out += cycles;
    }
}

static void sub_build2(Z80MusicCtx *ctx) {
    sub_build2_timed(ctx, NULL);
}

/* 0x0108; single update tick, returns false when stream ended. */
static bool sub_process_timed(Z80MusicCtx *ctx, uint32_t *cycles_out) {
    uint32_t cycles = 0;
    uint16_t hl;
    uint8_t in1_a;
    uint8_t in1_b;
    uint8_t a;
    uint8_t b;
    uint8_t va;
    uint8_t vb;
    uint8_t vc;

    /* Channel refill checks at 0x0108 */
    if (rd8(ctx, 0x4000) == 0) {
        cycles += 10 + 7 + 7 + 7 + 17; /* LD HL / LD A,(HL) / CP / JR NZ not taken / CALL */
        if (!sub_next_record_timed(ctx, &cycles)) {
            cycles += 11; /* RET Z taken */
            if (cycles_out) {
                *cycles_out += cycles;
            }
            return false;
        }
        cycles += 11; /* RET Z not taken */
        cycles += 17; /* CALL */
        sub_build0_timed(ctx, &cycles);
    } else {
        cycles += 10 + 7 + 7 + 12; /* JR NZ taken */
    }

    if (rd8(ctx, 0x4001) == 0) {
        cycles += 10 + 7 + 7 + 7 + 17;
        if (!sub_next_record_timed(ctx, &cycles)) {
            cycles += 11;
            if (cycles_out) {
                *cycles_out += cycles;
            }
            return false;
        }
        cycles += 11;
        cycles += 17;
        sub_build1_timed(ctx, &cycles);
    } else {
        cycles += 10 + 7 + 7 + 12;
    }

    if (rd8(ctx, 0x4002) == 0) {
        cycles += 10 + 7 + 7 + 7 + 17;
        if (!sub_next_record_timed(ctx, &cycles)) {
            cycles += 11;
            if (cycles_out) {
                *cycles_out += cycles;
            }
            return false;
        }
        cycles += 11;
        cycles += 17;
        sub_build2_timed(ctx, &cycles);
    } else {
        cycles += 10 + 7 + 7 + 12;
    }

    cycles += 10; /* LD HL,0x4000 */
    hl = 0x4000;

    out_port(ctx, 0x03, 0x0E);
    cycles += 7 + 11; /* LD A,0x0E; OUT */
    in1_a = in_port(ctx, 0x01);
    cycles += 4 + 11; /* XOR A; IN */
    b = (uint8_t)((in1_a & 0x30u) | 0x0Fu);
    cycles += 7 + 7 + 4; /* AND; OR; LD B,A */

    in1_b = in_port(ctx, 0x01);
    cycles += 4 + 11; /* XOR A; IN */
    if ((in1_b & 0x40u) != 0) {
        a = 0x38u;
        cycles += 8 + 7 + 12; /* BIT; JR Z not taken; LD A; JR */
    } else {
        a = 0x08u;
        cycles += 8 + 12 + 7; /* BIT; JR Z taken; LD A */
    }
    wr8(ctx, 0x400A, a);
    cycles += 13;

    out_port(ctx, 0x03, 0x07);
    out_port(ctx, 0x02, rd8(ctx, 0x400A));
    cycles += 7 + 11 + 13 + 11;
    out_port(ctx, 0x03, 0x06);
    cycles += 7 + 11;
    a = (uint8_t)(b >> 3);
    cycles += 4 + 8 + 8;

    /* Delay/output loop from 0x0166-0x016F: 674*B - 5 T-states */
    {
        uint8_t loop_b = b;
        do {
            uint8_t c = 0x28;
            out_port(ctx, 0x02, a);
            do {
                c--;
            } while (c != 0);
            loop_b--;
            a >>= 1;
        } while (loop_b != 0);
    }
    cycles += (uint32_t)(674u * b - 5u);

    wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
    hl = (uint16_t)(hl + 1);
    wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
    hl = (uint16_t)(hl + 1);
    wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
    hl = (uint16_t)(hl + 1);
    cycles += 11 + 6 + 11 + 6 + 11 + 6;

    out_port(ctx, 0x03, 0x08);
    a = (uint8_t)(rd8(ctx, hl) >> 2);
    out_port(ctx, 0x02, a);
    va = rd8(ctx, hl);
    cycles += 7 + 11 + 7 + 8 + 8 + 11 + 7 + 7;
    if (va == 0) {
        cycles += 12; /* JR Z taken */
    } else {
        cycles += 7; /* JR Z not taken */
        cycles += 7; /* CP 0x24 */
        if (va < 0x24) {
            cycles += 12; /* JR C taken */
            wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
            cycles += 11;
        } else {
            cycles += 7; /* JR C not taken */
            cycles += 7; /* CP 0x30 */
            if (va < 0x30) {
                cycles += 12; /* JR C taken */
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                cycles += 11 + 11;
            } else {
                cycles += 7; /* JR C not taken */
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                cycles += 11 + 11 + 11 + 11;
            }
        }
    }

    hl = (uint16_t)(hl + 1);
    out_port(ctx, 0x03, 0x09);
    a = (uint8_t)(rd8(ctx, hl) >> 2);
    out_port(ctx, 0x02, a);
    vb = rd8(ctx, hl);
    cycles += 6 + 7 + 11 + 7 + 8 + 8 + 11 + 7 + 7;
    if (vb == 0) {
        cycles += 12; /* JR Z taken */
    } else {
        cycles += 7; /* JR Z not taken */
        cycles += 7; /* CP 0x18 */
        if (vb < 0x18) {
            cycles += 12; /* JR C taken */
            wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
            cycles += 11;
        } else {
            cycles += 7; /* JR C not taken */
            a = rd8(ctx, 0x400A);
            cycles += 13 + 7;
            if (a == 0x08 || vb >= 0x24) {
                /* Either JR Z taken, or JR Z not + CP/JR C not. Both execute two DECs. */
                if (a == 0x08) {
                    cycles += 12;
                } else {
                    cycles += 7 + 7 + 7;
                }
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                cycles += 11 + 11;
            } else {
                cycles += 7 + 7 + 12; /* JR Z not, CP 0x24, JR C taken */
                /* fall to L01B6: no L01B4/L01B5 DECs in this path */
            }
            wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
            wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
            cycles += 11 + 11;
        }
    }

    hl = (uint16_t)(hl + 1);
    out_port(ctx, 0x03, 0x0A);
    a = (uint8_t)(rd8(ctx, hl) >> 2);
    out_port(ctx, 0x02, a);
    vc = rd8(ctx, hl);
    cycles += 6 + 7 + 11 + 7 + 8 + 8 + 11 + 7 + 7;
    if (vc == 0) {
        cycles += 12; /* JR Z taken */
    } else {
        cycles += 7; /* JR Z not taken */
        cycles += 7; /* CP 0x0C */
        if (vc < 0x0C) {
            cycles += 12; /* JR C taken */
            wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
            cycles += 11;
        } else {
            cycles += 7; /* JR C not taken */
            a = rd8(ctx, 0x400A);
            cycles += 13 + 7;
            if (a == 0x08 || vc >= 0x18) {
                if (a == 0x08) {
                    cycles += 12;
                } else {
                    cycles += 7 + 7 + 7;
                }
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
                cycles += 11 + 11;
            } else {
                cycles += 7 + 7 + 12; /* JR Z not, CP 0x18, JR C taken */
                /* fall to L01DB: no L01D9/L01DA DECs in this path */
            }
            wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
            wr8(ctx, hl, (uint8_t)(rd8(ctx, hl) - 1));
            cycles += 11 + 11;
        }
    }

    cycles += 10; /* JP 0x0108 */
    if (cycles_out) {
        *cycles_out += cycles;
    }
    return true;
}

/* 0x0108; returns false when stream ended. */
static bool sub_process(Z80MusicCtx *ctx) {
    return sub_process_timed(ctx, NULL);
}

/*
 * Entry behavior from 0x0000.
 * Returns false when track stream reaches 0xFF marker.
 */
bool z80_music_engine_init(Z80MusicCtx *ctx) {
    uint8_t a;
    uint16_t hl;

    out_port(ctx, 0x03, 0x07);
    out_port(ctx, 0x02, 0x38);
    wr8(ctx, 0x400A, 0x38);

    out_port(ctx, 0x03, 0x0E);
    a = (uint8_t)(in_port(ctx, 0x01) & 0x0Fu);
    a = (uint8_t)(a << 1);
    hl = (uint16_t)(0x01E0u + a);

    wr16(ctx, 0x400B, rd16(ctx, hl));

    if (!sub_next_record(ctx)) {
        return false;
    }
    sub_build0(ctx);

    if (!sub_next_record(ctx)) {
        return false;
    }
    sub_build1(ctx);

    if (!sub_next_record(ctx)) {
        return false;
    }
    sub_build2(ctx);

    return true;
}

bool z80_music_engine_step(Z80MusicCtx *ctx) {
    return sub_process(ctx);
}

bool z80_music_engine_step_timed(Z80MusicCtx *ctx, uint32_t *tstates) {
    if (tstates) {
        *tstates = 0;
    }
    return sub_process_timed(ctx, tstates);
}

bool z80_music_engine_reset_and_run(Z80MusicCtx *ctx) {
    if (!z80_music_engine_init(ctx)) {
        return false;
    }
    return z80_music_engine_step(ctx);
}
