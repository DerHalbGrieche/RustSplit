//! Reading and writing LiveSplit `.lss` split files.

use std::path::Path;

use base64::Engine;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use thiserror::Error;
use xmltree::XMLNode;

use crate::model::{
    Attempt, Run, RunMetadata, Segment, CURRENT_COMPARISON, GENERATED_COMPARISONS, HCP, NONE,
    PERSONAL_BEST,
};
use crate::time::{Time, TimeSpan};
use crate::xml::{
    attribute, child, child_text, children_named, element_with_text, parse_bool, parse_xml,
    set_inner_xml, set_text, text, text_trimmed, xml_to_string, XmlElement,
};

#[derive(Debug, Error)]
pub enum LssError {
    #[error("could not read split file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid split file: {0}")]
    Xml(#[from] crate::xml::XmlError),
    #[error("invalid split file: {0}")]
    Invalid(String),
    #[error("invalid base64 data: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("split file contains no segments")]
    NoSegments,
}

pub fn load_run(path: impl AsRef<Path>) -> Result<Run, LssError> {
    let path = path.as_ref();
    let data = decode_text(&std::fs::read(path)?)?;
    let mut run = load_run_from_str(&data)?;
    run.file_path = Some(path.to_path_buf());
    Ok(run)
}

pub fn load_run_from_str(data: &str) -> Result<Run, LssError> {
    // LiveSplit writes files with an UTF-8 byte order mark.  `xmltree` does
    // not accept a BOM in front of the XML declaration, so remove it first.
    let data = data.strip_prefix('\u{feff}').unwrap_or(data);
    let root = parse_xml(data.as_bytes())?;
    if root.name != "Run" {
        return Err(LssError::Invalid(format!(
            "expected <Run> root element, found <{}>",
            root.name
        )));
    }

    let mut run = Run::default();
    run.game_name = child_text(&root, "GameName").unwrap_or_default();
    run.category_name = child_text(&root, "CategoryName").unwrap_or_default();
    run.layout_path = child_text(&root, "LayoutPath").filter(|path| !path.is_empty());
    run.game_icon = child(&root, "GameIcon").and_then(decode_image);
    run.offset = child_text(&root, "Offset")
        .and_then(|value| TimeSpan::parse(&value).ok())
        .unwrap_or_default();
    run.attempt_count = child_text(&root, "AttemptCount")
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);

    if let Some(metadata) = child(&root, "Metadata") {
        run.metadata = parse_metadata(metadata);
    }

    // LiveSplit 1.5+ uses <AttemptHistory>.  Older versions used a
    // <RunHistory> containing indexed <Time> elements.
    if let Some(history) = child(&root, "AttemptHistory") {
        for attempt in children_named(history, "Attempt") {
            run.attempts.push(parse_attempt(attempt));
        }
    } else if let Some(history) = child(&root, "RunHistory") {
        for time in children_named(history, "Time") {
            run.attempts.push(Attempt {
                index: parse_i32_attribute(time, "id").unwrap_or(0),
                time: parse_time(time),
                started: None,
                ended: None,
                pause_time: None,
            });
        }
    }

    let Some(segments) = child(&root, "Segments") else {
        return Err(LssError::NoSegments);
    };

    let mut custom_comparisons = vec![PERSONAL_BEST.to_string()];
    for segment_element in children_named(segments, "Segment") {
        run.segments
            .push(parse_segment(segment_element, &mut custom_comparisons));
    }
    if run.segments.is_empty() {
        return Err(LssError::NoSegments);
    }

    // Custom comparisons are encoded as names on the split time elements.
    // Generated comparisons (and transient ones) must not be treated as
    // stored comparisons, otherwise saving the run would persist them.
    run.custom_comparisons = custom_comparisons;
    run.ensure_personal_best();

    if let Some(settings) = child(&root, "AutoSplitterSettings") {
        let inner = serialize_inner_xml(settings)?;
        if !inner.trim().is_empty() {
            run.auto_splitter_settings = Some(inner);
        }
    }

    Ok(run)
}

fn decode_text(bytes: &[u8]) -> Result<String, LssError> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec())
            .map_err(|error| LssError::Invalid(format!("split file is not valid UTF-8: {error}")));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, false);
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|error| LssError::Invalid(format!("split file is not valid UTF-8: {error}")))
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Result<String, LssError> {
    if bytes.len() % 2 != 0 {
        return Err(LssError::Invalid(
            "split file has an incomplete UTF-16 code unit".to_string(),
        ));
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units)
        .map_err(|error| LssError::Invalid(format!("split file is not valid UTF-16: {error}")))
}

