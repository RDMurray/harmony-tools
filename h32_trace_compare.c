#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "harmony32_board.h"
#include "z80_music_engine_equiv.h"

typedef struct {
    uint8_t input_port_1;
    uint32_t write_count;
    H32PortWrite writes[H32_STEP_TRACE_MAX_WRITES];
} TraceRecorder;

static uint8_t trace_in_port(void *user, uint8_t port) {
    TraceRecorder *rec = (TraceRecorder *)user;
    if (port == 0x01) {
        return rec->input_port_1;
    }
    return 0;
}

static void trace_out_port(void *user, uint8_t port, uint8_t value) {
    TraceRecorder *rec = (TraceRecorder *)user;
    if (rec->write_count >= H32_STEP_TRACE_MAX_WRITES) {
        return;
    }
    rec->writes[rec->write_count].port = port;
    rec->writes[rec->write_count].value = value;
    rec->write_count++;
}

static void usage(const char *argv0) {
    fprintf(stderr,
            "Usage: %s <rom.bin> [bank=0] [song=0] [speed=3] [drums_on=1] [ticks=64]\n",
            argv0);
}

int main(int argc, char **argv) {
    const char *rom_path;
    uint32_t bank = 0;
    int song = 0;
    int speed = 3;
    int drums_on = 1;
    int ticks = 64;
    FILE *rf = NULL;
    uint8_t *rom = NULL;
    long rom_size;
    size_t nread;
    H32Board board;
    Z80MusicCtx equiv;
    TraceRecorder board_rec;
    TraceRecorder equiv_rec;
    uint32_t max_tstate_delta = 0;
    int rc = 1;

    if (argc < 2) {
        usage(argv[0]);
        return 1;
    }

    rom_path = argv[1];
    if (argc >= 3) {
        bank = (uint32_t)strtoul(argv[2], NULL, 10);
    }
    if (argc >= 4) {
        song = atoi(argv[3]);
    }
    if (argc >= 5) {
        speed = atoi(argv[4]);
    }
    if (argc >= 6) {
        drums_on = atoi(argv[5]);
    }
    if (argc >= 7) {
        ticks = atoi(argv[6]);
    }

    if (song < 0 || song > 15 || speed < 0 || speed > 3 || (drums_on != 0 && drums_on != 1) || ticks <= 0) {
        usage(argv[0]);
        return 1;
    }

    memset(&board, 0, sizeof(board));
    memset(&equiv, 0, sizeof(equiv));
    memset(&board_rec, 0, sizeof(board_rec));
    memset(&equiv_rec, 0, sizeof(equiv_rec));

    board_rec.input_port_1 = (uint8_t)((song & 0x0F) | ((speed & 0x03) << 4) | (drums_on ? 0x00 : 0x40));
    equiv_rec.input_port_1 = board_rec.input_port_1;

    rf = fopen(rom_path, "rb");
    if (!rf) {
        fprintf(stderr, "Failed to open ROM '%s': %s\n", rom_path, strerror(errno));
        goto done;
    }
    if (fseek(rf, 0, SEEK_END) != 0) {
        fprintf(stderr, "Failed to seek ROM.\n");
        goto done;
    }
    rom_size = ftell(rf);
    if (rom_size <= 0) {
        fprintf(stderr, "Unexpected ROM size.\n");
        goto done;
    }
    if (fseek(rf, 0, SEEK_SET) != 0) {
        fprintf(stderr, "Failed to rewind ROM.\n");
        goto done;
    }

    rom = (uint8_t *)malloc((size_t)rom_size);
    if (!rom) {
        fprintf(stderr, "Failed to allocate ROM buffer.\n");
        goto done;
    }
    nread = fread(rom, 1, (size_t)rom_size, rf);
    if (nread != (size_t)rom_size) {
        fprintf(stderr, "Failed to read ROM.\n");
        goto done;
    }

    h32_board_init(&board, trace_in_port, trace_out_port, &board_rec);
    if (h32_board_load_rom(&board, rom, (uint32_t)rom_size) != 0) {
        fprintf(stderr, "Failed to load ROM into board.\n");
        goto done;
    }
    if (!h32_board_bank_available(&board, bank)) {
        fprintf(stderr, "Bank %u not available.\n", bank);
        goto done;
    }
    h32_board_set_bank_pin(&board, bank);
    if (h32_board_reset_song(&board) != 0) {
        fprintf(stderr, "Board reset failed.\n");
        goto done;
    }

    memset(&equiv.mem, 0, sizeof(equiv.mem));
    {
        uint32_t bank_base = bank * 0x4000u;
        uint32_t available = (uint32_t)rom_size > bank_base ? (uint32_t)rom_size - bank_base : 0;
        uint32_t copy_len = available > 0x4000u ? 0x4000u : available;
        memcpy(equiv.mem, &rom[bank_base], copy_len);
    }
    equiv.in_port = trace_in_port;
    equiv.out_port = trace_out_port;
    equiv.user = &equiv_rec;
    if (!z80_music_engine_init(&equiv)) {
        fprintf(stderr, "Equivalent reset failed.\n");
        goto done;
    }

    for (int tick = 0; tick < ticks; tick++) {
        H32StepTrace trace;
        uint32_t equiv_tstates = 0;
        int board_alive;
        int equiv_alive;

        memset(&trace, 0, sizeof(trace));
        board_rec.write_count = 0;
        equiv_rec.write_count = 0;

        board_alive = h32_board_step_music_tick_trace(&board, &trace);
        equiv_alive = z80_music_engine_step_timed(&equiv, &equiv_tstates) ? 1 : 0;

        if ((board_alive > 0) != (equiv_alive > 0)) {
            fprintf(stderr, "Tick %d: alive mismatch board=%d equiv=%d\n", tick, board_alive, equiv_alive);
            goto done;
        }
        if (trace.total_tstates != equiv_tstates) {
            uint32_t delta = trace.total_tstates > equiv_tstates
                                 ? (trace.total_tstates - equiv_tstates)
                                 : (equiv_tstates - trace.total_tstates);
            if (delta > max_tstate_delta) {
                max_tstate_delta = delta;
            }
        }
        if (trace.write_count != equiv_rec.write_count) {
            fprintf(stderr, "Tick %d: write count mismatch board=%u equiv=%u\n",
                    tick, trace.write_count, equiv_rec.write_count);
            goto done;
        }
        if (memcmp(&board.ram[0x4000], &equiv.mem[0x4000], 0x0D) != 0) {
            fprintf(stderr, "Tick %d: engine RAM mismatch.\n", tick);
            goto done;
        }
        for (uint32_t i = 0; i < trace.write_count; i++) {
            if (trace.writes[i].port != equiv_rec.writes[i].port ||
                trace.writes[i].value != equiv_rec.writes[i].value) {
                fprintf(stderr,
                        "Tick %d write %u: board=(port=0x%02X,value=0x%02X) equiv=(port=0x%02X,value=0x%02X)\n",
                        tick,
                        i,
                        trace.writes[i].port,
                        trace.writes[i].value,
                        equiv_rec.writes[i].port,
                        equiv_rec.writes[i].value);
                goto done;
            }
            if (trace.writes[i].tstate_end > trace.total_tstates) {
                fprintf(stderr, "Tick %d write %u: timestamp %u beyond tick total %u\n",
                        tick, i, trace.writes[i].tstate_end, trace.total_tstates);
                goto done;
            }
            if (i > 0 && trace.writes[i].tstate_end < trace.writes[i - 1u].tstate_end) {
                fprintf(stderr, "Tick %d write %u: timestamps not monotonic\n", tick, i);
                goto done;
            }
        }
        if (board_alive <= 0) {
            break;
        }
    }

    fprintf(stderr,
            "Trace comparison passed for %d ticks (write/RAM parity ok, max tstate delta vs legacy equiv=%u).\n",
            ticks,
            max_tstate_delta);
    rc = 0;

done:
    if (rf) {
        fclose(rf);
    }
    free(rom);
    h32_board_deinit(&board);
    return rc;
}
