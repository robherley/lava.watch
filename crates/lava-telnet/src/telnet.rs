//! Minimal telnet protocol handling — just enough to stream a terminal
//! animation to a raw `telnet` client. We negotiate three options up front
//! (server-side echo, suppress-go-ahead, and NAWS window-size reports), then
//! treat the inbound byte stream as terminal input for the engine plus the
//! occasional IAC command we either act on (NAWS) or politely refuse.
//!
//! We never need to escape *outbound* data: every byte the renderer emits is
//! a printable ANSI escape or a UTF-8 block glyph, none of which is `0xFF`
//! (IAC), so frame bytes pass through a telnet stream unaltered.

// Telnet protocol bytes (RFC 854 / 855) and the option codes we care about.
const IAC: u8 = 255;
const SE: u8 = 240;
const SB: u8 = 250;
const WILL: u8 = 251;
const WONT: u8 = 252;
const DO: u8 = 253;
const DONT: u8 = 254;

const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_NAWS: u8 = 31;

/// Bytes we send on connect to put the client into the mode we want: we echo
/// (so the client doesn't local-echo keystrokes onto the animation), suppress
/// go-ahead (full-duplex, char-at-a-time), and ask the client to report its
/// window size via NAWS.
pub(crate) const INITIAL_NEGOTIATION: &[u8] = &[
    IAC, WILL, OPT_ECHO, //
    IAC, WILL, OPT_SGA, //
    IAC, DO, OPT_NAWS, //
];

/// Something the parser pulled out of the inbound telnet byte stream.
pub(crate) enum Event {
    /// Terminal input bytes (escape sequences, keypresses) for the engine.
    Data(Vec<u8>),
    /// A NAWS window-size report, in character columns × rows.
    Resize(u16, u16),
}

#[derive(Default)]
enum State {
    #[default]
    Data,
    Iac,
    /// Saw `IAC <verb>`; the next byte is the option being negotiated.
    Verb(u8),
    /// Saw `IAC SB`; the next byte is the subnegotiation's option code.
    SubOption,
    SubData,
    SubIac,
}

/// Incremental telnet stream parser. Feed it chunks as they arrive off the
/// socket; it carries command state across chunk boundaries.
#[derive(Default)]
pub(crate) struct Parser {
    state: State,
    sub_option: u8,
    sub_payload: Vec<u8>,
}

impl Parser {
    /// Consume `input`, appending extracted [`Event`]s to `events` and any
    /// protocol replies (option refusals) to `reply`.
    pub(crate) fn feed(&mut self, input: &[u8], events: &mut Vec<Event>, reply: &mut Vec<u8>) {
        // Contiguous runs of data bytes are emitted as one `Data` event so a
        // multi-byte escape sequence (e.g. an arrow key) reaches `parse_input`
        // intact rather than split byte-by-byte.
        let mut data = Vec::new();
        for &b in input {
            match self.state {
                State::Data => {
                    if b == IAC {
                        self.state = State::Iac;
                    } else {
                        data.push(b);
                    }
                }
                State::Iac => match b {
                    // Escaped 0xFF literal in the data stream.
                    IAC => {
                        data.push(IAC);
                        self.state = State::Data;
                    }
                    WILL | WONT | DO | DONT => self.state = State::Verb(b),
                    SB => {
                        self.sub_payload.clear();
                        self.state = State::SubOption;
                    }
                    // Standalone two-byte commands (NOP, etc.) — ignore.
                    _ => self.state = State::Data,
                },
                State::Verb(verb) => {
                    Self::respond(verb, b, reply);
                    self.state = State::Data;
                }
                State::SubOption => {
                    self.sub_option = b;
                    self.state = State::SubData;
                }
                State::SubData => {
                    if b == IAC {
                        self.state = State::SubIac;
                    } else {
                        self.sub_payload.push(b);
                    }
                }
                State::SubIac => match b {
                    // Escaped 0xFF inside the subnegotiation payload.
                    IAC => {
                        self.sub_payload.push(IAC);
                        self.state = State::SubData;
                    }
                    SE => {
                        self.finish_subneg(events);
                        self.state = State::Data;
                    }
                    // Malformed — bail back to the top level.
                    _ => self.state = State::Data,
                },
            }
        }
        if !data.is_empty() {
            events.push(Event::Data(data));
        }
    }