fn parse_metadata(element: &XmlElement) -> RunMetadata {
    let mut metadata = RunMetadata::default();

    if let Some(run) = child(element, "Run") {
        metadata.run_id = attribute(run, "id")
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
    }

    if let Some(platform) = child(element, "Platform") {
        metadata.uses_emulator = parse_bool(attribute(platform, "usesEmulator"), false);
        let name = text_trimmed(platform);
        if !name.is_empty() {
            metadata.platform_name = Some(name);
        }
    }

    if let Some(region) = child_text(element, "Region").filter(|region| !region.is_empty()) {
        metadata.region_name = Some(region);
    }

    // `SpeedrunComVariables` was used by older versions / other tools.
    for container in ["Variables", "SpeedrunComVariables"] {
        if let Some(variables) = child(element, container) {
            for variable in children_named(variables, "Variable") {
                if let Some(name) = attribute(variable, "name")
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                {
                    metadata
                        .variables
                        .insert(name.to_string(), text_trimmed(variable));
                }
            }
        }
    }

    if let Some(variables) = child(element, "CustomVariables") {
        for variable in children_named(variables, "Variable") {
            if let Some(name) = attribute(variable, "name")
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                metadata
                    .custom_variables
                    .insert(name.to_string(), text_trimmed(variable));
            }
        }
    }

    metadata
}

fn parse_attempt(element: &XmlElement) -> Attempt {
    let mut time = parse_time(element);
    // Be lenient towards files written by RustSplit versions that wrapped the
    // attempt time in an extra <Time> element.
    if let Some(nested) = child(element, "Time") {
        let nested = parse_time(nested);
        time.real_time = time.real_time.or(nested.real_time);
        time.game_time = time.game_time.or(nested.game_time);
    }

    let started = attribute(element, "started").and_then(parse_datetime);
    let mut ended = attribute(element, "ended").and_then(parse_datetime);
    // LiveSplit <= 1.7 had a bug where an attempt could end before it
    // started.  The reference parser drops such an invalid end timestamp.
    if let (Some(started), Some(ended_time)) = (started, ended) {
        if ended_time < started {
            ended = None;
        }
    }

    let pause_time =
        child_text(element, "PauseTime").and_then(|value| TimeSpan::parse(&value).ok());

    Attempt {
        index: parse_i32_attribute(element, "id").unwrap_or(0),
        time,
        started,
        ended,
        pause_time,
    }
}

