# gdb — debugger (with pwndbg/gef in mind)

## Start
gdb ./bin                       # load binary
gdb ./bin core                  # load with core dump
gdb -p PID                      # attach to process
run [args]        / r           # start
start                           # break at main then run

## Breakpoints
break main        / b main
break *0x401234                 # at address
break file.c:42
tbreak main                     # one-shot
info breakpoints  / i b
delete N                        # remove breakpoint
continue          / c

## Stepping
next / n                        # step over
step / s                        # step into
stepi / si                      # one instruction
finish                          # run until return

## Inspect
info registers    / i r
x/16xw $rsp                     # examine 16 words hex at rsp
x/s 0x601000                    # string at address
print $rax        / p $rax
disassemble main  / disas
backtrace         / bt

## pwndbg / gef extras
checksec                        # binary protections
vmmap                           # memory map
cyclic 200                      # De Bruijn pattern
cyclic -l 0x6161616c            # find offset
