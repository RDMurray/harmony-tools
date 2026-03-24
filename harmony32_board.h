#ifndef HARMONY32_BOARD_H
#define HARMONY32_BOARD_H

#include <stdint.h>

#include "z80_mini.h"

typedef uint8_t (*h32_in_port_fn)(void *user, uint8_t port);
typedef void (*h32_out_port_fn)(void *user, uint8_t port, uint8_t value);

typedef struct {
    Z80Mini cpu;
    uint8_t *rom_image;
    uint32_t rom_len;
    uint32_t bank_pin;
    uint8_t ram[0x10000];

    h32_in_port_fn in_port;
    h32_out_port_fn out_port;
    void *io_user;
} H32Board;

void h32_board_init(H32Board *b, h32_in_port_fn in_port, h32_out_port_fn out_port, void *io_user);
void h32_board_deinit(H32Board *b);
int h32_board_load_rom(H32Board *b, const uint8_t *rom, uint32_t rom_len);
void h32_board_set_bank_pin(H32Board *b, uint32_t bank_pin);
uint32_t h32_board_bank_count(const H32Board *b);
int h32_board_bank_available(const H32Board *b, uint32_t bank_pin);

int h32_board_reset_song(H32Board *b);
int h32_board_reset_cpu(H32Board *b);
int h32_board_reset_full(H32Board *b);
int h32_board_step_music_tick(H32Board *b, uint32_t *tstates_out);

#endif
