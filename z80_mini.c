#include "z80_mini.h"

#define FLAG_C 0x01u
#define FLAG_N 0x02u
#define FLAG_PV 0x04u
#define FLAG_H 0x10u
#define FLAG_Z 0x40u
#define FLAG_S 0x80u

static uint8_t parity_even(uint8_t v) {
    v ^= (uint8_t)(v >> 4);
    v ^= (uint8_t)(v >> 2);
    v ^= (uint8_t)(v >> 1);
    return (uint8_t)((~v) & 1u);
}

static uint8_t flags_szp(uint8_t v) {
    uint8_t f = 0;
    if (v & 0x80u) {
        f |= FLAG_S;
    }
    if (v == 0) {
        f |= FLAG_Z;
    }
    if (parity_even(v)) {
        f |= FLAG_PV;
    }
    return f;
}

static uint8_t rd8(Z80Mini *cpu, uint16_t addr) {
    if (!cpu->mem_read) {
        return 0xFFu;
    }
    return cpu->mem_read(cpu->user, addr);
}

static void wr8(Z80Mini *cpu, uint16_t addr, uint8_t v) {
    if (cpu->mem_write) {
        cpu->mem_write(cpu->user, addr, v);
    }
}

static uint8_t in8(Z80Mini *cpu, uint8_t port) {
    if (!cpu->io_read) {
        return 0;
    }
    return cpu->io_read(cpu->user, port);
}

static void out8(Z80Mini *cpu, uint8_t port, uint8_t value) {
    if (cpu->io_write) {
        cpu->io_write(cpu->user, port, value);
    }
}

static uint16_t hl(const Z80Mini *cpu) {
    return (uint16_t)(((uint16_t)cpu->h << 8) | cpu->l);
}

static void set_hl(Z80Mini *cpu, uint16_t v) {
    cpu->h = (uint8_t)(v >> 8);
    cpu->l = (uint8_t)(v & 0xFFu);
}

static uint16_t de(const Z80Mini *cpu) {
    return (uint16_t)(((uint16_t)cpu->d << 8) | cpu->e);
}

static void set_de(Z80Mini *cpu, uint16_t v) {
    cpu->d = (uint8_t)(v >> 8);
    cpu->e = (uint8_t)(v & 0xFFu);
}

static uint16_t bc(const Z80Mini *cpu) {
    return (uint16_t)(((uint16_t)cpu->b << 8) | cpu->c);
}

static void set_bc(Z80Mini *cpu, uint16_t v) {
    cpu->b = (uint8_t)(v >> 8);
    cpu->c = (uint8_t)(v & 0xFFu);
}

static uint8_t fetch8(Z80Mini *cpu) {
    uint8_t v = rd8(cpu, cpu->pc);
    cpu->pc = (uint16_t)(cpu->pc + 1u);
    return v;
}

static uint16_t fetch16(Z80Mini *cpu) {
    uint8_t lo = fetch8(cpu);
    uint8_t hi = fetch8(cpu);
    return (uint16_t)(((uint16_t)hi << 8) | lo);
}

static void push16(Z80Mini *cpu, uint16_t v) {
    cpu->sp = (uint16_t)(cpu->sp - 1u);
    wr8(cpu, cpu->sp, (uint8_t)(v >> 8));
    cpu->sp = (uint16_t)(cpu->sp - 1u);
    wr8(cpu, cpu->sp, (uint8_t)(v & 0xFFu));
}

static uint16_t pop16(Z80Mini *cpu) {
    uint8_t lo = rd8(cpu, cpu->sp);
    cpu->sp = (uint16_t)(cpu->sp + 1u);
    uint8_t hi = rd8(cpu, cpu->sp);
    cpu->sp = (uint16_t)(cpu->sp + 1u);
    return (uint16_t)(((uint16_t)hi << 8) | lo);
}

