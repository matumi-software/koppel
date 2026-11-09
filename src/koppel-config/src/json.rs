//! JSON the way Koppel 0.1.x writes and reads its files: System.Text.Json with
//! `JsonSerializerDefaults.Web`, indented, and enums as camelCase names.
//!
//! What that means in detail, each point checked against files written by the C# code:
//!
//! - Two spaces of indent, `": "` between a name and its value, and the platform's line ending
//!   (`\r\n` on Windows, `\n` elsewhere), with no line ending after the last brace.
//! - Strings escape everything outside printable ASCII as `\uXXXX` in upper case hex, and also
//!   `"`, `&`, `'`, `+`, `<`, `>` and the backtick. A backslash is `\\`, and `\b`, `\f`, `\n`,
//!   `\r`, `\t` keep their short forms.
//! - Property names are matched without regard to case when read, and unknown ones are ignored.
//! - A `DateTimeOffset` is `2026-11-09T08:30:15.1234567+00:00`: the fraction has at most seven
//!   digits with trailing zeros cut, none at all when it is zero, and the offset is never `Z`.
//! - A dictionary keeps the order it was written in, and a repeated key takes the last value.

use std::fmt;
use std::io;
use std::marker::PhantomData;

use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::ser::{CharEscape, Formatter, PrettyFormatter};
use serde_json::value::RawValue;
use serde_json::{Map, Value};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

/// The line ending System.Text.Json writes: `Environment.NewLine`.
#[cfg(windows)]
pub(crate) const NEW_LINE: &[u8] = b"\r\n";
#[cfg(not(windows))]
pub(crate) const NEW_LINE: &[u8] = b"\n";

/// `value` as Koppel 0.1.x writes it.
pub fn to_vec<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut out = Vec::with_capacity(256);
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, DotnetFormatter::new());
    value.serialize(&mut serializer)?;
    if NEW_LINE == b"\n" {
        return Ok(out);
    }
    // Every line break here is the formatter's: a line break inside a string is escaped.
    let mut converted = Vec::with_capacity(out.len() + out.len() / 16);
    for byte in out {
        if byte == b'\n' {
            converted.extend_from_slice(NEW_LINE);
        } else {
            converted.push(byte);
        }
    }
    Ok(converted)
}

/// Reads `bytes` as Koppel 0.1.x does: a UTF-8 byte order mark is skipped, and the top level
/// property names are matched without regard to case.
pub fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    from_value(parse(bytes)?)
}

/// The JSON in `bytes`, after a UTF-8 byte order mark if there is one.
pub fn parse(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
}

/// `value` as a `T`, with the top level property names matched without regard to case.
pub fn from_value<T: DeserializeOwned>(mut value: Value) -> Result<T, serde_json::Error> {
    if let (Value::Object(map), Some(fields)) = (&mut value, struct_fields::<T>()) {
        fold_names(map, fields);
    }
    T::deserialize(value)
}

/// Renames properties that match a field of the type in another case, as System.Text.Json's
/// case-insensitive matching would read them. An exact match wins over one in another case.
fn fold_names(map: &mut Map<String, Value>, fields: &[&str]) {
    let renames: Vec<(String, &str)> = map
        .keys()
        .filter(|key| !fields.contains(&key.as_str()))
        .filter_map(|key| {
            fields
                .iter()
                .find(|field| field.eq_ignore_ascii_case(key))
                .map(|field| (key.clone(), *field))
        })
        .collect();
    for (from, to) in renames {
        if let Some(value) = map.shift_remove(&from)
            && !map.contains_key(to)
        {
            map.insert(to.to_string(), value);
        }
    }
}

/// The JSON names of a struct's fields, as serde's derive lists them, or `None` for anything
/// that is not a struct.
fn struct_fields<T: DeserializeOwned>() -> Option<&'static [&'static str]> {
    struct Probe;
    #[derive(Debug)]
    struct Found(Option<&'static [&'static str]>);
    impl fmt::Display for Found {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("probe")
        }
    }
    impl std::error::Error for Found {}
    impl de::Error for Found {
        fn custom<M: fmt::Display>(_: M) -> Self {
            Found(None)
        }
    }
    impl<'de> Deserializer<'de> for Probe {
        type Error = Found;
        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Found> {
            Err(Found(None))
        }
        fn deserialize_struct<V: Visitor<'de>>(
            self,
            _: &'static str,
            fields: &'static [&'static str],
            _: V,
        ) -> Result<V::Value, Found> {
            Err(Found(Some(fields)))
        }
        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
            option unit unit_struct newtype_struct seq tuple tuple_struct map enum identifier
            ignored_any
        }
    }
    match T::deserialize(Probe) {
        Err(Found(fields)) => fields,
        Ok(_) => None,
    }
}

