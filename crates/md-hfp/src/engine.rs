//! The hands-free (HF) side of the HFP AT dialogue, with no I/O ("sans-I/O").
//!
//! Feed it the bytes the phone (the audio gateway, AG) sends with
//! [`Engine::receive`]; take what to write back and what happened from
//! [`Engine::poll`]. Commands go one at a time: HFP doesn't allow a new one
//! before the last one's `OK` / `ERROR`, so they queue here.
//!
//! The service-level connection (SLC) is set up as HFP 1.7 §4.2 says:
//!
//! ```text
//! AT+BRSF=<HF features>        +BRSF: <AG features>   OK
//! AT+BAC=1,2                   OK          (only when both negotiate codecs)
//! AT+CIND=?                    +CIND: ("call",(0,1)),...   OK
//! AT+CIND?                     +CIND: 0,0,1,4,0,5,0   OK
//! AT+CMER=3,0,0,1              OK          (indicator events on)
//! AT+CHLD=?                    +CHLD: (0,1,2,3)   OK  (both do three-way)
//! ```
//!
//! then, once up: caller id (`AT+CLIP=1`), call waiting (`AT+CCWA=1`),
//! extended errors (`AT+CMEE=1`) and the current call list (`AT+CLCC`).

use std::collections::VecDeque;
use std::fmt;

/// HF supported features (`AT+BRSF`), HFP 1.7 table 4.2.
pub mod hf {
    pub const EC_NR: u32 = 1 << 0;
    pub const THREE_WAY: u32 = 1 << 1;
    pub const CALLER_ID: u32 = 1 << 2;
    pub const VOICE_RECOGNITION: u32 = 1 << 3;
    pub const REMOTE_VOLUME: u32 = 1 << 4;
    pub const ENHANCED_STATUS: u32 = 1 << 5;
    pub const ENHANCED_CONTROL: u32 = 1 << 6;
    pub const CODEC_NEGOTIATION: u32 = 1 << 7;
    pub const HF_INDICATORS: u32 = 1 << 8;
    pub const ESCO_S4: u32 = 1 << 9;
}

/// AG supported features (`+BRSF`), HFP 1.7 table 4.3.
pub mod ag {
    pub const THREE_WAY: u32 = 1 << 0;
    pub const EC_NR: u32 = 1 << 1;
    pub const VOICE_RECOGNITION: u32 = 1 << 2;
    pub const IN_BAND_RING: u32 = 1 << 3;
    pub const VOICE_TAG: u32 = 1 << 4;
    pub const REJECT: u32 = 1 << 5;
    pub const ENHANCED_STATUS: u32 = 1 << 6;
    pub const ENHANCED_CONTROL: u32 = 1 << 7;
    pub const EXTENDED_ERRORS: u32 = 1 << 8;
    pub const CODEC_NEGOTIATION: u32 = 1 << 9;
    pub const HF_INDICATORS: u32 = 1 << 10;
    pub const ESCO_S4: u32 = 1 << 11;
}

/// Narrowband voice: 8 kHz, 16-bit PCM over the air (the radio does CVSD).
pub const CODEC_CVSD: u8 = 1;
/// Wideband voice: 16 kHz mSBC frames (needs an SBC codec on this side).
pub const CODEC_MSBC: u8 = 2;

/// What this hands-free offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// [`hf`] feature bits for `AT+BRSF`.
    pub features: u32,
    /// Codec ids for `AT+BAC` (only sent with [`hf::CODEC_NEGOTIATION`]).
    pub codecs: Vec<u8>,
    /// Ask the phone to turn its echo cancelling off (`AT+NREC=0`): only
    /// right when this side cancels echo itself.
    pub disable_ag_nrec: bool,
}

impl Config {
    /// Caller id, three-way calling (call waiting), remote volume, the
    /// current call list; CVSD audio only (no codec negotiation).
    pub fn cvsd_only() -> Self {
        Config {
            features: hf::THREE_WAY | hf::CALLER_ID | hf::REMOTE_VOLUME | hf::ENHANCED_STATUS,
            codecs: vec![CODEC_CVSD],
            disable_ag_nrec: false,
        }
    }

    /// As [`Config::cvsd_only`], plus wideband (mSBC) by codec negotiation.
    pub fn with_msbc() -> Self {
        let c = Self::cvsd_only();
        Config {
            features: c.features | hf::CODEC_NEGOTIATION,
            codecs: vec![CODEC_CVSD, CODEC_MSBC],
            ..c
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::cvsd_only()
    }
}

/// A line of the phone's current call list (`+CLCC`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedCall {
    pub index: u8,
    /// Mobile terminated (someone called the phone).
    pub incoming: bool,
    /// 0 active, 1 held, 2 dialling, 3 alerting, 4 incoming, 5 waiting.
    pub status: u8,
    pub multiparty: bool,
    pub number: Option<String>,
    pub name: Option<String>,
}