    /// Reply to an option negotiation. We accept only the three options we
    /// drive ourselves and refuse everything else. Refusals (WONT/DONT) are
    /// terminal in the telnet state machine, so they can't trigger a loop.
    fn respond(verb: u8, opt: u8, reply: &mut Vec<u8>) {
        match verb {
            // Peer asks us to enable an option.
            DO => {
                if opt != OPT_ECHO && opt != OPT_SGA {
                    reply.extend_from_slice(&[IAC, WONT, opt]);
                }
                // ECHO / SGA we already offered — no reply needed.
            }
            // Peer announces it will enable an option.
            WILL => {
                if opt != OPT_NAWS {
                    reply.extend_from_slice(&[IAC, DONT, opt]);
                }
                // NAWS we requested — accept silently.
            }
            // Peer refuses / disables — acknowledge so it stops asking.
            DONT => reply.extend_from_slice(&[IAC, WONT, opt]),
            WONT => reply.extend_from_slice(&[IAC, DONT, opt]),
            _ => {}
        }
    }

    fn finish_subneg(&mut self, events: &mut Vec<Event>) {
        if self.sub_option == OPT_NAWS && self.sub_payload.len() == 4 {
            let cols = u16::from_be_bytes([self.sub_payload[0], self.sub_payload[1]]);
            let rows = u16::from_be_bytes([self.sub_payload[2], self.sub_payload[3]]);
            // Width 0 means "unknown" per the NAWS spec — ignore those.
            if cols > 0 && rows > 0 {
                events.push(Event::Resize(cols, rows));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_of(events: &[Event]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Data(d) => Some(d.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn resize_of(events: &[Event]) -> Option<(u16, u16)> {
        events.iter().find_map(|e| match e {
            Event::Resize(c, r) => Some((*c, *r)),
            _ => None,
        })
    }

    #[test]
    fn plain_data_passes_through() {
        let mut p = Parser::default();
        let (mut ev, mut reply) = (Vec::new(), Vec::new());
        p.feed(b"\x1b[Cq", &mut ev, &mut reply);
        assert_eq!(data_of(&ev), b"\x1b[Cq");
        assert!(reply.is_empty());
    }

    #[test]
    fn naws_subnegotiation_yields_resize() {
        let mut p = Parser::default();
        let (mut ev, mut reply) = (Vec::new(), Vec::new());
        // IAC SB NAWS 0 120 0 40 IAC SE
        p.feed(
            &[IAC, SB, OPT_NAWS, 0, 120, 0, 40, IAC, SE],
            &mut ev,
            &mut reply,
        );
        assert_eq!(resize_of(&ev), Some((120, 40)));
    }

    #[test]
    fn naws_split_across_chunks() {
        let mut p = Parser::default();
        let (mut ev, mut reply) = (Vec::new(), Vec::new());
        p.feed(&[IAC, SB, OPT_NAWS, 0], &mut ev, &mut reply);
        p.feed(&[120, 0, 40, IAC, SE], &mut ev, &mut reply);
        assert_eq!(resize_of(&ev), Some((120, 40)));
    }

    #[test]
    fn naws_with_escaped_255_dimension() {
        let mut p = Parser::default();
        let (mut ev, mut reply) = (Vec::new(), Vec::new());
        // A 255-wide column count is sent as the escaped pair IAC IAC.
        p.feed(
            &[IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 40, IAC, SE],
            &mut ev,
            &mut reply,
        );
        assert_eq!(resize_of(&ev), Some((255, 40)));
    }

    #[test]
    fn iac_does_not_leak_into_data() {
        let mut p = Parser::default();
        let (mut ev, mut reply) = (Vec::new(), Vec::new());
        p.feed(&[b'q', IAC, WILL, OPT_NAWS, b'a'], &mut ev, &mut reply);
        assert_eq!(data_of(&ev), b"qa");
    }

    #[test]
    fn unknown_options_are_refused() {
        let mut p = Parser::default();
        let (mut ev, mut reply) = (Vec::new(), Vec::new());
        // Peer offers to enable an option we don't want.
        p.feed(&[IAC, WILL, 24 /* TERMINAL-TYPE */], &mut ev, &mut reply);
        assert_eq!(reply, vec![IAC, DONT, 24]);
        // Peer asks us to enable one we don't drive.
        reply.clear();
        p.feed(&[IAC, DO, 34 /* LINEMODE */], &mut ev, &mut reply);
        assert_eq!(reply, vec![IAC, WONT, 34]);
    }

    #[test]
    fn driven_options_are_not_refused() {
        let mut p = Parser::default();
        let (mut ev, mut reply) = (Vec::new(), Vec::new());
        // Acceptances of the options we offered draw no reply (no loop).
        p.feed(
            &[IAC, DO, OPT_ECHO, IAC, DO, OPT_SGA, IAC, WILL, OPT_NAWS],
            &mut ev,
            &mut reply,
        );
        assert!(reply.is_empty());
    }
}
