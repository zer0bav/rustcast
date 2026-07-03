# nmap — network scanning

## Host discovery
nmap -sn 10.0.0.0/24            # ping sweep, no port scan
nmap -Pn target                 # skip host discovery (assume up)
nmap -sL 10.0.0.0/24            # list targets without scanning

## Port scans
nmap target                     # top 1000 TCP ports
nmap -p- target                 # all 65535 TCP ports
nmap -p 22,80,443 target        # specific ports
nmap -F target                  # fast (top 100)
nmap -sU target                 # UDP scan
nmap -sS target                 # SYN (stealth, needs root)

## Service / OS detection
nmap -sV target                 # service/version detection
nmap -O target                  # OS fingerprint
nmap -A target                  # aggressive: -sV -O -sC --traceroute
nmap -sC target                 # default NSE scripts

## NSE scripts
nmap --script vuln target       # known-vuln checks
nmap --script http-enum target  # web content discovery
nmap --script "smb-*" target    # SMB script category

## Timing & output
nmap -T4 target                 # faster timing template
nmap -oA scan target            # save .nmap/.gp/.xml
nmap -oN out.txt target         # normal output to file
