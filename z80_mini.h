#ifndef Z80_MINI_H
#define Z80_MINI_H

#include <stdint.h>

typedef uint8_t (*z80_mem_read_fn)(void *user, uint16_t addr);
typedef void (*z80_mem_write_fn)(void *user, uint16_t addr, uint8_t value);
typedef uint8_t (*z80_io_read_fn)(void *user, uint8_t port);
typedef void (*z80_io_write_fn)(void *user, uint8_t port, uint8_t value);

typedef struct {
    uint8_t a;
    uint8_t f;
    uint8_t b;
    uint8_t c;
    uint8_t d;
    uint8_t e;
    uint8_t h;
    uint8_t l;
    uint16_t sp;
    uint16_t pc;

    z80_mem_read_fn mem_read;
    z80_mem_write_fn mem_write;
    z80_io_read_fn io_read;
    z80_io_write_fn io_write;
    void *user;

    uint8_t halted;
    uint8_t faulted;
    uint8_t fault_opcode;
    uint16_t fault_pc;
} Z80Mini;

void z80_mini_reset(Z80Mini *cpu);
int z80_mini_step(Z80Mini *cpu, uint32_t *cycles_out);

#endif