/// What the phone said happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The service-level connection is up: calls can be seen and made.
    Ready {
        ag_features: u32,
    },
    /// Setting the link up failed (the phone refused a required step).
    SetupFailed(String),
    /// The "service" indicator: the phone has a network.
    Service(bool),
    /// The "call" indicator: a call is in progress.
    Call(bool),
    /// The "callsetup" indicator: 0 none, 1 incoming, 2 dialling, 3 alerting.
    Setup(u8),
    /// The "callheld" indicator: 0 none, 1 held + active, 2 held only.
    Held(u8),
    /// Signal 0-5.
    Signal(u8),
    Roaming(bool),
    /// Phone battery 0-5.
    Battery(u8),
    /// `RING`: an incoming call alerts (repeats every few seconds).
    Ring,
    /// `+CLIP`: the incoming caller.
    Caller {
        number: String,
        name: Option<String>,
    },
    /// `+CCWA`: a second call waiting during this one.
    Waiting {
        number: String,
        name: Option<String>,
    },
    /// A line of the call list asked for with `AT+CLCC`.
    ListedCall(ListedCall),
    /// The call list is complete (possibly empty).
    ListEnd,
    /// The phone picked the audio codec for the next audio link.
    Codec(u8),
    /// Remote volume: speaker / microphone gain 0-15.
    SpeakerGain(u8),
    MicGain(u8),
    /// The phone refused a command (`ERROR`, `+CME ERROR`, `NO CARRIER`...).
    Refused {
        command: String,
    },
}

/// Where the service-level connection setup stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Features,
    Codecs,
    IndicatorNames,
    IndicatorValues,
    Reporting,
    Holding,
    Ready,
    Failed,
}

/// A command couldn't be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The service-level connection isn't up (yet).
    NotReady,
    /// The phone or this side doesn't do that.
    Unsupported(&'static str),
    BadArgument(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotReady => write!(f, "the phone isn't connected for calls yet"),
            Error::Unsupported(what) => write!(f, "the phone doesn't support {what}"),
            Error::BadArgument(a) => write!(f, "can't send {a:?} to the phone"),
        }
    }
}

impl std::error::Error for Error {}

/// Something to do after feeding the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Write these bytes to the RFCOMM channel (`\r` included).
    Send(String),
    Event(Event),
}

/// The longest line kept while waiting for its end (a stuck peer).
const MAX_LINE: usize = 4096;

/// The HF's AT state machine.
#[derive(Debug)]
pub struct Engine {
    cfg: Config,
    stage: Stage,
    ag_features: u32,
    /// Indicator names by position (from `AT+CIND=?`), lowercased.
    indicators: Vec<String>,
    /// Three-way calling actions the phone offers (`AT+CHLD=?`).
    hold_actions: Vec<String>,
    line: Vec<u8>,
    queue: VecDeque<String>,
    in_flight: Option<String>,
    out: VecDeque<Output>,
}

impl Engine {
    pub fn new(cfg: Config) -> Self {
        Engine {
            cfg,
            stage: Stage::Idle,
            ag_features: 0,
            indicators: Vec::new(),
            hold_actions: Vec::new(),
            line: Vec::new(),
            queue: VecDeque::new(),
            in_flight: None,
            out: VecDeque::new(),
        }
    }

    /// Begin the service-level connection (once, right after the RFCOMM
    /// channel opens).
    pub fn start(&mut self) {
        if self.stage == Stage::Idle {
            self.stage = Stage::Features;
            self.command(format!("AT+BRSF={}", self.cfg.features));
        }
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn is_ready(&self) -> bool {
        self.stage == Stage::Ready
    }

    pub fn ag_features(&self) -> u32 {
        self.ag_features
    }

    /// The three-way calling actions the phone offers (`0`, `1`, `2x`...).
    pub fn hold_actions(&self) -> &[String] {
        &self.hold_actions
    }

    /// Both sides negotiate the audio codec (`AT+BAC` / `+BCS` / `AT+BCC`).
    pub fn codec_negotiation(&self) -> bool {
        self.cfg.features & hf::CODEC_NEGOTIATION != 0
            && self.ag_features & ag::CODEC_NEGOTIATION != 0
    }

    fn both(&self, hf_bit: u32, ag_bit: u32) -> bool {
        self.cfg.features & hf_bit != 0 && self.ag_features & ag_bit != 0
    }

    /// The next thing to do, if any.
    pub fn poll(&mut self) -> Option<Output> {
        self.out.pop_front()
    }

    /// Everything to do now.
    pub fn drain(&mut self) -> Vec<Output> {
        self.out.drain(..).collect()
    }

    /// Bytes from the phone (any split: partial and multiple lines are fine).
    pub fn receive(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b == b'\r' || b == b'\n' {
                if !self.line.is_empty() {
                    let line = String::from_utf8_lossy(&self.line).trim().to_string();
                    self.line.clear();
                    if !line.is_empty() {
                        self.line_in(&line);
                    }
                }
            } else if self.line.len() < MAX_LINE {
                self.line.push(b);
            }
        }
    }

