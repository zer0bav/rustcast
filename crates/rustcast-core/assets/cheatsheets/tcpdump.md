# tcpdump — packet capture

## Basics
tcpdump -i eth0                 # capture on interface
tcpdump -i any                  # all interfaces
tcpdump -D                      # list interfaces
tcpdump -c 100                  # stop after 100 packets
tcpdump -n                      # no name resolution
tcpdump -nn                     # no name or port resolution

## Write / read
tcpdump -i eth0 -w cap.pcap     # write raw capture
tcpdump -r cap.pcap             # read a capture file
tcpdump -w cap.pcap -C 100      # rotate every 100 MB

## Filters
tcpdump host 10.0.0.5
tcpdump src 10.0.0.5
tcpdump dst 10.0.0.5
tcpdump port 443
tcpdump portrange 1-1024
tcpdump tcp
tcpdump udp
tcpdump 'tcp[tcpflags] & tcp-syn != 0'   # SYN packets
tcpdump 'port 80 and host 10.0.0.5'

## Inspect payloads
tcpdump -A                      # print ASCII payload
tcpdump -X                      # hex + ASCII
tcpdump -vvv                    # very verbose