/// serde_json's pretty printer with System.Text.Json's string escaping.
struct DotnetFormatter {
    inner: PrettyFormatter<'static>,
}

impl DotnetFormatter {
    fn new() -> Self {
        Self {
            inner: PrettyFormatter::with_indent(b"  "),
        }
    }
}

/// True for a character System.Text.Json's default encoder writes as it is.
fn passes(c: char) -> bool {
    matches!(c, ' '..='~') && !matches!(c, '"' | '&' | '\'' | '+' | '<' | '>' | '\\' | '`')
}

fn write_unicode_escape<W: ?Sized + io::Write>(writer: &mut W, c: char) -> io::Result<()> {
    let mut units = [0u16; 2];
    for unit in c.encode_utf16(&mut units) {
        write!(writer, "\\u{unit:04X}")?;
    }
    Ok(())
}

impl Formatter for DotnetFormatter {
    fn begin_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.inner.begin_array(writer)
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.inner.end_array(writer)
    }
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.inner.begin_array_value(writer, first)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.inner.end_array_value(writer)
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.inner.begin_object(writer)
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.inner.end_object(writer)
    }
    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.inner.begin_object_key(writer, first)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.inner.begin_object_value(writer)
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.inner.end_object_value(writer)
    }

    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut start = 0;
        for (index, c) in fragment.char_indices() {
            if passes(c) {
                continue;
            }
            writer.write_all(&fragment.as_bytes()[start..index])?;
            write_unicode_escape(writer, c)?;
            start = index + c.len_utf8();
        }
        writer.write_all(&fragment.as_bytes()[start..])
    }

    fn write_char_escape<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        char_escape: CharEscape,
    ) -> io::Result<()> {
        let short: &[u8] = match char_escape {
            CharEscape::ReverseSolidus => b"\\\\",
            CharEscape::Backspace => b"\\b",
            CharEscape::FormFeed => b"\\f",
            CharEscape::LineFeed => b"\\n",
            CharEscape::CarriageReturn => b"\\r",
            CharEscape::Tab => b"\\t",
            CharEscape::Quote => return write_unicode_escape(writer, '"'),
            CharEscape::Solidus => b"/",
            CharEscape::AsciiControl(byte) => {
                return write_unicode_escape(writer, char::from(byte));
            }
        };
        writer.write_all(short)
    }
}

/// A `DateTimeOffset`, read and written as System.Text.Json does.
pub mod date_time_offset {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &OffsetDateTime,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        raw_date(*value)?.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<OffsetDateTime, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_date(&text).ok_or_else(|| {
            de::Error::custom(format_args!(
                "{text:?} is not a date and time with an offset"
            ))
        })
    }

    /// For an `Option<DateTimeOffset>`, null when absent.
    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(
            value: &Option<OffsetDateTime>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            match value {
                Some(value) => raw_date(*value)?.serialize(serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<OffsetDateTime>, D::Error> {
            match Option::<String>::deserialize(deserializer)? {
                None => Ok(None),
                Some(text) => parse_date(&text).map(Some).ok_or_else(|| {
                    de::Error::custom(format_args!(
                        "{text:?} is not a date and time with an offset"
                    ))
                }),
            }
        }
    }
}

/// A date as a raw JSON string: System.Text.Json writes the `+` of the offset as it is, where any
/// other string would have it escaped.
fn raw_date<E: serde::ser::Error>(value: OffsetDateTime) -> Result<Box<RawValue>, E> {
    RawValue::from_string(format!("\"{}\"", format_date(value))).map_err(E::custom)
}

/// `2026-11-09T08:30:15.1234567+00:00`, as System.Text.Json writes a `DateTimeOffset`.
pub fn format_date(value: OffsetDateTime) -> String {
    let mut text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        value.year(),
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second()
    );
    let ticks = value.nanosecond() / 100;
    if ticks != 0 {
        let fraction = format!("{ticks:07}");
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    let offset = value.offset();
    let minutes = offset.whole_minutes();
    let sign = if minutes < 0 { '-' } else { '+' };
    let minutes = minutes.unsigned_abs();
    text.push_str(&format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60));
    text
}