    // -- commands ---------------------------------------------------------

    fn user(&mut self, cmd: String) -> Result<(), Error> {
        if self.stage != Stage::Ready {
            return Err(Error::NotReady);
        }
        self.command(cmd);
        Ok(())
    }

    /// Answer the ringing call.
    pub fn answer(&mut self) -> Result<(), Error> {
        self.user("ATA".into())
    }

    /// Decline the ringing call, or end the current one.
    pub fn hang_up(&mut self) -> Result<(), Error> {
        self.user("AT+CHUP".into())
    }

    /// Call `number` (digits, `*`, `#`, a leading `+`).
    pub fn dial(&mut self, number: &str) -> Result<(), Error> {
        let ok = !number.is_empty()
            && number.len() <= 32
            && number
                .char_indices()
                .all(|(i, c)| c.is_ascii_digit() || c == '*' || c == '#' || (c == '+' && i == 0));
        if !ok {
            return Err(Error::BadArgument(number.into()));
        }
        self.user(format!("ATD{number};"))
    }

    /// Call the last number again.
    pub fn redial(&mut self) -> Result<(), Error> {
        self.user("AT+BLDN".into())
    }

    /// One keypad tone: 0-9, `*`, `#`, A-D.
    pub fn dtmf(&mut self, c: char) -> Result<(), Error> {
        if !matches!(c, '0'..='9' | '*' | '#' | 'A'..='D') {
            return Err(Error::BadArgument(c.to_string()));
        }
        self.user(format!("AT+VTS={c}"))
    }

    /// Ask for the current call list (`+CLCC` lines, then [`Event::ListEnd`]).
    pub fn list_calls(&mut self) -> Result<(), Error> {
        self.user("AT+CLCC".into())
    }

    /// A three-way calling action (`AT+CHLD=<action>`): `0` release held /
    /// reject waiting, `1` end active and accept the other, `2` swap...
    pub fn hold(&mut self, action: &str) -> Result<(), Error> {
        let ok = !action.is_empty()
            && action.len() <= 3
            && action.chars().all(|c| c.is_ascii_digit() || c == 'x');
        if !ok {
            return Err(Error::BadArgument(action.into()));
        }
        if self.ag_features & ag::THREE_WAY == 0 {
            return Err(Error::Unsupported("three-way calling"));
        }
        self.user(format!("AT+CHLD={action}"))
    }

    /// Tell the phone this side's speaker gain (0-15).
    pub fn speaker_gain(&mut self, gain: u8) -> Result<(), Error> {
        self.user(format!("AT+VGS={}", gain.min(15)))
    }

    /// Tell the phone this side's microphone gain (0-15).
    pub fn mic_gain(&mut self, gain: u8) -> Result<(), Error> {
        self.user(format!("AT+VGM={}", gain.min(15)))
    }

    /// Ask the phone to open the audio link (`AT+BCC`). Only with codec
    /// negotiation; without it the hands-free opens the SCO link itself.
    pub fn audio_connect(&mut self) -> Result<(), Error> {
        if !self.codec_negotiation() {
            return Err(Error::Unsupported("codec negotiation"));
        }
        self.user("AT+BCC".into())
    }

    fn command(&mut self, cmd: String) {
        self.queue.push_back(cmd);
        self.pump();
    }

    fn pump(&mut self) {
        if self.in_flight.is_none() {
            if let Some(cmd) = self.queue.pop_front() {
                self.out.push_back(Output::Send(format!("{cmd}\r")));
                self.in_flight = Some(cmd);
            }
        }
    }

    fn emit(&mut self, e: Event) {
        self.out.push_back(Output::Event(e));
    }

    // -- what the phone says ------------------------------------------------

