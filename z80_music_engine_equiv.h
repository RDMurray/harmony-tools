#ifndef Z80_MUSIC_ENGINE_EQUIV_H
#define Z80_MUSIC_ENGINE_EQUIV_H

#include <stdbool.h>
#include <stdint.h>

typedef uint8_t (*in_port_fn)(void *user, uint8_t port);
typedef void (*out_port_fn)(void *user, uint8_t port, uint8_t value);

typedef struct {
    uint8_t mem[0x10000];
    in_port_fn in_port;
    out_port_fn out_port;
    void *user;
} Z80MusicCtx;

bool z80_music_engine_init(Z80MusicCtx *ctx);
bool z80_music_engine_step(Z80MusicCtx *ctx);
bool z80_music_engine_step_timed(Z80MusicCtx *ctx, uint32_t *tstates);
bool z80_music_engine_reset_and_run(Z80MusicCtx *ctx);

#endif