static void do_add_a_n(Z80Mini *cpu, uint8_t n) {
    uint16_t res = (uint16_t)cpu->a + (uint16_t)n;
    uint8_t out = (uint8_t)res;
    uint8_t f = 0;
    if (out & 0x80u) {
        f |= FLAG_S;
    }
    if (out == 0) {
        f |= FLAG_Z;
    }
    if (((cpu->a & 0x0Fu) + (n & 0x0Fu)) & 0x10u) {
        f |= FLAG_H;
    }
    if (((~(cpu->a ^ n) & (cpu->a ^ out)) & 0x80u) != 0) {
        f |= FLAG_PV;
    }
    if (res & 0x100u) {
        f |= FLAG_C;
    }
    cpu->a = out;
    cpu->f = f;
}

static void do_cp_n(Z80Mini *cpu, uint8_t n) {
    uint16_t res = (uint16_t)cpu->a - (uint16_t)n;
    uint8_t out = (uint8_t)res;
    uint8_t f = FLAG_N;
    if (out & 0x80u) {
        f |= FLAG_S;
    }
    if (out == 0) {
        f |= FLAG_Z;
    }
    if (((cpu->a ^ n ^ out) & 0x10u) != 0) {
        f |= FLAG_H;
    }
    if ((((cpu->a ^ n) & (cpu->a ^ out)) & 0x80u) != 0) {
        f |= FLAG_PV;
    }
    if (res & 0x100u) {
        f |= FLAG_C;
    }
    cpu->f = f;
}

static void do_inc_a(Z80Mini *cpu) {
    uint8_t in = cpu->a;
    uint8_t out = (uint8_t)(in + 1u);
    uint8_t f = cpu->f & FLAG_C;
    if (out & 0x80u) {
        f |= FLAG_S;
    }
    if (out == 0) {
        f |= FLAG_Z;
    }
    if (((in & 0x0Fu) + 1u) & 0x10u) {
        f |= FLAG_H;
    }
    if (in == 0x7Fu) {
        f |= FLAG_PV;
    }
    cpu->a = out;
    cpu->f = f;
}

static uint8_t do_dec8(Z80Mini *cpu, uint8_t in) {
    uint8_t out = (uint8_t)(in - 1u);
    uint8_t f = cpu->f & FLAG_C;
    f |= FLAG_N;
    if (out & 0x80u) {
        f |= FLAG_S;
    }
    if (out == 0) {
        f |= FLAG_Z;
    }
    if ((in & 0x0Fu) == 0x00u) {
        f |= FLAG_H;
    }
    if (in == 0x80u) {
        f |= FLAG_PV;
    }
    cpu->f = f;
    return out;
}

void z80_mini_reset(Z80Mini *cpu) {
    if (!cpu) {
        return;
    }
    cpu->a = 0;
    cpu->f = 0;
    cpu->b = 0;
    cpu->c = 0;
    cpu->d = 0;
    cpu->e = 0;
    cpu->h = 0;
    cpu->l = 0;
    cpu->sp = 0;
    cpu->pc = 0;
    cpu->halted = 0;
    cpu->faulted = 0;
    cpu->fault_opcode = 0;
    cpu->fault_pc = 0;
}

static int fault(Z80Mini *cpu, uint8_t op, uint16_t pc) {
    cpu->faulted = 1;
    cpu->fault_opcode = op;
    cpu->fault_pc = pc;
    return -1;
}