    fn line_in(&mut self, line: &str) {
        match line {
            "OK" => return self.finish(true),
            "ERROR" | "NO CARRIER" | "BUSY" | "NO ANSWER" | "DELAYED" | "BLACKLISTED" => {
                if self.in_flight.is_some() {
                    self.finish(false);
                }
                return;
            }
            "RING" => return self.emit(Event::Ring),
            _ => {}
        }
        let Some((name, rest)) = split_result(line) else {
            return;
        };
        let in_flight = self.in_flight.clone().unwrap_or_default();
        match name.as_str() {
            "+CME ERROR" => {
                if self.in_flight.is_some() {
                    self.finish(false);
                }
            }
            "+BRSF" => self.ag_features = rest.trim().parse().unwrap_or(0),
            "+CIND" if in_flight == "AT+CIND=?" => self.indicators = quoted(rest),
            "+CIND" if in_flight == "AT+CIND?" => {
                for (i, v) in numbers(rest).into_iter().enumerate() {
                    self.indicator(i, v);
                }
            }
            "+CHLD" => {
                self.hold_actions = rest
                    .trim()
                    .trim_start_matches('(')
                    .trim_end_matches(')')
                    .split(',')
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect();
            }
            "+CIEV" => {
                let n = numbers(rest);
                if let (Some(&i), Some(&v)) = (n.first(), n.get(1)) {
                    if i >= 1 {
                        self.indicator(i as usize - 1, v);
                    }
                }
            }
            "+CLIP" => {
                let f = fields(rest);
                self.emit(Event::Caller {
                    number: field(&f, 0).unwrap_or_default(),
                    name: field(&f, 4),
                });
            }
            "+CCWA" => {
                let f = fields(rest);
                self.emit(Event::Waiting {
                    number: field(&f, 0).unwrap_or_default(),
                    name: field(&f, 3),
                });
            }
            "+CLCC" => {
                let f = fields(rest);
                let num = |i: usize| field(&f, i).and_then(|v| v.parse::<u8>().ok());
                if let (Some(index), Some(dir), Some(status)) = (num(0), num(1), num(2)) {
                    self.emit(Event::ListedCall(ListedCall {
                        index,
                        incoming: dir == 1,
                        status,
                        multiparty: num(4) == Some(1),
                        number: field(&f, 5),
                        name: field(&f, 7),
                    }));
                }
            }
            "+BCS" => {
                let id = rest.trim().parse::<u8>().unwrap_or(0);
                if self.cfg.codecs.contains(&id) {
                    self.command(format!("AT+BCS={id}"));
                    self.emit(Event::Codec(id));
                } else {
                    // Not one of ours: say again which ones are.
                    self.command(self.bac());
                }
            }
            "+VGS" => self.emit(Event::SpeakerGain(gain(rest))),
            "+VGM" => self.emit(Event::MicGain(gain(rest))),
            _ => {}
        }
    }

    fn bac(&self) -> String {
        let list: Vec<String> = self.cfg.codecs.iter().map(|c| c.to_string()).collect();
        format!("AT+BAC={}", list.join(","))
    }

    /// Indicator `i` (0-based, in `AT+CIND=?` order) is now `v`.
    fn indicator(&mut self, i: usize, v: u32) {
        let Some(name) = self.indicators.get(i) else {
            return;
        };
        let small = v.min(u8::MAX as u32) as u8;
        let e = match name.as_str() {
            "call" => Event::Call(v != 0),
            "callsetup" | "call_setup" => Event::Setup(small.min(3)),
            "callheld" | "call_held" => Event::Held(small.min(2)),
            "signal" => Event::Signal(small.min(5)),
            "battchg" => Event::Battery(small.min(5)),
            "service" => Event::Service(v != 0),
            "roam" => Event::Roaming(v != 0),
            _ => return,
        };
        self.emit(e);
    }

    /// The in-flight command finished.
    fn finish(&mut self, ok: bool) {
        let Some(cmd) = self.in_flight.take() else {
            return;
        };
        match self.stage {
            Stage::Features => {
                if !ok {
                    // A pre-1.0 phone: no feature exchange.
                    self.ag_features = 0;
                }
                if self.codec_negotiation() {
                    self.stage = Stage::Codecs;
                    self.command(self.bac());
                } else {
                    self.indicator_names();
                }
            }
            Stage::Codecs => self.indicator_names(),
            Stage::IndicatorNames => {
                if ok && !self.indicators.is_empty() {
                    self.stage = Stage::IndicatorValues;
                    self.command("AT+CIND?".into());
                } else {
                    self.fail("the phone didn't list its call indicators");
                }
            }
            Stage::IndicatorValues => {
                if ok {
                    self.stage = Stage::Reporting;
                    self.command("AT+CMER=3,0,0,1".into());
                } else {
                    self.fail("the phone didn't give its call indicators");
                }
            }
            Stage::Reporting => {
                if !ok {
                    self.fail("the phone wouldn't report call events");
                } else if self.both(hf::THREE_WAY, ag::THREE_WAY) {
                    self.stage = Stage::Holding;
                    self.command("AT+CHLD=?".into());
                } else {
                    self.ready();
                }
            }
            Stage::Holding => self.ready(),
            Stage::Ready => {
                if cmd == "AT+CLCC" && ok {
                    self.emit(Event::ListEnd);
                } else if !ok {
                    self.emit(Event::Refused { command: cmd });
                }
            }
            Stage::Idle | Stage::Failed => {}
        }
        self.pump();
    }

