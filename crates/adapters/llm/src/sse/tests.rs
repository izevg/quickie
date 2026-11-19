use super::{Decoder, Event, MAX_DATA_BYTES, MAX_LINE_BYTES, Overflow};

fn data(text: &str) -> Event {
    Event {
        name: None,
        data: text.to_owned(),
    }
}

/// Every event of `pieces`, pushed in order, then finished.
fn decode(pieces: &[&[u8]]) -> Vec<Event> {
    let mut decoder = Decoder::default();
    let mut events: Vec<Event> = pieces
        .iter()
        .flat_map(|piece| decoder.push(piece).unwrap())
        .collect();
    events.extend(decoder.finish().unwrap());
    events
}

// ---- bounds ------------------------------------------------------------------------------------

#[test]
fn a_line_of_the_longest_length_is_read() {
    let mut line = b"data: ".to_vec();
    line.resize(MAX_LINE_BYTES, b'x');
    line.extend_from_slice(b"\n\n");

    let events = decode(&[&line]);

    assert_eq!(events[0].data.len(), MAX_LINE_BYTES - 6);
}

#[test]
fn a_line_one_byte_too_long_overflows_and_lets_go_of_its_bytes() {
    let mut decoder = Decoder::default();
    let line = vec![b'x'; MAX_LINE_BYTES + 1];

    let result = decoder.push(&line);

    assert_eq!(result, Err(Overflow));
    assert_eq!(decoder.line.capacity(), 0);
}

#[test]
fn two_mebibytes_without_a_line_break_overflow() {
    let mut decoder = Decoder::default();
    let piece = vec![b'x'; 64 * 1024];

    let overflowed = (0..32).any(|_| decoder.push(&piece).is_err());

    assert!(overflowed);
    assert!(decoder.line.len() <= MAX_LINE_BYTES);
}

#[test]
fn data_lines_of_the_most_an_event_holds_are_read() {
    let line = format!("data: {}\n", "x".repeat(1023));
    let stream = line.repeat(MAX_DATA_BYTES / 1024);

    let events = decode(&[stream.as_bytes(), b"\n"]);

    assert_eq!(events[0].data.len(), MAX_DATA_BYTES - 1);
}

#[test]
fn data_lines_past_what_an_event_holds_overflow() {
    let mut decoder = Decoder::default();
    let line = format!("data: {}\n", "x".repeat(1023));
    let stream = line.repeat(MAX_DATA_BYTES / 1024 + 1);

    let result = decoder.push(stream.as_bytes());

    assert_eq!(result, Err(Overflow));
    assert_eq!(decoder.data.capacity(), 0);
}

#[test]
fn a_last_line_too_long_overflows_at_the_end() {
    let line = format!("data: {}\n", "x".repeat(1023));
    let full = line.repeat(MAX_DATA_BYTES / 1024);
    let mut decoder = Decoder::default();
    decoder.push(full.as_bytes()).unwrap();
    decoder.push(b"data: x").unwrap();

    let result = decoder.finish();

    assert_eq!(result, Err(Overflow));
}

/// A chat stream as servers send it: two tokens, one of them Cyrillic and one an emoji, a
/// usage chunk, a comment, and the end.
const STREAM: &str = ": keep-alive\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"Привет\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\" 🌍\"}}]}\r\n\r\n\
data: {\"usage\":{\"completion_tokens\":2}}\r\r\
data: [DONE]\n\n";

fn stream_events() -> Vec<Event> {
    vec![
        data("{\"choices\":[{\"delta\":{\"content\":\"Привет\"}}]}"),
        data("{\"choices\":[{\"delta\":{\"content\":\" 🌍\"}}]}"),
        data("{\"usage\":{\"completion_tokens\":2}}"),
        data("[DONE]"),
    ]
}

#[test]
fn a_stream_in_one_piece_decodes_into_its_events() {
    let events = decode(&[STREAM.as_bytes()]);

    assert_eq!(events, stream_events());
}

#[test]
fn a_stream_split_at_any_byte_decodes_the_same() {
    let bytes = STREAM.as_bytes();

    for at in 0..=bytes.len() {
        let (head, tail) = bytes.split_at(at);

        let events = decode(&[head, tail]);

        assert_eq!(events, stream_events(), "split at byte {at}");
    }
}

