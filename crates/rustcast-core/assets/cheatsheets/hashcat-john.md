# password cracking — hashcat & john

## hashcat modes (-m)
0     MD5
100   SHA1
1400  SHA256
1700  SHA512
1800  sha512crypt $6$ (Linux)
3200  bcrypt $2*$
1000  NTLM
5600  NetNTLMv2
22000 WPA-PBKDF2 / WPA2
13100 Kerberos TGS-REP (kerberoast)

## hashcat attacks (-a)
hashcat -m 0 -a 0 hashes.txt rockyou.txt          # straight (wordlist)
hashcat -m 0 -a 0 hashes.txt rockyou.txt -r best64.rule
hashcat -m 0 -a 3 hashes.txt ?l?l?l?l?l?d         # mask (brute)
hashcat -m 0 hashes.txt --show                    # show cracked
hashcat -m 0 hashes.txt --username                # hash file has user:hash

## mask charsets
?l lower  ?u upper  ?d digit  ?s special  ?a all  ?b raw byte

## john the ripper
john --wordlist=rockyou.txt hashes.txt
john --format=sha512crypt hashes.txt
john --show hashes.txt
john --incremental hashes.txt
# helper -> john format
unshadow /etc/passwd /etc/shadow > hashes.txt
zip2john file.zip > hash.txt
ssh2john id_rsa > hash.txt