    fn indicator_names(&mut self) {
        self.stage = Stage::IndicatorNames;
        self.command("AT+CIND=?".into());
    }

    fn fail(&mut self, why: &str) {
        self.stage = Stage::Failed;
        self.queue.clear();
        self.emit(Event::SetupFailed(why.into()));
    }

    fn ready(&mut self) {
        self.stage = Stage::Ready;
        let ag_features = self.ag_features;
        self.emit(Event::Ready { ag_features });
        self.command("AT+CLIP=1".into());
        if self.both(hf::THREE_WAY, ag::THREE_WAY) {
            self.command("AT+CCWA=1".into());
        }
        if self.cfg.disable_ag_nrec && ag_features & ag::EC_NR != 0 {
            self.command("AT+NREC=0".into());
        }
        if ag_features & ag::EXTENDED_ERRORS != 0 {
            self.command("AT+CMEE=1".into());
        }
        if self.both(hf::ENHANCED_STATUS, ag::ENHANCED_STATUS) {
            self.command("AT+CLCC".into());
        }
    }
}

/// `+NAME: rest` (or `+NAME=rest`, as some phones send `+VGS=7`).
fn split_result(line: &str) -> Option<(String, &str)> {
    if !line.starts_with('+') {
        return None;
    }
    let at = line.find([':', '='])?;
    Some((line[..at].trim().to_ascii_uppercase(), &line[at + 1..]))
}