#[test]
fn a_stream_split_at_any_two_bytes_decodes_the_same() {
    let bytes = STREAM.as_bytes();

    for first in (0..bytes.len()).step_by(7) {
        for second in first..=bytes.len() {
            let events = decode(&[&bytes[..first], &bytes[first..second], &bytes[second..]]);

            assert_eq!(events, stream_events(), "split at {first} and {second}");
        }
    }
}

#[test]
fn a_stream_one_byte_at_a_time_decodes_the_same() {
    let pieces: Vec<&[u8]> = STREAM.as_bytes().chunks(1).collect();

    let events = decode(&pieces);

    assert_eq!(events, stream_events());
}

#[test]
fn a_character_split_inside_its_bytes_arrives_whole() {
    let bytes = "data: я\n\n".as_bytes();
    let (head, tail) = bytes.split_at(7);

    let events = decode(&[head, tail]);

    assert_eq!(events, [data("я")]);
}

#[test]
fn an_event_completes_only_at_its_blank_line() {
    let mut decoder = Decoder::default();

    let before = decoder.push(b"data: a\n");
    let after = decoder.push(b"\n");

    assert_eq!(before, Ok(Vec::new()));
    assert_eq!(after, Ok(vec![data("a")]));
}

#[test]
fn a_crlf_split_between_its_bytes_is_one_line_break() {
    let events = decode(&[b"data: a\r", b"\ndata: b\r\n\r\n"]);

    assert_eq!(events, [data("a\nb")]);
}

#[test]
fn a_lone_cr_ends_a_line() {
    let events = decode(&[b"data: a\rdata: b\r\r"]);

    assert_eq!(events, [data("a\nb")]);
}

#[test]
fn data_lines_join_with_line_breaks() {
    let events = decode(&[b"data: one\ndata: two\ndata:\n\n"]);

    assert_eq!(events, [data("one\ntwo\n")]);
}

#[test]
fn only_one_space_after_the_colon_is_dropped() {
    let events = decode(&[b"data:no space\n\ndata:  two spaces\n\n"]);

    assert_eq!(events, [data("no space"), data(" two spaces")]);
}

#[test]
fn a_field_without_a_colon_has_an_empty_value() {
    let events = decode(&[b"data\n\n"]);

    assert_eq!(events, [data("")]);
}

#[test]
fn an_event_names_its_type() {
    let events = decode(&[b"event: error\ndata: {\"error\":\"boom\"}\n\n"]);

    assert_eq!(
        events,
        [Event {
            name: Some("error".to_owned()),
            data: "{\"error\":\"boom\"}".to_owned(),
        }]
    );
}

#[test]
fn an_event_type_does_not_carry_over_to_the_next_event() {
    let events = decode(&[b"event: ping\ndata: 1\n\ndata: 2\n\n"]);

    assert_eq!(events[1], data("2"));
}

#[test]
fn a_blank_line_without_data_dispatches_nothing() {
    let events = decode(&[b"event: ping\n\n\n\nid: 7\nretry: 10\n\n"]);

    assert_eq!(events, []);
}

#[test]
fn comments_are_ignored() {
    let events = decode(&[b": ping\ndata: x\n: another\n\n"]);

    assert_eq!(events, [data("x")]);
}

#[test]
fn a_byte_order_mark_at_the_start_is_dropped() {
    let events = decode(&["\u{feff}data: x\n\n".as_bytes()]);

    assert_eq!(events, [data("x")]);
}

#[test]
fn a_byte_order_mark_later_is_kept() {
    let events = decode(&["data: x\n\n\u{feff}data: y\n\n".as_bytes()]);

    assert_eq!(events, [data("x")]);
}

#[test]
fn an_event_cut_off_by_the_end_of_the_stream_is_kept() {
    let events = decode(&[b"data: [DONE]"]);

    assert_eq!(events, [data("[DONE]")]);
}

#[test]
fn an_event_ended_by_one_line_break_at_the_end_is_kept() {
    let events = decode(&[b"data: last\n"]);

    assert_eq!(events, [data("last")]);
}

#[test]
fn an_empty_stream_has_no_events() {
    let events = decode(&[b""]);

    assert_eq!(events, []);
}

#[test]
fn invalid_utf8_is_replaced_instead_of_failing() {
    let events = decode(&[b"data: a\xffb\n\n"]);

    assert_eq!(events, [data("a\u{fffd}b")]);
}
