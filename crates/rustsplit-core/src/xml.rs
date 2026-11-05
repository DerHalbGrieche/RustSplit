//! A very small convenience wrapper around `xmltree` used by the split file
//! and layout parsers.

use std::io::Cursor;

use thiserror::Error;
use xmltree::{Element, XMLNode};

pub type XmlElement = Element;

#[derive(Debug, Error)]
pub enum XmlError {
    #[error("could not parse XML: {0}")]
    Parse(#[from] xmltree::ParseError),
    #[error("could not write XML: {0}")]
    Write(#[from] xmltree::Error),
    #[error("expected element <{0}>")]
    MissingElement(String),
}

/// Parse an XML document into a tree.
pub fn parse_xml(data: impl AsRef<[u8]>) -> Result<XmlElement, XmlError> {
    Ok(Element::parse(Cursor::new(data.as_ref()))?)
}

/// Serialize an XML tree with an XML declaration and pretty indentation.
pub fn xml_to_string(element: &XmlElement) -> Result<String, XmlError> {
    let config = xmltree::EmitterConfig::new()
        .perform_indent(true)
        .write_document_declaration(false)
        .indent_string("  ");
    let mut output = Vec::new();
    element.write_with_config(&mut output, config)?;
    let body = String::from_utf8_lossy(&output);
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}",
        body.trim_start()
    ))
}

/// Text content of an element, concatenating text and CDATA children.
pub fn text(element: &XmlElement) -> String {
    let mut result = String::new();
    for child in &element.children {
        match child {
            XMLNode::Text(text) | XMLNode::CData(text) => result.push_str(text),
            _ => {}
        }
    }
    result
}

/// Trimmed text content of an element.
pub fn text_trimmed(element: &XmlElement) -> String {
    text(element).trim().to_string()
}

/// Replace the text content of an element.
pub fn set_text(element: &mut XmlElement, value: impl Into<String>) {
    element
        .children
        .retain(|child| !matches!(child, XMLNode::Text(_) | XMLNode::CData(_)));
    element.children.push(XMLNode::Text(value.into()));
}

/// First direct child element with the given tag name.
pub fn child<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    element.children.iter().find_map(|node| match node {
        XMLNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

/// Mutable first direct child element with the given tag name.
pub fn child_mut<'a>(element: &'a mut XmlElement, name: &str) -> Option<&'a mut XmlElement> {
    element.children.iter_mut().find_map(|node| match node {
        XMLNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

/// All direct child elements with the given tag name.
pub fn children_named<'a>(
    element: &'a XmlElement,
    name: &'a str,
) -> impl Iterator<Item = &'a XmlElement> + 'a {
    element.children.iter().filter_map(move |node| match node {
        XMLNode::Element(child) if child.name == name => Some(child),
        _ => None,
    })
}

/// Text of a direct child element.
pub fn child_text(element: &XmlElement, name: &str) -> Option<String> {
    child(element, name).map(text_trimmed)
}

/// Attribute value by name.
pub fn attribute<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element.attributes.get(name).map(String::as_str)
}

/// Parse a boolean the way LiveSplit's settings helper does.
pub fn parse_bool(value: Option<&str>, default: bool) -> bool {
    match value.map(str::trim) {
        Some("True") | Some("true") | Some("1") => true,
        Some("False") | Some("false") | Some("0") => false,
        _ => default,
    }
}

/// Parse an integer, falling back to a default.
pub fn parse_i32(value: Option<&str>, default: i32) -> i32 {
    value
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

/// Parse a float, falling back to a default.
pub fn parse_f32(value: Option<&str>, default: f32) -> f32 {
    value
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

/// Create an element with text content.
pub fn element_with_text(name: &str, value: impl Into<String>) -> XmlElement {
    let mut element = XmlElement::new(name);
    set_text(&mut element, value);
    element
}

/// Inner XML of an element (its children serialized without a wrapper).
pub fn inner_xml(element: &XmlElement) -> Result<String, XmlError> {
    let config = xmltree::EmitterConfig::new()
        .perform_indent(true)
        .write_document_declaration(false)
        .indent_string("  ");
    let mut output = Vec::new();
    for child in &element.children {
        match child {
            XMLNode::Element(child) => child.write_with_config(&mut output, config.clone())?,
            XMLNode::Text(text) => output.extend_from_slice(text.as_bytes()),
            XMLNode::CData(text) => {
                output.extend_from_slice(b"<![CDATA[");
                output.extend_from_slice(text.as_bytes());
                output.extend_from_slice(b"]]>");
            }
            _ => {}
        }
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Replace an element's children with parsed inner XML wrapped in a root tag.
pub fn set_inner_xml(element: &mut XmlElement, inner: &str) -> Result<(), XmlError> {
    element.children.clear();
    if inner.trim().is_empty() {
        return Ok(());
    }
    let wrapped = format!("<root>{inner}</root>");
    let root = parse_xml(wrapped)?;
    element.children = root.children;
    Ok(())
}