int z80_mini_step(Z80Mini *cpu, uint32_t *cycles_out) {
    uint16_t pc_before;
    uint8_t op;
    uint32_t cyc;

    if (!cpu || cpu->faulted) {
        return -1;
    }

    pc_before = cpu->pc;
    op = fetch8(cpu);
    cyc = 0;

    switch (op) {
        case 0x0D:
            cpu->c = do_dec8(cpu, cpu->c);
            cyc = 4;
            break;
        case 0x0E:
            cpu->c = fetch8(cpu);
            cyc = 7;
            break;
        case 0x10: {
            int8_t rel = (int8_t)fetch8(cpu);
            cpu->b = (uint8_t)(cpu->b - 1u);
            if (cpu->b != 0) {
                cpu->pc = (uint16_t)(cpu->pc + rel);
                cyc = 13;
            } else {
                cyc = 8;
            }
            break;
        }
        case 0x11:
            set_de(cpu, fetch16(cpu));
            cyc = 10;
            break;
        case 0x13:
            set_de(cpu, (uint16_t)(de(cpu) + 1u));
            cyc = 6;
            break;
        case 0x17: {
            uint8_t old_a = cpu->a;
            uint8_t carry_in = (uint8_t)(cpu->f & FLAG_C ? 1u : 0u);
            cpu->a = (uint8_t)((old_a << 1) | carry_in);
            cpu->f = (uint8_t)((cpu->f & (FLAG_S | FLAG_Z | FLAG_PV)) | ((old_a & 0x80u) ? FLAG_C : 0u));
            cyc = 4;
            break;
        }
        case 0x18: {
            int8_t rel = (int8_t)fetch8(cpu);
            cpu->pc = (uint16_t)(cpu->pc + rel);
            cyc = 12;
            break;
        }
        case 0x1A:
            cpu->a = rd8(cpu, de(cpu));
            cyc = 7;
            break;
        case 0x1F: {
            uint8_t old_a = cpu->a;
            uint8_t carry_in = (uint8_t)(cpu->f & FLAG_C ? 1u : 0u);
            cpu->a = (uint8_t)((old_a >> 1) | (carry_in << 7));
            cpu->f = (uint8_t)((cpu->f & (FLAG_S | FLAG_Z | FLAG_PV)) | ((old_a & 0x01u) ? FLAG_C : 0u));
            cyc = 4;
            break;
        }
        case 0x20: {
            int8_t rel = (int8_t)fetch8(cpu);
            if ((cpu->f & FLAG_Z) == 0) {
                cpu->pc = (uint16_t)(cpu->pc + rel);
                cyc = 12;
            } else {
                cyc = 7;
            }
            break;
        }
        case 0x21:
            set_hl(cpu, fetch16(cpu));
            cyc = 10;
            break;
        case 0x22: {
            uint16_t addr = fetch16(cpu);
            uint16_t v = hl(cpu);
            wr8(cpu, addr, (uint8_t)(v & 0xFFu));
            wr8(cpu, (uint16_t)(addr + 1u), (uint8_t)(v >> 8));
            cyc = 16;
            break;
        }
        case 0x23:
            set_hl(cpu, (uint16_t)(hl(cpu) + 1u));
            cyc = 6;
            break;
        case 0x26:
            cpu->h = fetch8(cpu);
            cyc = 7;
            break;
        case 0x28: {
            int8_t rel = (int8_t)fetch8(cpu);
            if ((cpu->f & FLAG_Z) != 0) {
                cpu->pc = (uint16_t)(cpu->pc + rel);
                cyc = 12;
            } else {
                cyc = 7;
            }
            break;
        }
        case 0x2A: {
            uint16_t addr = fetch16(cpu);
            uint16_t v = (uint16_t)rd8(cpu, addr) | ((uint16_t)rd8(cpu, (uint16_t)(addr + 1u)) << 8);
            set_hl(cpu, v);
            cyc = 16;
            break;
        }
        case 0x30: {
            int8_t rel = (int8_t)fetch8(cpu);
            if ((cpu->f & FLAG_C) == 0) {
                cpu->pc = (uint16_t)(cpu->pc + rel);
                cyc = 12;
            } else {
                cyc = 7;
            }
            break;
        }
        case 0x31:
            cpu->sp = fetch16(cpu);
            cyc = 10;
            break;
        case 0x32: {
            uint16_t addr = fetch16(cpu);
            wr8(cpu, addr, cpu->a);
            cyc = 13;
            break;
        }
        case 0x35: {
            uint16_t addr = hl(cpu);
            uint8_t v = rd8(cpu, addr);
            v = do_dec8(cpu, v);
            wr8(cpu, addr, v);
            cyc = 11;
            break;
        }
        case 0x38: {
            int8_t rel = (int8_t)fetch8(cpu);
            if ((cpu->f & FLAG_C) != 0) {
                cpu->pc = (uint16_t)(cpu->pc + rel);
                cyc = 12;
            } else {
                cyc = 7;
            }
            break;
        }
        case 0x3A: {
            uint16_t addr = fetch16(cpu);
            cpu->a = rd8(cpu, addr);
            cyc = 13;
            break;
        }
        case 0x3C:
            do_inc_a(cpu);
            cyc = 4;
            break;
        case 0x3E:
            cpu->a = fetch8(cpu);
            cyc = 7;
            break;
        case 0x47:
            cpu->b = cpu->a;
            cyc = 4;
            break;
        case 0x5E:
            cpu->e = rd8(cpu, hl(cpu));
            cyc = 7;
            break;
        case 0x6F:
            cpu->l = cpu->a;
            cyc = 4;
            break;
        case 0x78:
            cpu->a = cpu->b;
            cyc = 4;
            break;
        case 0x7E:
            cpu->a = rd8(cpu, hl(cpu));
            cyc = 7;
            break;
        case 0xAF:
            cpu->a = 0;
            cpu->f = FLAG_Z;
            cyc = 4;
            break;
        case 0xC3:
            cpu->pc = fetch16(cpu);
            cyc = 10;
            break;
        case 0xC6:
            do_add_a_n(cpu, fetch8(cpu));
            cyc = 7;
            break;
        case 0xC8:
            if ((cpu->f & FLAG_Z) != 0) {
                cpu->pc = pop16(cpu);
                cyc = 11;
            } else {
                cyc = 5;
            }
            break;
        case 0xC9:
            cpu->pc = pop16(cpu);
            cyc = 10;
            break;
        case 0xCD: {
            uint16_t target = fetch16(cpu);
            push16(cpu, cpu->pc);
            cpu->pc = target;
            cyc = 17;
            break;
        }
        case 0xD3: {
            uint8_t port = fetch8(cpu);
            out8(cpu, port, cpu->a);
            cyc = 11;
            break;
        }
        case 0xDB: {
            uint8_t port = fetch8(cpu);
            cpu->a = in8(cpu, port);
            cyc = 11;
            break;
        }
        case 0xE6: {
            uint8_t n = fetch8(cpu);
            cpu->a = (uint8_t)(cpu->a & n);
            cpu->f = (uint8_t)(flags_szp(cpu->a) | FLAG_H);
            cyc = 7;
            break;
        }
        case 0xED: {
            uint8_t ext = fetch8(cpu);
            if (ext == 0xA0) {
                uint8_t v = rd8(cpu, hl(cpu));
                wr8(cpu, de(cpu), v);
                set_hl(cpu, (uint16_t)(hl(cpu) + 1u));
                set_de(cpu, (uint16_t)(de(cpu) + 1u));
                set_bc(cpu, (uint16_t)(bc(cpu) - 1u));
                cyc = 16;
            } else {
                return fault(cpu, ext, pc_before);
            }
            break;
        }
        case 0xF6: {
            uint8_t n = fetch8(cpu);
            cpu->a = (uint8_t)(cpu->a | n);
            cpu->f = flags_szp(cpu->a);
            cyc = 7;
            break;
        }
        case 0xFE:
            do_cp_n(cpu, fetch8(cpu));
            cyc = 7;
            break;
        case 0xCB: {
            uint8_t cb = fetch8(cpu);
            if (cb == 0x27) {
                uint8_t old = cpu->a;
                cpu->a = (uint8_t)(old << 1);
                cpu->f = flags_szp(cpu->a);
                if (old & 0x80u) {
                    cpu->f |= FLAG_C;
                }
                cyc = 8;
            } else if (cb == 0x3F) {
                uint8_t old = cpu->a;
                cpu->a = (uint8_t)(old >> 1);
                cpu->f = flags_szp(cpu->a);
                if (old & 0x01u) {
                    cpu->f |= FLAG_C;
                }
                cyc = 8;
            } else if (cb == 0x77) {
                uint8_t bit_set = (uint8_t)((cpu->a >> 6) & 1u);
                uint8_t keep_c = (uint8_t)(cpu->f & FLAG_C);
                cpu->f = (uint8_t)(keep_c | FLAG_H);
                if (!bit_set) {
                    cpu->f |= FLAG_Z;
                    cpu->f |= FLAG_PV;
                }
                if (cpu->a & 0x80u) {
                    cpu->f |= FLAG_S;
                }
                cyc = 8;
            } else {
                return fault(cpu, cb, pc_before);
            }
            break;
        }
        default:
            return fault(cpu, op, pc_before);
    }

    if (cycles_out) {
        *cycles_out = cyc;
    }
    return 0;
}
