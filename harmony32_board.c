#include "harmony32_board.h"

#include <stdlib.h>
#include <string.h>

static void board_trace_reset(H32Board *b, H32StepTrace *trace) {
    b->active_trace = trace;
    b->pending_write_count = 0;
    b->trace_overflow = 0;
    if (trace) {
        memset(trace, 0, sizeof(*trace));
    }
}

static void board_trace_record_pending(H32Board *b, uint8_t port, uint8_t value) {
    if (!b->active_trace) {
        return;
    }
    if (b->pending_write_count >= H32_STEP_TRACE_MAX_PENDING_WRITES) {
        b->trace_overflow = 1;
        return;
    }
    b->pending_writes[b->pending_write_count].port = port;
    b->pending_writes[b->pending_write_count].value = value;
    b->pending_write_count++;
}

static int board_trace_commit_instruction(H32Board *b, uint32_t tstate_end) {
    uint8_t i;
    if (!b->active_trace) {
        b->pending_write_count = 0;
        return 0;
    }
    if (b->trace_overflow) {
        return -1;
    }
    for (i = 0; i < b->pending_write_count; i++) {
        H32StepTrace *trace = b->active_trace;
        if (trace->write_count >= H32_STEP_TRACE_MAX_WRITES) {
            b->trace_overflow = 1;
            b->pending_write_count = 0;
            return -1;
        }
        trace->writes[trace->write_count].tstate_end = tstate_end;
        trace->writes[trace->write_count].port = b->pending_writes[i].port;
        trace->writes[trace->write_count].value = b->pending_writes[i].value;
        trace->write_count++;
    }
    b->pending_write_count = 0;
    return 0;
}

static uint8_t board_mem_read(void *user, uint16_t addr) {
    H32Board *b = (H32Board *)user;
    if (addr < 0x4000u) {
        uint32_t phys = b->bank_pin * 0x4000u + (uint32_t)addr;
        if (phys < b->rom_len) {
            return b->rom_image[phys];
        }
        return 0xFFu;
    }
    return b->ram[addr];
}

static void board_mem_write(void *user, uint16_t addr, uint8_t value) {
    H32Board *b = (H32Board *)user;
    if (addr < 0x4000u) {
        return;
    }
    b->ram[addr] = value;
}

static uint8_t board_io_read(void *user, uint8_t port) {
    H32Board *b = (H32Board *)user;
    if (!b->in_port) {
        return 0;
    }
    return b->in_port(b->io_user, port);
}

static void board_io_write(void *user, uint8_t port, uint8_t value) {
    H32Board *b = (H32Board *)user;
    board_trace_record_pending(b, port, value);
    if (b->out_port) {
        b->out_port(b->io_user, port, value);
    }
}

void h32_board_init(H32Board *b, h32_in_port_fn in_port, h32_out_port_fn out_port, void *io_user) {
    if (!b) {
        return;
    }
    memset(b, 0, sizeof(*b));
    b->in_port = in_port;
    b->out_port = out_port;
    b->io_user = io_user;
    b->cpu.mem_read = board_mem_read;
    b->cpu.mem_write = board_mem_write;
    b->cpu.io_read = board_io_read;
    b->cpu.io_write = board_io_write;
    b->cpu.user = b;
    z80_mini_reset(&b->cpu);
}

void h32_board_deinit(H32Board *b) {
    if (!b) {
        return;
    }
    free(b->rom_image);
    b->rom_image = NULL;
    b->rom_len = 0;
    b->bank_pin = 0;
}

int h32_board_load_rom(H32Board *b, const uint8_t *rom, uint32_t rom_len) {
    uint8_t *new_rom;
    if (!b || !rom || rom_len == 0) {
        return -1;
    }

    new_rom = (uint8_t *)malloc((size_t)rom_len);
    if (!new_rom) {
        return -1;
    }

    memcpy(new_rom, rom, rom_len);
    free(b->rom_image);
    b->rom_image = new_rom;
    b->rom_len = rom_len;
    if (!h32_board_bank_available(b, b->bank_pin)) {
        b->bank_pin = 0;
    }
    return 0;
}

void h32_board_set_bank_pin(H32Board *b, uint32_t bank_pin) {
    if (!b) {
        return;
    }
    b->bank_pin = bank_pin;
}

uint32_t h32_board_bank_count(const H32Board *b) {
    if (!b || b->rom_len == 0) {
        return 0;
    }
    return (b->rom_len + 0x3FFFu) >> 14;
}

int h32_board_bank_available(const H32Board *b, uint32_t bank_pin) {
    uint32_t count = h32_board_bank_count(b);
    if (count == 0) {
        return 0;
    }
    return bank_pin < count;
}

int h32_board_reset_cpu(H32Board *b) {
    if (!b || b->rom_len < 0x4000u || !h32_board_bank_available(b, b->bank_pin)) {
        return -1;
    }

    z80_mini_reset(&b->cpu);
    return 0;
}

int h32_board_reset_full(H32Board *b) {
    uint32_t i;
    if (!b || b->rom_len < 0x4000u || !h32_board_bank_available(b, b->bank_pin)) {
        return -1;
    }

    memset(b->ram, 0, sizeof(b->ram));
    z80_mini_reset(&b->cpu);

    for (i = 0; i < 200000u; i++) {
        uint32_t cyc;
        if (b->cpu.pc == 0x0108u) {
            return 0;
        }
        if (z80_mini_step(&b->cpu, &cyc) != 0) {
            return -1;
        }
        if (b->cpu.pc == 0x0000u && i > 4u) {
            return -1;
        }
    }

    return -1;
}

int h32_board_reset_song(H32Board *b) {
    return h32_board_reset_full(b);
}

int h32_board_step_music_tick_trace(H32Board *b, H32StepTrace *trace_out) {
    uint32_t t = 0;
    uint32_t i;

    if (!b) {
        return -1;
    }

    board_trace_reset(b, trace_out);
    for (i = 0; i < 200000u; i++) {
        uint32_t cyc = 0;
        if (z80_mini_step(&b->cpu, &cyc) != 0) {
            b->active_trace = NULL;
            return -1;
        }
        t += cyc;
        if (board_trace_commit_instruction(b, t) != 0) {
            b->active_trace = NULL;
            return -1;
        }

        if (b->cpu.pc == 0x0039u) {
            if (trace_out) {
                trace_out->total_tstates = t;
            }
            b->active_trace = NULL;
            return 0;
        }

        if (b->cpu.pc == 0x0108u) {
            if (trace_out) {
                trace_out->total_tstates = t;
            }
            b->active_trace = NULL;
            return 1;
        }
    }

    b->active_trace = NULL;
    return -1;
}

int h32_board_step_music_tick(H32Board *b, uint32_t *tstates_out) {
    int rc;
    H32StepTrace trace;

    rc = h32_board_step_music_tick_trace(b, &trace);
    if (tstates_out) {
        *tstates_out = trace.total_tstates;
    }
    return rc;
}
