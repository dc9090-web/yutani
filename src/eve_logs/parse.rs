//! One log line → one event. Pure; every pattern here is a verbatim line
//! from a real log (see the spec's table of colour tags).

use super::{EveTime, EventKind};

const INCOMING_DAMAGE: &str = "<color=0xffcc0000><b>";
const INCOMING_ENERGY: &str = "<color=0xffe57f7f><b>";
const SCRAM: &str = "<b>Warp scramble attempt</b>";
const POINT: &str = "<b>Warp disruption attempt</b>";
/// The dim suffix that follows a name in the combat markup.
const DIM: &str = "<color=0x77ffffff>";

/// Drop every `<…>` tag and decode the three entities the client writes.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            c => out.push(c),
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// `[ YYYY.MM.DD HH:MM:SS ] rest`, with any leading BOM or whitespace.
pub fn split_stamp(line: &str) -> Option<(EveTime, &str)> {
    let line = line.trim_start_matches(['\u{feff}', ' ', '\t']);
    let body = line.strip_prefix("[ ")?;
    let stamp = body.get(..19)?;
    let rest = body.get(19..)?.strip_prefix(" ] ")?;
    Some((EveTime::parse(stamp)?, rest))
}

/// The name in `s`: from its first `<b>` up to the dim suffix, markup
/// stripped (a player's name carries nested tags for ship and corp).
/// Also returns what follows the name.
fn name_before_dim(s: &str) -> Option<(String, &str)> {
    let start = s.find("<b>")? + 3;
    let end = s[start..].find(DIM)?;
    let name = strip_markup(&s[start..start + end]).trim().to_string();
    Some((name, &s[start + end..]))
}

pub fn game_line(line: &str) -> Option<(EveTime, EventKind)> {
    let (at, rest) = split_stamp(line)?;
    let body = rest.strip_prefix('(')?;
    let close = body.find(") ")?;
    let text = &body[close + 2..];
    let kind = if let Some(t) = text.strip_prefix("Jumping from ") {
        EventKind::System(t.rsplit(" to ").next()?.trim().to_string())
    } else if let Some(t) = text.strip_prefix("Undocking from ") {
        let sys = t.trim_end().strip_suffix(" solar system.")?.rsplit(" to ").next()?;
        EventKind::Undocked(sys.trim().to_string())
    } else if text.starts_with("Docking operation already in progress") {
        EventKind::Docked
    } else if let Some(t) = text.strip_prefix(INCOMING_DAMAGE) {
        let amount: u32 = t[..t.find("</b>")?].parse().ok()?;
        let after = &t[t.find("from</font>")? + 11..];
        let (from, tail) = name_before_dim(after)?;
        let tail = strip_markup(tail);
        let parts: Vec<&str> = tail.split(" - ").map(str::trim).filter(|p| !p.is_empty()).collect();
        let weapon = (parts.len() >= 2).then(|| parts[0].to_string());
        EventKind::Damage { amount, from, weapon }
    } else if let Some(t) = text.strip_prefix(INCOMING_ENERGY) {
        let num = &t[..t.find(" GJ")?];
        let gj: u32 = num.trim_start_matches(['-', '+']).parse().ok()?;
        let (from, _) = name_before_dim(&t[t.find("</font>")?..])?;
        EventKind::Neuted { gj, from }
    } else if text.contains(SCRAM) || text.contains(POINT) {
        if !text.trim_end().ends_with("you!") {
            return None;
        }
        let after = &text[text.find("from</font>")? + 11..];
        let end = after.find(DIM)?;
        let from = strip_markup(&after[..end]).trim().to_string();
        if text.contains(SCRAM) { EventKind::Scrambled(from) } else { EventKind::Disrupted(from) }
    } else if let Some(t) = text.strip_prefix("You're jammed by ") {
        EventKind::Jammed(t.split(" - ").next()?.trim().to_string())
    } else {
        return None;
    };
    Some((at, kind))
}