/// Reads `yyyy-MM-ddTHH:mm[:ss[.fffffff]]` followed by `Z` or `+HH:mm`. Digits past the seventh
/// are cut, as .NET does. A time without an offset is refused: .NET would read it in the local
/// time zone, and nothing Koppel wrote has one.
pub fn parse_date(text: &str) -> Option<OffsetDateTime> {
    fn digits(text: &str, count: usize) -> Option<(u32, &str)> {
        let (head, rest) = (text.get(..count)?, text.get(count..)?);
        head.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| (head.parse().ok(), rest))
            .and_then(|(value, rest)| value.map(|v| (v, rest)))
    }
    fn expect(text: &str, c: char) -> Option<&str> {
        text.strip_prefix(c)
    }

    let (year, rest) = digits(text, 4)?;
    let (month, rest) = digits(expect(rest, '-')?, 2)?;
    let (day, rest) = digits(expect(rest, '-')?, 2)?;
    let (hour, rest) = digits(expect(rest, 'T')?, 2)?;
    let (minute, mut rest) = digits(expect(rest, ':')?, 2)?;
    let mut second = 0;
    let mut nanos = 0;
    if let Some(after) = rest.strip_prefix(':') {
        let (value, after) = digits(after, 2)?;
        second = value;
        rest = after;
        if let Some(after) = rest.strip_prefix('.') {
            let count = after.bytes().take_while(u8::is_ascii_digit).count();
            if count == 0 {
                return None;
            }
            let fraction = &after[..count];
            let kept = &fraction[..count.min(7)];
            let ticks: u32 = format!("{kept:0<7}").parse().ok()?;
            nanos = ticks * 100;
            rest = &after[count..];
        }
    }
    let offset = if rest == "Z" {
        UtcOffset::UTC
    } else {
        let sign: i8 = match rest.as_bytes().first()? {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let (hours, after) = digits(&rest[1..], 2)?;
        let (minutes, after) = digits(expect(after, ':')?, 2)?;
        if !after.is_empty() {
            return None;
        }
        UtcOffset::from_hms(
            sign * i8::try_from(hours).ok()?,
            sign * i8::try_from(minutes).ok()?,
            0,
        )
        .ok()?
    };
    let date = Date::from_calendar_date(
        i32::try_from(year).ok()?,
        Month::try_from(u8::try_from(month).ok()?).ok()?,
        u8::try_from(day).ok()?,
    )
    .ok()?;
    let time = Time::from_hms_nano(
        u8::try_from(hour).ok()?,
        u8::try_from(minute).ok()?,
        u8::try_from(second).ok()?,
        nanos,
    )
    .ok()?;
    Some(PrimitiveDateTime::new(date, time).assume_offset(offset))
}

/// A JSON object read into a list, so it is written back in the order it came in. A key that
/// appears twice keeps its last value, as System.Text.Json reads a dictionary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrderedMap<V>(pub Vec<(String, V)>);

impl<V> OrderedMap<V> {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// The value for `key`, if there is one.
    pub fn get(&self, key: &str) -> Option<&V> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Sets `key`, in its old place if it was there, otherwise at the end.
    pub fn insert(&mut self, key: impl Into<String>, value: V) {
        let key = key.into();
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key, value)),
        }
    }

    /// Takes `key` out, if it was there.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        let index = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(index).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<V: Serialize> Serialize for OrderedMap<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for OrderedMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor<V>(PhantomData<V>);
        impl<'de, V: Deserialize<'de>> Visitor<'de> for MapVisitor<V> {
            type Value = OrderedMap<V>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut map = OrderedMap::new();
                while let Some((key, value)) = access.next_entry::<String, V>()? {
                    map.insert(key, value);
                }
                Ok(map)
            }
        }
        deserializer.deserialize_map(MapVisitor(PhantomData))
    }
}

