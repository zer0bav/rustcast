# web fuzzing — ffuf & gobuster

## ffuf — directories
ffuf -u https://target/FUZZ -w wordlist.txt
ffuf -u https://target/FUZZ -w list.txt -mc 200,301,302   # match codes
ffuf -u https://target/FUZZ -w list.txt -fc 404           # filter codes
ffuf -u https://target/FUZZ -w list.txt -fs 0             # filter size
ffuf -u https://target/FUZZ -w list.txt -e .php,.txt,.bak # extensions
ffuf -u https://target/FUZZ -w list.txt -recursion

## ffuf — vhosts / subdomains
ffuf -u https://target -H "Host: FUZZ.target" -w subs.txt -fs 0

## ffuf — parameters / POST
ffuf -u "https://target/api?FUZZ=1" -w params.txt
ffuf -u https://target/login -X POST -d "user=admin&pass=FUZZ" -w list.txt

## gobuster
gobuster dir -u https://target -w list.txt
gobuster dir -u https://target -w list.txt -x php,txt,html
gobuster dns -d target.com -w subs.txt
gobuster vhost -u https://target -w subs.txt

## common wordlists (seclists)
/usr/share/seclists/Discovery/Web-Content/directory-list-2.3-medium.txt
/usr/share/seclists/Discovery/DNS/subdomains-top1million-5000.txt
