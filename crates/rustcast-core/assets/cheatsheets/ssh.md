# ssh — remote access & tunneling

## Connect
ssh user@host
ssh -p 2222 user@host           # custom port
ssh -i key.pem user@host        # identity file
ssh -v user@host                # verbose (debug auth)

## Keys
ssh-keygen -t ed25519 -C "me"   # generate key
ssh-copy-id user@host           # install pubkey
cat ~/.ssh/id_ed25519.pub       # your public key

## Port forwarding (tunnels)
ssh -L 8080:localhost:80 user@host      # local: your :8080 -> host:80
ssh -R 9000:localhost:3000 user@host    # remote: host:9000 -> your :3000
ssh -D 1080 user@host                   # dynamic SOCKS proxy on :1080
ssh -f -N -L 8080:db:5432 user@host     # background, no shell

## Files
scp file user@host:/path
scp -r dir user@host:/path
scp user@host:/path/file .
rsync -avz dir/ user@host:/path/

## Config (~/.ssh/config)
Host box
    HostName 10.0.0.5
    User root
    Port 2222
    IdentityFile ~/.ssh/box