pub fn chat_line(line: &str) -> Option<(EveTime, EventKind)> {
    let (at, rest) = split_stamp(line)?;
    let sys = rest.strip_prefix("EVE System > Channel changed to Local : ")?;
    Some((at, EventKind::System(sys.trim().to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eve_logs::EventKind::*;

    fn kind(line: &str) -> Option<EventKind> {
        game_line(line).map(|(_, k)| k)
    }

    #[test]
    fn strip_markup_flattens_names() {
        assert_eq!(
            strip_markup("<font size=14><color=0xFFFFFF40>(<u><b>Tholos</b></u>)</color></font>Ishtar Komarovo [DDOG.]&lt;MSF.&gt;"),
            "(Tholos)Ishtar Komarovo [DDOG.]<MSF.>"
        );
        assert_eq!(strip_markup("plain"), "plain");
    }

    #[test]
    fn stamp_is_split_off() {
        let (at, rest) = split_stamp("[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora").unwrap();
        assert_eq!(at, EveTime::parse("2026.11.07 09:16:47").unwrap());
        assert_eq!(rest, "(None) Jumping from Messoya to Akora");
        assert_eq!(split_stamp("  Listener: TrilliumONE"), None);
        assert!(split_stamp("\u{feff}[ 2026.11.07 09:20:47 ] EVE System > x").is_some());
    }

    #[test]
    fn navigation() {
        assert_eq!(kind("[ 2026.11.07 09:16:47 ] (None) Jumping from Messoya to Akora"), Some(System("Akora".into())));
        assert_eq!(
            kind("[ 2026.11.07 03:40:12 ] (None) Undocking from Jita IV - Moon 4 - Caldari Navy Assembly Plant to Jita solar system."),
            Some(Undocked("Jita".into()))
        );
        assert_eq!(
            kind("[ 2026.07.31 04:11:07 ] (notify) Docking operation already in progress.<br>Estimated time left: 9 seconds."),
            Some(Docked)
        );
        assert_eq!(kind("[ 2026.11.07 09:15:36 ] (notify) Autopilot warping to Akora"), None);
    }

    #[test]
    fn incoming_damage_only() {
        let hit = "[ 2026.11.07 09:09:53 ] (combat) <color=0xffcc0000><b>50</b> <color=0x77ffffff><font size=10>from</font> <b><color=0xffffffff>Pith Extinguisher</b><font size=10><color=0x77ffffff> - Scourge Torpedo - Hits";
        assert_eq!(kind(hit), Some(Damage { amount: 50, from: "Pith Extinguisher".into(), weapon: Some("Scourge Torpedo".into()) }));
        let bare = "[ 2026.07.25 02:08:48 ] (combat) <color=0xffcc0000><b>28</b> <color=0x77ffffff><font size=10>from</font> <b><color=0xffffffff>Tower Sentry Guristas I</b><font size=10><color=0x77ffffff> - Penetrates";
        assert_eq!(kind(bare), Some(Damage { amount: 28, from: "Tower Sentry Guristas I".into(), weapon: None }));
        let out = "[ 2026.07.31 08:37:45 ] (combat) <color=0xff00ffff><b>31</b> <color=0x77ffffff><font size=10>to</font> <b><color=0xffffffff>Guristas Demolisher</b><font size=10><color=0x77ffffff> - Caldari Navy Scourge Heavy Assault Missile - Hits";
        assert_eq!(kind(out), None);
        assert_eq!(kind("[ 2026.11.07 09:09:52 ] (combat) Pithum Abolisher misses you completely"), None);
    }

    #[test]
    fn tackle_on_you_only() {
        let scram = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp scramble attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b>Emergent Preserver</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>you!";
        assert_eq!(kind(scram), Some(Scrambled("Emergent Preserver".into())));
        let point = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp disruption attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b>Blood Raiders Cruor</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>you!";
        assert_eq!(kind(point), Some(Disrupted("Blood Raiders Cruor".into())));
        let other = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp disruption attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b>Renouncer Harbinger</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>Republic Hurricane";
        assert_eq!(kind(other), None);
        let player = "[ 2026.07.26 05:31:11 ] (combat) <color=0xffffffff><b>Warp scramble attempt</b> <color=0x77ffffff><font size=10>from</font> <color=0xffffffff><b><font size=14><color=0xFFFFFF40>(<u><b>Tholos</b></u>)</color></font>Ishtar Komarovo [DDOG.]&lt;MSF.&gt;</b> <color=0x77ffffff><font size=10>to <b><color=0xffffffff></font>you!";
        assert_eq!(kind(player), Some(Scrambled("(Tholos)Ishtar Komarovo [DDOG.]<MSF.>".into())));
    }

    #[test]
    fn jam_and_energy() {
        assert_eq!(
            kind("[ 2026.08.13 04:39:26 ] (combat) You're jammed by Guristas Saboteur - Guristas Saboteur"),
            Some(Jammed("Guristas Saboteur".into()))
        );
        let neut = "[ 2026.08.13 04:39:26 ] (combat) <color=0xffe57f7f><b>30 GJ</b><color=0x77ffffff><font size=10> energy neutralized </font><b><color=0xffffffff>Emergent Watchman</b><color=0x77ffffff><font size=10> - Emergent Watchman</font>";
        assert_eq!(kind(neut), Some(Neuted { gj: 30, from: "Emergent Watchman".into() }));
        let nos = "[ 2026.08.13 08:24:29 ] (combat) <color=0xffe57f7f><b>-11 GJ</b><color=0x77ffffff><font size=10> energy drained to </font><b><color=0xffffffff>Blood Raiders Cruor</b><color=0x77ffffff><font size=10> - ♦ Cruor</font>";
        assert_eq!(kind(nos), Some(Neuted { gj: 11, from: "Blood Raiders Cruor".into() }));
        let mine = "[ 2026.08.01 01:56:53 ] (combat) <color=0xff7fffff><b>+0 GJ</b><color=0x77ffffff><font size=10> energy drained from </font><b><color=0xffffffff>Pithi Arrogator</b><color=0x77ffffff><font size=10> - Small Energy Nosferatu I</font>";
        assert_eq!(kind(mine), None);
    }

    #[test]
    fn chat_system_change() {
        let line = "\u{feff}[ 2026.11.07 09:20:47 ] EVE System > Channel changed to Local : Korsiki";
        assert_eq!(chat_line(line).map(|(_, k)| k), Some(System("Korsiki".into())));
        assert_eq!(chat_line("\u{feff}[ 2026.11.07 09:20:47 ] Some Pilot > o7"), None);
        assert_eq!(chat_line("          Listener:        TrilliumONE"), None);
    }
}
