# curl — HTTP from the command line

## Basics
curl https://target
curl -i URL                     # include response headers
curl -I URL                     # headers only (HEAD)
curl -v URL                     # verbose (request + response)
curl -s URL                     # silent (no progress)
curl -L URL                     # follow redirects
curl -o out.html URL            # save to file
curl -k URL                     # ignore TLS cert errors

## Methods & data
curl -X POST URL
curl -d "a=1&b=2" URL           # form POST (urlencoded)
curl -d @body.json URL          # body from file
curl -H "Content-Type: application/json" -d '{"a":1}' URL
curl -F "file=@shell.php" URL   # multipart upload

## Headers, auth, cookies
curl -H "Authorization: Bearer TOKEN" URL
curl -u user:pass URL           # basic auth
curl -b "SESSION=abc" URL       # send cookie
curl -c jar.txt -b jar.txt URL  # save & reuse cookies
curl -A "Mozilla/5.0" URL       # user agent
curl -x http://127.0.0.1:8080 URL   # through a proxy (Burp)

## Handy
curl -w "%{http_code} %{time_total}\n" -o /dev/null -s URL