/// Comma-separated values, quotes respected and removed, trimmed.
fn fields(rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in rest.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

/// Field `i`, when there and not empty.
fn field(f: &[String], i: usize) -> Option<String> {
    f.get(i).filter(|v| !v.is_empty()).cloned()
}

/// The quoted strings in order (indicator names in `+CIND: ("call",(0,1)),...`).
fn quoted(rest: &str) -> Vec<String> {
    rest.split('"')
        .skip(1)
        .step_by(2)
        .map(|s| s.trim().to_ascii_lowercase())
        .collect()
}

/// Comma-separated numbers (anything unreadable counts as 0).
fn numbers(rest: &str) -> Vec<u32> {
    rest.split(',')
        .map(|n| n.trim().parse().unwrap_or(0))
        .collect()
}

fn gain(rest: &str) -> u8 {
    rest.trim().parse::<u8>().unwrap_or(0).min(15)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Android's indicator order (not the spec's example order).
    const ANDROID_CIND: &str = "\r\n+CIND: (\"call\",(0,1)),(\"callsetup\",(0-3)),(\"service\",(0-1)),(\"signal\",(0-5)),(\"roam\",(0,1)),(\"battchg\",(0-5)),(\"callheld\",(0-2))\r\n\r\nOK\r\n";

    fn sent(out: &[Output]) -> Vec<String> {
        out.iter()
            .filter_map(|o| match o {
                Output::Send(s) => Some(s.clone()),
                Output::Event(_) => None,
            })
            .collect()
    }

    fn events(out: &[Output]) -> Vec<Event> {
        out.iter()
            .filter_map(|o| match o {
                Output::Event(e) => Some(e.clone()),
                Output::Send(_) => None,
            })
            .collect()
    }

    /// An Android 13 phone (Pixel): AG features 3943 = three-way, EC/NR,
    /// voice recognition, reject, enhanced status + control, extended
    /// errors, codec negotiation, HF indicators, eSCO S4.
    fn android_ready(cfg: Config) -> Engine {
        let mut e = Engine::new(cfg);
        e.start();
        assert_eq!(sent(&e.drain()), ["AT+BRSF=54\r"]);
        e.receive(b"\r\n+BRSF: 3943\r\n\r\nOK\r\n");
        assert_eq!(sent(&e.drain()), ["AT+CIND=?\r"]);
        e.receive(ANDROID_CIND.as_bytes());
        assert_eq!(sent(&e.drain()), ["AT+CIND?\r"]);
        e.receive(b"\r\n+CIND: 0,0,1,4,0,5,0\r\n\r\nOK\r\n");
        let out = e.drain();
        assert_eq!(
            events(&out),
            [
                Event::Call(false),
                Event::Setup(0),
                Event::Service(true),
                Event::Signal(4),
                Event::Roaming(false),
                Event::Battery(5),
                Event::Held(0),
            ]
        );
        assert_eq!(sent(&out), ["AT+CMER=3,0,0,1\r"]);
        e.receive(b"\r\nOK\r\n");
        assert_eq!(sent(&e.drain()), ["AT+CHLD=?\r"]);
        e.receive(b"\r\n+CHLD: (0,1,1x,2,2x,3,4)\r\n\r\nOK\r\n");
        let out = e.drain();
        assert_eq!(events(&out), [Event::Ready { ag_features: 3943 }]);
        // Only one command in flight at a time.
        assert_eq!(sent(&out), ["AT+CLIP=1\r"]);
        assert!(e.is_ready());
        assert_eq!(e.hold_actions(), ["0", "1", "1x", "2", "2x", "3", "4"]);
        for (reply, next) in [
            ("AT+CLIP=1", "AT+CCWA=1\r"),
            ("AT+CCWA=1", "AT+CMEE=1\r"),
            ("AT+CMEE=1", "AT+CLCC\r"),
        ] {
            e.receive(b"\r\nOK\r\n");
            assert_eq!(sent(&e.drain()), [next], "after {reply}");
        }
        // No call yet: an empty list.
        e.receive(b"\r\nOK\r\n");
        assert_eq!(events(&e.drain()), [Event::ListEnd]);
        e
    }

    #[test]
    fn slc_with_an_android_phone() {
        let e = android_ready(Config::default());
        assert!(!e.codec_negotiation());
        assert_eq!(e.ag_features(), 3943);
    }

    #[test]
    fn slc_negotiates_codecs_when_both_can() {
        let mut e = Engine::new(Config::with_msbc());
        e.start();
        assert_eq!(sent(&e.drain()), ["AT+BRSF=182\r"]);
        e.receive(b"\r\n+BRSF: 3943\r\n\r\nOK\r\n");
        assert_eq!(sent(&e.drain()), ["AT+BAC=1,2\r"]);
        e.receive(b"\r\nOK\r\n");
        assert_eq!(sent(&e.drain()), ["AT+CIND=?\r"]);
        assert!(e.codec_negotiation());
        // The phone without codec negotiation: no AT+BAC.
        let mut e = Engine::new(Config::with_msbc());
        e.start();
        e.drain();
        e.receive(b"\r\n+BRSF: 359\r\n\r\nOK\r\n");
        assert_eq!(sent(&e.drain()), ["AT+CIND=?\r"]);
        assert!(!e.codec_negotiation());
    }

    #[test]
    fn slc_with_an_old_phone_and_spec_order() {
        // No AT+BRSF (pre-1.0), the spec's indicator order, no three-way.
        let mut e = Engine::new(Config::default());
        e.start();
        e.drain();
        e.receive(b"\r\nERROR\r\n");
        assert_eq!(sent(&e.drain()), ["AT+CIND=?\r"]);
        e.receive(b"\r\n+CIND: (\"service\",(0,1)),(\"call\",(0,1)),(\"callsetup\",(0-3)),(\"callheld\",(0-2)),(\"signal\",(0-5)),(\"roam\",(0,1)),(\"battchg\",(0-5))\r\nOK\r\n");
        e.drain();
        e.receive(b"\r\n+CIND: 1,1,0,0,3,0,2\r\nOK\r\n");
        let out = e.drain();
        assert!(events(&out).contains(&Event::Call(true)));
        assert!(events(&out).contains(&Event::Battery(2)));
        e.receive(b"OK\r");
        let out = e.drain();
        // Straight to ready: no AT+CHLD=? without the phone's three-way bit.
        assert_eq!(events(&out), [Event::Ready { ag_features: 0 }]);
        assert_eq!(sent(&out), ["AT+CLIP=1\r"]);
        e.receive(b"OK\r");
        // Nothing else to set up (no enhanced call status on the phone).
        assert!(e.drain().is_empty());
        // +CIEV indexes follow this phone's order: 2 is "call".
        e.receive(b"\r\n+CIEV: 2,0\r\n");
        assert_eq!(events(&e.drain()), [Event::Call(false)]);
    }

    #[test]
    fn slc_fails_when_the_phone_refuses_indicators() {
        let mut e = Engine::new(Config::default());
        e.start();
        e.receive(b"\r\n+BRSF: 0\r\nOK\r\n");
        e.receive(b"\r\nERROR\r\n");
        let out = e.drain();
        assert!(matches!(events(&out).last(), Some(Event::SetupFailed(_))));
        assert_eq!(e.stage(), Stage::Failed);
        assert_eq!(e.answer(), Err(Error::NotReady));
    }

    #[test]
    fn commands_wait_for_setup() {
        let mut e = Engine::new(Config::default());
        assert_eq!(e.answer(), Err(Error::NotReady));
        assert_eq!(e.dial("123"), Err(Error::NotReady));
        e.start();
        assert_eq!(e.hang_up(), Err(Error::NotReady));
    }

    #[test]
    fn incoming_call_answered_and_ended() {
        let mut e = android_ready(Config::default());
        // Android: callsetup=1, RING, then +CLIP with every RING.
        e.receive(b"\r\n+CIEV: 2,1\r\n\r\nRING\r\n\r\n+CLIP: \"+919876543210\",145,,,\"Asha\"\r\n");
        assert_eq!(
            events(&e.drain()),
            [
                Event::Setup(1),
                Event::Ring,
                Event::Caller {
                    number: "+919876543210".into(),
                    name: Some("Asha".into())
                }
            ]
        );
        e.answer().unwrap();
        assert_eq!(sent(&e.drain()), ["ATA\r"]);
        e.receive(b"\r\nOK\r\n\r\n+CIEV: 1,1\r\n\r\n+CIEV: 2,0\r\n");
        assert_eq!(events(&e.drain()), [Event::Call(true), Event::Setup(0)]);
        e.hang_up().unwrap();
        assert_eq!(sent(&e.drain()), ["AT+CHUP\r"]);
        e.receive(b"\r\nOK\r\n\r\n+CIEV: 1,0\r\n");
        assert_eq!(events(&e.drain()), [Event::Call(false)]);
    }

    #[test]
    fn caller_without_a_name_and_withheld() {
        let mut e = android_ready(Config::default());
        e.receive(b"\r\n+CLIP: \"5551234\",129\r\n");
        assert_eq!(
            events(&e.drain()),
            [Event::Caller {
                number: "5551234".into(),
                name: None
            }]
        );
        e.receive(b"\r\n+CLIP: \"\",128\r\n");
        assert_eq!(
            events(&e.drain()),
            [Event::Caller {
                number: String::new(),
                name: None
            }]
        );
    }

    #[test]
    fn outgoing_call_and_dtmf() {
        let mut e = android_ready(Config::default());
        e.dial("+918012345678").unwrap();
        assert_eq!(sent(&e.drain()), ["ATD+918012345678;\r"]);
        // Queued until the dial's OK.
        e.dtmf('5').unwrap();
        assert!(e.drain().is_empty());
        e.receive(b"\r\nOK\r\n\r\n+CIEV: 2,2\r\n");
        let out = e.drain();
        assert_eq!(sent(&out), ["AT+VTS=5\r"]);
        assert_eq!(events(&out), [Event::Setup(2)]);
        e.receive(b"\r\nOK\r\n\r\n+CIEV: 2,3\r\n\r\n+CIEV: 1,1\r\n\r\n+CIEV: 2,0\r\n");
        assert_eq!(
            events(&e.drain()),
            [Event::Setup(3), Event::Call(true), Event::Setup(0)]
        );
        assert!(e.dial("12;ATH").is_err());
        assert!(e.dial("1+2").is_err());
        assert!(e.dial("").is_err());
        assert!(e.dtmf('E').is_err());
    }

    #[test]
    fn redial_refused_with_no_last_number() {
        let mut e = android_ready(Config::default());
        e.redial().unwrap();
        assert_eq!(sent(&e.drain()), ["AT+BLDN\r"]);
        e.receive(b"\r\n+CME ERROR: 30\r\n");
        assert_eq!(
            events(&e.drain()),
            [Event::Refused {
                command: "AT+BLDN".into()
            }]
        );
        // The next command goes out after the refusal.
        e.dial("100").unwrap();
        assert_eq!(sent(&e.drain()), ["ATD100;\r"]);
        e.receive(b"\r\nNO CARRIER\r\n");
        assert_eq!(
            events(&e.drain()),
            [Event::Refused {
                command: "ATD100;".into()
            }]
        );
        // An unsolicited NO CARRIER (no command waiting) is ignored.
        e.receive(b"\r\nNO CARRIER\r\n");
        assert!(e.drain().is_empty());
    }

    #[test]
    fn call_list_lines() {
        let mut e = android_ready(Config::default());
        e.list_calls().unwrap();
        e.drain();
        e.receive(b"\r\n+CLCC: 1,1,0,0,0,\"+919876543210\",145\r\n\r\n+CLCC: 2,1,5,0,0,\"5551234\",129,\"Ravi\"\r\n\r\nOK\r\n");
        assert_eq!(
            events(&e.drain()),
            [
                Event::ListedCall(ListedCall {
                    index: 1,
                    incoming: true,
                    status: 0,
                    multiparty: false,
                    number: Some("+919876543210".into()),
                    name: None
                }),
                Event::ListedCall(ListedCall {
                    index: 2,
                    incoming: true,
                    status: 5,
                    multiparty: false,
                    number: Some("5551234".into()),
                    name: Some("Ravi".into())
                }),
                Event::ListEnd
            ]
        );
    }

    #[test]
    fn call_waiting_and_hold() {
        let mut e = android_ready(Config::default());
        e.receive(b"\r\n+CCWA: \"5551234\",129,1\r\n\r\n+CIEV: 2,1\r\n");
        assert_eq!(
            events(&e.drain()),
            [
                Event::Waiting {
                    number: "5551234".into(),
                    name: None
                },
                Event::Setup(1)
            ]
        );
        e.hold("2").unwrap();
        assert_eq!(sent(&e.drain()), ["AT+CHLD=2\r"]);
        assert!(e.hold("2;").is_err());
    }

    #[test]
    fn lines_split_across_reads() {
        let mut e = android_ready(Config::default());
        for part in [
            "\r\n+CI",
            "EV: 4,",
            "2\r",
            "\n\r\n+CIEV: 6,1\r\n\r\nRI",
            "NG\r\n",
        ] {
            e.receive(part.as_bytes());
        }
        assert_eq!(
            events(&e.drain()),
            [Event::Signal(2), Event::Battery(1), Event::Ring]
        );
    }

    #[test]
    fn codec_selection() {
        let mut e = android_ready(Config::default());
        // Not negotiating: no AT+BCC.
        assert_eq!(
            e.audio_connect(),
            Err(Error::Unsupported("codec negotiation"))
        );
        // A phone asking for mSBC anyway: CVSD is all there is.
        e.receive(b"\r\n+BCS: 2\r\n");
        assert_eq!(sent(&e.drain()), ["AT+BAC=1\r"]);
        e.receive(b"\r\nOK\r\n+BCS: 1\r\n");
        let out = e.drain();
        assert_eq!(sent(&out), ["AT+BCS=1\r"]);
        assert_eq!(events(&out), [Event::Codec(CODEC_CVSD)]);

        let mut e = Engine::new(Config::with_msbc());
        e.start();
        e.receive(b"\r\n+BRSF: 3943\r\nOK\r\n");
        e.receive(b"OK\r\n");
        e.receive(ANDROID_CIND.as_bytes());
        e.receive(b"+CIND: 0,0,1,4,0,5,0\r\nOK\r\n");
        e.receive(b"OK\r\n");
        e.receive(b"+CHLD: (0,1,2,3)\r\nOK\r\n");
        assert!(e.is_ready());
        for _ in 0..4 {
            e.receive(b"OK\r\n");
        }
        e.drain();
        e.audio_connect().unwrap();
        assert_eq!(sent(&e.drain()), ["AT+BCC\r"]);
        e.receive(b"\r\nOK\r\n\r\n+BCS: 2\r\n");
        let out = e.drain();
        assert_eq!(sent(&out), ["AT+BCS=2\r"]);
        assert_eq!(events(&out), [Event::Codec(CODEC_MSBC)]);
    }

    #[test]
    fn remote_volume() {
        let mut e = android_ready(Config::default());
        e.receive(b"\r\n+VGS: 9\r\n\r\n+VGM=20\r\n");
        assert_eq!(
            events(&e.drain()),
            [Event::SpeakerGain(9), Event::MicGain(15)]
        );
        e.speaker_gain(30).unwrap();
        assert_eq!(sent(&e.drain()), ["AT+VGS=15\r"]);
        e.receive(b"OK\r\n");
        e.mic_gain(8).unwrap();
        assert_eq!(sent(&e.drain()), ["AT+VGM=8\r"]);
    }

    #[test]
    fn junk_and_long_lines_are_harmless() {
        let mut e = android_ready(Config::default());
        e.receive(b"\r\n+BSIR: 1\r\n\r\nhello\r\n+CIEV: 99,1\r\n+CIEV: x\r\n");
        assert!(e.drain().is_empty());
        e.receive(&[b'A'; 10_000]);
        e.receive(b"\r\nRING\r\n");
        assert_eq!(events(&e.drain()), [Event::Ring]);
    }

    #[test]
    fn nrec_only_when_asked() {
        let cfg = Config {
            disable_ag_nrec: true,
            ..Config::default()
        };
        let mut e = Engine::new(cfg);
        e.start();
        e.receive(b"+BRSF: 3\r\nOK\r\n");
        e.receive(ANDROID_CIND.as_bytes());
        e.receive(b"+CIND: 0,0,1,4,0,5,0\r\nOK\r\n");
        e.receive(b"OK\r\n");
        e.receive(b"+CHLD: (0,1,2,3)\r\nOK\r\n");
        e.receive(b"OK\r\nOK\r\n");
        let all = sent(&e.drain());
        assert_eq!(all.last().map(String::as_str), Some("AT+NREC=0\r"));
    }
}
