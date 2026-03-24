#include "harmony32_board.h"

#include <stdlib.h>
#include <string.h>

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

int h32_board_step_music_tick(H32Board *b, uint32_t *tstates_out) {
    uint32_t t = 0;
    uint32_t i;

    if (!b) {
        return -1;
    }

    for (i = 0; i < 200000u; i++) {
        uint32_t cyc = 0;
        if (z80_mini_step(&b->cpu, &cyc) != 0) {
            return -1;
        }
        t += cyc;

        if (b->cpu.pc == 0x0039u) {
            if (tstates_out) {
                *tstates_out = t;
            }
            return 0;
        }

        if (b->cpu.pc == 0x0108u) {
            if (tstates_out) {
                *tstates_out = t;
            }
            return 1;
        }
    }

    return -1;
}