fn parse_datetime(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    for format in [
        // LiveSplit's invariant culture format.
        "%m/%d/%Y %H:%M:%S%.f",
        "%m/%d/%Y %H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, format) {
            return Some(Utc.from_utc_datetime(&naive));
        }
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn parse_segment(element: &XmlElement, custom_comparisons: &mut Vec<String>) -> Segment {
    let mut segment = Segment::new(child_text(element, "Name").unwrap_or_default());
    segment.icon = child(element, "Icon").and_then(decode_image);

    // LiveSplit < 1.3 stored the personal best in a separate element.
    if let Some(best) = child(element, "PersonalBestSplitTime") {
        segment.set_personal_best(parse_time(best));
    }

    if let Some(split_times) = child(element, "SplitTimes") {
        for split_time in children_named(split_times, "SplitTime") {
            let Some(name) = attribute(split_time, "name") else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            segment.set_comparison(name, parse_time(split_time));
            if !is_generated_comparison(name)
                && !custom_comparisons.iter().any(|existing| existing == name)
            {
                custom_comparisons.push(name.to_string());
            }
        }
    }

    if let Some(best) = child(element, "BestSegmentTime") {
        segment.best_segment_time = parse_time(best);
    }

    if let Some(history) = child(element, "SegmentHistory") {
        for time in children_named(history, "Time") {
            if let Some(index) = parse_i32_attribute(time, "id") {
                segment.segment_history.insert(index, parse_time(time));
            }
        }
    }

    segment
}

/// Comparisons that LiveSplit computes instead of storing them in the file.
fn is_generated_comparison(name: &str) -> bool {
    GENERATED_COMPARISONS
        .iter()
        .any(|generated| *generated == name)
        || name == NONE
        || name == CURRENT_COMPARISON
        || name == HCP
        || name.starts_with("[Race]")
}

fn parse_time(element: &XmlElement) -> Time {
    let mut time = Time {
        real_time: child_text(element, "RealTime")
            .and_then(|value| TimeSpan::parse(&value).ok()),
        game_time: child_text(element, "GameTime")
            .and_then(|value| TimeSpan::parse(&value).ok()),
    };
    // Files from LiveSplit < 1.4 stored a plain time as the element text.
    if time.is_empty() {
        let legacy = text_trimmed(element);
        if !legacy.is_empty() {
            time.real_time = TimeSpan::parse(&legacy).ok();
        }
    }
    time
}

fn parse_i32_attribute(element: &XmlElement, name: &str) -> Option<i32> {
    attribute(element, name)
        .map(str::trim)
        .and_then(|value| value.parse().ok())
}

/// Serialize the children of an element without the wrapper element, escaping
/// text nodes correctly so the result can be parsed again on save.
fn serialize_inner_xml(element: &XmlElement) -> Result<String, crate::xml::XmlError> {
    let config = xmltree::EmitterConfig::new()
        .perform_indent(true)
        .write_document_declaration(false)
        .indent_string("  ");
    let mut output = Vec::new();
    for child in &element.children {
        match child {
            XMLNode::Element(child) => child.write_with_config(&mut output, config.clone())?,
            XMLNode::Text(value) | XMLNode::CData(value) => {
                escape_xml_text(value, &mut output);
            }
            _ => {}
        }
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn escape_xml_text(value: &str, output: &mut Vec<u8>) {
    for ch in value.chars() {
        match ch {
            '&' => output.extend_from_slice(b"&amp;"),
            '<' => output.extend_from_slice(b"&lt;"),
            '>' => output.extend_from_slice(b"&gt;"),
            _ => {
                let mut buffer = [0; 4];
                output.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
}

fn decode_image(element: &XmlElement) -> Option<Vec<u8>> {
    // Base64 is sometimes wrapped over multiple lines or padded with
    // whitespace by third party tools.
    let encoded = text(element)
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    if encoded.is_empty() {
        return None;
    }
    let data = decode_base64(&encoded)?;
    Some(unwrap_image(data))
}

fn decode_base64(encoded: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

    let unpadded = encoded.trim_end_matches('=');
    STANDARD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(unpadded))
        .or_else(|_| URL_SAFE.decode(encoded))
        .or_else(|_| URL_SAFE_NO_PAD.decode(unpadded))
        .ok()
}

/// Header of the `BinaryFormatter` blob LiveSplit's C# saver writes for a
/// `System.Drawing.Bitmap`.  The actual image data follows a four byte length
/// and a BinaryFormatter byte array marker at offset 161.
const LIVESPLIT_IMAGE_PREFIX: &[u8] = b"\x00\x01\x00\x00\x00\xff\xff\xff\xff\x01\x00\x00\x00\x00\x00\x00\x00\x0c\x02\x00\x00\x00QSystem.Drawing, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b03f5f7f11d50a3a\x05\x01\x00\x00\x00\x15System.Drawing.Bitmap\x01\x00\x00\x00\x04Data\x07\x02\x02\x00\x00\x00\x09\x03\x00\x00\x00\x0f\x03\x00\x00\x00";
const IMAGE_DATA_OFFSET: usize = LIVESPLIT_IMAGE_PREFIX.len() + 5;

fn unwrap_image(data: Vec<u8>) -> Vec<u8> {
    if is_image(&data) {
        return data;
    }

    // Exactly the serialization LiveSplit itself writes.
    if data.len() >= IMAGE_DATA_OFFSET
        && data.starts_with(LIVESPLIT_IMAGE_PREFIX)
        && data.last() == Some(&0x0b)
    {
        return data[IMAGE_DATA_OFFSET..data.len() - 1].to_vec();
    }

    // Some environments use a slightly different BinaryFormatter header.
    // The image itself always follows the 161 byte header and is terminated
    // by the 0x0b end-of-object marker.
    if data.len() > IMAGE_DATA_OFFSET && data.last() == Some(&0x0b) {
        if is_image(&data[IMAGE_DATA_OFFSET..]) {
            return data[IMAGE_DATA_OFFSET..data.len() - 1].to_vec();
        }
        if let Some(start) = find_image_start(&data) {
            return data[start..data.len() - 1].to_vec();
        }
    }

    data
}

fn is_image(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG\r\n\x1a\n")
        || data.starts_with(&[0xFF, 0xD8, 0xFF])
        || data.starts_with(b"GIF87a")
        || data.starts_with(b"GIF89a")
        || data.starts_with(b"BM")
        || (data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP")
        || data.starts_with(b"\x00\x00\x01\x00")
        || data.starts_with(b"II*\x00")
        || data.starts_with(b"MM\x00*")
        || (data.len() >= 12 && &data[4..8] == b"ftyp")
        || data.starts_with(b"qoif")
}

fn find_image_start(data: &[u8]) -> Option<usize> {
    let limit = data.len().saturating_sub(8).min(512);
    (0..=limit).find(|&offset| is_image(&data[offset..]))
}

fn encode_image(data: &[u8]) -> String {
    let mut blob = Vec::with_capacity(IMAGE_DATA_OFFSET + data.len() + 1);
    blob.extend_from_slice(LIVESPLIT_IMAGE_PREFIX);
    blob.extend_from_slice(&(data.len() as u32).to_le_bytes());
    blob.push(0x02);
    blob.extend_from_slice(data);
    blob.push(0x0b);
    base64::engine::general_purpose::STANDARD.encode(blob)
}

pub fn save_run(run: &Run, path: impl AsRef<Path>) -> Result<(), LssError> {
    let xml = save_run_to_string(run)?;
    std::fs::write(path, xml)?;
    Ok(())
}

pub fn save_run_to_string(run: &Run) -> Result<String, LssError> {
    let mut root = XmlElement::new("Run");
    root.attributes
        .insert("version".to_string(), "1.7.0".to_string());

    root.children.push(XMLNode::Element(image_element(
        "GameIcon",
        run.game_icon.as_deref(),
    )));
    root.children.push(XMLNode::Element(element_with_text(
        "GameName",
        run.game_name.clone(),
    )));
    root.children.push(XMLNode::Element(element_with_text(
        "CategoryName",
        run.category_name.clone(),
    )));
    root.children.push(XMLNode::Element(element_with_text(
        "LayoutPath",
        run.layout_path.clone().unwrap_or_default(),
    )));

    root.children
        .push(XMLNode::Element(metadata_element(&run.metadata)));
    root.children.push(XMLNode::Element(element_with_text(
        "Offset",
        run.offset.to_precise_string(),
    )));
    root.children.push(XMLNode::Element(element_with_text(
        "AttemptCount",
        run.attempt_count.to_string(),
    )));

    let mut history = XmlElement::new("AttemptHistory");
    for attempt in &run.attempts {
        history.children.push(XMLNode::Element(attempt_element(attempt)));
    }
    root.children.push(XMLNode::Element(history));

    let mut segments = XmlElement::new("Segments");
    for segment in &run.segments {
        segments
            .children
            .push(XMLNode::Element(segment_element(run, segment)));
    }
    root.children.push(XMLNode::Element(segments));

    let mut auto_splitter = XmlElement::new("AutoSplitterSettings");
    if let Some(settings) = &run.auto_splitter_settings {
        set_inner_xml(&mut auto_splitter, settings)?;
    }
    root.children.push(XMLNode::Element(auto_splitter));

    Ok(xml_to_string(&root)?)
}

fn metadata_element(metadata: &RunMetadata) -> XmlElement {
    let mut element = XmlElement::new("Metadata");

    let mut run = XmlElement::new("Run");
    run.attributes.insert(
        "id".to_string(),
        metadata.run_id.clone().unwrap_or_default(),
    );
    element.children.push(XMLNode::Element(run));

    let mut platform = XmlElement::new("Platform");
    platform.attributes.insert(
        "usesEmulator".to_string(),
        if metadata.uses_emulator {
            "True".to_string()
        } else {
            "False".to_string()
        },
    );
    if let Some(name) = &metadata.platform_name {
        set_text(&mut platform, name.clone());
    }
    element.children.push(XMLNode::Element(platform));

    element.children.push(XMLNode::Element(element_with_text(
        "Region",
        metadata.region_name.clone().unwrap_or_default(),
    )));

    let mut variables = XmlElement::new("Variables");
    for (name, value) in &metadata.variables {
        let mut variable = element_with_text("Variable", value.clone());
        variable
            .attributes
            .insert("name".to_string(), name.clone());
        variables.children.push(XMLNode::Element(variable));
    }
    element.children.push(XMLNode::Element(variables));

    let mut custom_variables = XmlElement::new("CustomVariables");
    for (name, value) in &metadata.custom_variables {
        let mut variable = element_with_text("Variable", value.clone());
        variable
            .attributes
            .insert("name".to_string(), name.clone());
        custom_variables.children.push(XMLNode::Element(variable));
    }
    element.children.push(XMLNode::Element(custom_variables));
    element
}

fn attempt_element(attempt: &Attempt) -> XmlElement {
    let mut element = XmlElement::new("Attempt");
    element
        .attributes
        .insert("id".to_string(), attempt.index.to_string());
    if let Some(started) = attempt.started {
        element.attributes.insert(
            "started".to_string(),
            started.format("%m/%d/%Y %H:%M:%S").to_string(),
        );
        element
            .attributes
            .insert("isStartedSynced".to_string(), "False".to_string());
    }
    if let Some(ended) = attempt.ended {
        element.attributes.insert(
            "ended".to_string(),
            ended.format("%m/%d/%Y %H:%M:%S").to_string(),
        );
        element
            .attributes
            .insert("isEndedSynced".to_string(), "False".to_string());
    }

    // LiveSplit writes the attempt's times directly into <Attempt>, not into
    // a nested <Time> element.
    push_time_fields(&mut element, &attempt.time);
    if let Some(pause_time) = attempt.pause_time {
        element.children.push(XMLNode::Element(element_with_text(
            "PauseTime",
            pause_time.to_precise_string(),
        )));
    }
    element
}

fn segment_element(run: &Run, segment: &Segment) -> XmlElement {
    let mut element = XmlElement::new("Segment");
    element.children.push(XMLNode::Element(element_with_text(
        "Name",
        segment.name.clone(),
    )));
    element
        .children
        .push(XMLNode::Element(image_element("Icon", segment.icon.as_deref())));

    let mut split_times = XmlElement::new("SplitTimes");
    let mut comparison_names = Vec::new();
    for comparison in &run.custom_comparisons {
        let comparison = comparison.trim();
        if comparison.is_empty()
            || is_generated_comparison(comparison)
            || comparison_names
                .iter()
                .any(|existing| existing == comparison)
        {
            continue;
        }
        comparison_names.push(comparison.to_string());
    }
    // The personal best is the only comparison that must always be present.
    if !comparison_names
        .iter()
        .any(|comparison| comparison == PERSONAL_BEST)
    {
        comparison_names.insert(0, PERSONAL_BEST.to_string());
    }
    for comparison in comparison_names {
        let mut time = time_element(&segment.comparison(&comparison), "SplitTime");
        time.attributes
            .insert("name".to_string(), comparison);
        split_times.children.push(XMLNode::Element(time));
    }
    element.children.push(XMLNode::Element(split_times));

    element.children.push(XMLNode::Element(time_element(
        &segment.best_segment_time,
        "BestSegmentTime",
    )));

    let mut history = XmlElement::new("SegmentHistory");
    for (index, time) in &segment.segment_history {
        let mut element = time_element(time, "Time");
        element
            .attributes
            .insert("id".to_string(), index.to_string());
        history.children.push(XMLNode::Element(element));
    }
    element.children.push(XMLNode::Element(history));
    element
}

fn image_element(name: &str, data: Option<&[u8]>) -> XmlElement {
    match data {
        Some(data) if !data.is_empty() => {
            element_with_text(name, encode_image(data))
        }
        _ => XmlElement::new(name),
    }
}

fn time_element(time: &Time, name: &str) -> XmlElement {
    let mut element = XmlElement::new(name);
    push_time_fields(&mut element, time);
    element
}

fn push_time_fields(element: &mut XmlElement, time: &Time) {
    if let Some(real) = time.real_time {
        element.children.push(XMLNode::Element(element_with_text(
            "RealTime",
            real.to_precise_string(),
        )));
    }
    if let Some(game) = time.game_time {
        element.children.push(XMLNode::Element(element_with_text(
            "GameTime",
            game.to_precise_string(),
        )));
    }
}

/// Convenience helper used by the editor: create the XML for an empty
/// auto-splitter settings element.
pub fn default_auto_splitter_settings() -> String {
    String::new()
}