/// An `OrderedMap` of `DateTimeOffset` values.
pub mod date_map {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &Option<OrderedMap<OffsetDateTime>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            None => serializer.serialize_none(),
            Some(map) => {
                let mut out = serializer.serialize_map(Some(map.len()))?;
                for (key, value) in map.iter() {
                    out.serialize_entry(key, &raw_date(*value)?)?;
                }
                out.end()
            }
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OrderedMap<OffsetDateTime>>, D::Error> {
        let Some(texts) = Option::<OrderedMap<String>>::deserialize(deserializer)? else {
            return Ok(None);
        };
        let mut map = OrderedMap::new();
        for (key, text) in texts.0 {
            let value = parse_date(&text).ok_or_else(|| {
                de::Error::custom(format_args!(
                    "{text:?} is not a date and time with an offset"
                ))
            })?;
            map.insert(key, value);
        }
        Ok(Some(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> String {
        text.replace('\n', std::str::from_utf8(NEW_LINE).unwrap())
    }

    #[test]
    fn strings_are_escaped_as_system_text_json_escapes_them() {
        // The expected text is what the C# code wrote for the same string.
        let value = "lab <pc> & 'x' + é \"q\" \\ / \n\t\r\u{8}\u{c} \u{1} \u{1f} \u{7f} \u{80} \u{a0} \u{2028} \u{feff} 😀 ` ~";
        let expected = r#""lab \u003Cpc\u003E \u0026 \u0027x\u0027 \u002B \u00E9 \u0022q\u0022 \\ / \n\t\r\b\f \u0001 \u001F \u007F \u0080 \u00A0 \u2028 \uFEFF \uD83D\uDE00 \u0060 ~""#;
        assert_eq!(
            String::from_utf8(to_vec(&value).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn layout_matches_system_text_json() {
        #[derive(Serialize)]
        struct Shape {
            a: u32,
            b: Vec<u32>,
            c: OrderedMap<u32>,
            d: Vec<u32>,
        }
        let shape = Shape {
            a: 1,
            b: vec![1, 2],
            c: OrderedMap::new(),
            d: vec![],
        };
        let expected =
            lines("{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ],\n  \"c\": {},\n  \"d\": []\n}");
        assert_eq!(
            String::from_utf8(to_vec(&shape).unwrap()).unwrap(),
            expected
        );
    }

    #[test]
    fn dates_are_written_as_system_text_json_writes_them() {
        let at = |text| parse_date(text).unwrap();
        // Each pair is what C# wrote for a value, and that value read back and written again.
        for text in [
            "2026-11-09T10:15:30+00:00",
            "2026-11-09T10:15:30.5+00:00",
            "2026-11-09T10:15:30.1234567+02:00",
            "2026-11-09T10:15:30.12-05:30",
            "2026-11-09T10:15:30.0000001+00:00",
        ] {
            assert_eq!(format_date(at(text)), text);
        }
    }

    #[test]
    fn dates_are_read_as_dotnet_reads_them() {
        assert_eq!(
            format_date(parse_date("2026-11-09T10:15:30Z").unwrap()),
            "2026-11-09T10:15:30+00:00"
        );
        assert_eq!(
            format_date(parse_date("2026-11-09T10:15:30.123456789+00:00").unwrap()),
            "2026-11-09T10:15:30.1234567+00:00"
        );
        assert_eq!(
            format_date(parse_date("2026-11-09T10:15+00:00").unwrap()),
            "2026-11-09T10:15:00+00:00"
        );
        for refused in [
            "2026-11-09T10:15:30",
            "2026-11-09",
            "2026-11-09 10:15:30Z",
            "2026-11-09t10:15:30z",
            "",
        ] {
            assert_eq!(parse_date(refused), None, "{refused}");
        }
    }

    #[test]
    fn names_match_without_regard_to_case_and_unknown_ones_are_ignored() {
        #[derive(Deserialize, Debug, PartialEq)]
        #[serde(rename_all = "camelCase")]
        struct Shape {
            share_on_startup: bool,
            #[serde(default)]
            role: String,
        }
        let read: Shape = from_slice(br#"{"SHAREONSTARTUP":true,"Role":"x","extra":5}"#).unwrap();
        assert_eq!(
            read,
            Shape {
                share_on_startup: true,
                role: "x".into()
            }
        );
        let exact_wins: Shape =
            from_slice(br#"{"shareOnStartup":true,"ShareOnStartup":false}"#).unwrap();
        assert!(exact_wins.share_on_startup);
    }

    #[test]
    fn a_byte_order_mark_is_skipped() {
        let read: OrderedMap<u32> = from_slice(b"\xEF\xBB\xBF{\"a\":1}").unwrap();
        assert_eq!(read.get("a"), Some(&1));
    }

    #[test]
    fn a_repeated_key_keeps_its_last_value_and_its_first_place() {
        let read: OrderedMap<String> = from_slice(br#"{"a":"1","b":"x","a":"2"}"#).unwrap();
        assert_eq!(
            read.0,
            vec![
                ("a".to_string(), "2".to_string()),
                ("b".to_string(), "x".to_string())
            ]
        );
    }
}
