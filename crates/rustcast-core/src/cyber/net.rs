//! Network / OSINT / forensics helpers: CIDR math, epoch conversion,
//! defang/refang, and link-out URL builders.

use chrono::TimeZone;
use ipnet::IpNet;

/// CIDR breakdown for something like `10.0.0.0/24`.
pub fn cidr_info(input: &str) -> Option<Vec<(String, String)>> {
    let net: IpNet = input.trim().parse().ok()?;
    let hosts = match net {
        IpNet::V4(n) => {
            let prefix = n.prefix_len();
            if prefix >= 31 {
                2u128.pow((32 - prefix) as u32)
            } else {
                2u128.pow((32 - prefix) as u32).saturating_sub(2)
            }
        }
        IpNet::V6(n) => {
            let bits = 128 - n.prefix_len() as u32;
            if bits >= 127 {
                u128::MAX
            } else {
                2u128.pow(bits)
            }
        }
    };
    Some(vec![
        ("network".into(), net.network().to_string()),
        ("broadcast".into(), net.broadcast().to_string()),
        ("netmask".into(), net.netmask().to_string()),
        ("prefix".into(), format!("/{}", net.prefix_len())),
        ("usable hosts".into(), hosts.to_string()),
    ])
}

/// If input is an integer epoch, render human time (auto-detect s / ms).
pub fn epoch_to_human(input: &str) -> Option<String> {
    let n: i64 = input.trim().parse().ok()?;
    // Heuristic: 13-digit values are milliseconds.
    let (secs, nanos) = if input.trim().len() >= 12 {
        (n / 1000, ((n % 1000) * 1_000_000) as u32)
    } else {
        (n, 0)
    };
    let dt = chrono::Utc.timestamp_opt(secs, nanos).single()?;
    Some(format!(
        "{}  ({} local)",
        dt.format("%Y-%m-%d %H:%M:%S UTC"),
        dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S")
    ))
}

/// Defang an indicator so it can't be accidentally clicked.
pub fn defang(s: &str) -> String {
    s.replace("http", "hxxp")
        .replace('.', "[.]")
        .replace("://", "[://]")
        .replace('@', "[@]")
}

/// Reverse of [`defang`].
pub fn refang(s: &str) -> String {
    s.replace("[.]", ".")
        .replace("[://]", "://")
        .replace("[@]", "@")
        .replace("hxxp", "http")
}

/// OSINT / lookup link-outs for an indicator (IP, domain, hash, CVE).
pub fn links(indicator: &str) -> Vec<(&'static str, String)> {
    let ind = indicator.trim();
    let enc = urlencoding::encode(ind);
    let mut out = Vec::new();
    let upper = ind.to_uppercase();
    if upper.starts_with("CVE-") {
        out.push(("NVD", format!("https://nvd.nist.gov/vuln/detail/{upper}")));
        out.push(("MITRE", format!("https://cve.mitre.org/cgi-bin/cvename.cgi?name={upper}")));
        return out;
    }
    out.push(("VirusTotal", format!("https://www.virustotal.com/gui/search/{enc}")));
    out.push(("Shodan", format!("https://www.shodan.io/search?query={enc}")));
    out.push(("AbuseIPDB", format!("https://www.abuseipdb.com/check/{enc}")));
    out.push(("ipinfo", format!("https://ipinfo.io/{enc}")));
    out.push(("Google", format!("https://www.google.com/search?q={enc}")));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_v4() {
        let info = cidr_info("10.0.0.0/24").unwrap();
        let map: std::collections::HashMap<_, _> = info.into_iter().collect();
        assert_eq!(map["network"], "10.0.0.0");
        assert_eq!(map["broadcast"], "10.0.0.255");
        assert_eq!(map["usable hosts"], "254");
    }

    #[test]
    fn epoch() {
        assert!(epoch_to_human("1516239022").unwrap().contains("2018-01-18"));
    }

    #[test]
    fn defang_roundtrip() {
        let d = defang("http://1.2.3.4");
        assert_eq!(d, "hxxp[://]1[.]2[.]3[.]4");
        assert_eq!(refang(&d), "http://1.2.3.4");
    }

    #[test]
    fn cve_links() {
        let l = links("CVE-2021-44228");
        assert!(l.iter().any(|(_, u)| u.contains("nvd.nist.gov")));
    }
}
