//! Auto splitter settings stored in the `.lss` `<AutoSplitterSettings>`
//! element, in the same format livesplit-core and LiveSplit's Auto Splitting
//! Runtime component use:
//!
//! ```xml
//! <AutoSplitterSettings>
//!   <Version>1.0</Version>
//!   <ScriptPath>/path/to/splitter.wasm</ScriptPath>
//!   <CustomSettings>
//!     <Setting id="start" type="bool">True</Setting>
//!     <Setting id="route" type="string" value="glitchless" />
//!     <Setting id="nested" type="map"><Setting id="x" type="i64">3</Setting></Setting>
//!   </CustomSettings>
//! </AutoSplitterSettings>
//! ```
//!
//! [`rustsplit_core::Run::auto_splitter_settings`] holds the element's inner
//! XML, which is what [`StoredSettings::parse`] and
//! [`StoredSettings::to_inner_xml`] work with.

use std::sync::Arc;

use livesplit_auto_splitting::settings::{List, Map, Value};
use rustsplit_core::xml::{self, XmlElement};
use xmltree::XMLNode;

const FORMAT_VERSION: &str = "1.0";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredSettings {
    pub script_path: Option<String>,
    pub settings: Map,
}

impl StoredSettings {
    /// Parses the inner XML of `<AutoSplitterSettings>`.
    ///
    /// Unknown elements and settings of unknown types are skipped so that
    /// newer or foreign (ASL) payloads never prevent a run from loading.
    pub fn parse(inner_xml: &str) -> Result<Self, xml::XmlError> {
        if inner_xml.trim().is_empty() {
            return Ok(Self::default());
        }
        let root = xml::parse_xml(format!(
            "<AutoSplitterSettings>{inner_xml}</AutoSplitterSettings>"
        ))?;
        let script_path = xml::child(&root, "ScriptPath")
            .map(xml::text_trimmed)
            .filter(|path| !path.is_empty());
        let settings = xml::child(&root, "CustomSettings")
            .map(parse_map)
            .unwrap_or_default();
        Ok(Self {
            script_path,
            settings,
        })
    }

    /// Serializes the settings into the inner XML of `<AutoSplitterSettings>`.
    pub fn to_inner_xml(&self) -> String {
        let mut root = XmlElement::new("AutoSplitterSettings");
        push(&mut root, xml::element_with_text("Version", FORMAT_VERSION));
        if let Some(path) = &self.script_path {
            push(
                &mut root,
                xml::element_with_text("ScriptPath", path.clone()),
            );
        }
        let mut custom = XmlElement::new("CustomSettings");
        write_map(&mut custom, &self.settings);
        push(&mut root, custom);
        // Only fails for I/O errors, which cannot happen with a Vec.
        xml::inner_xml(&root).unwrap_or_default()
    }

    /// The settings to hand to `script` when it is loaded: settings without a
    /// stored script path are accepted (LiveSplit One does the same), but
    /// settings that belong to another auto splitter are not.
    pub fn settings_for(&self, script: &std::path::Path) -> Option<Map> {
        let belongs = self
            .script_path
            .as_deref()
            .is_none_or(|path| std::path::Path::new(path) == script);
        (belongs && !self.settings.is_empty()).then(|| self.settings.clone())
    }
}

fn push(parent: &mut XmlElement, child: XmlElement) {
    parent.children.push(XMLNode::Element(child));
}

fn setting_elements(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(|node| match node {
        XMLNode::Element(child) if child.name == "Setting" => Some(child),
        _ => None,
    })
}

fn parse_map(element: &XmlElement) -> Map {
    let mut map = Map::new();
    for setting in setting_elements(element) {
        let Some(id) = xml::attribute(setting, "id") else {
            continue;
        };
        if let Some(value) = parse_value(setting) {
            map.insert(Arc::from(id), value);
        }
    }
    map
}

fn parse_list(element: &XmlElement) -> List {
    let mut list = List::new();
    for setting in setting_elements(element) {
        if let Some(value) = parse_value(setting) {
            list.push(value);
        }
    }
    list
}

fn parse_value(setting: &XmlElement) -> Option<Value> {
    let text = || xml::text_trimmed(setting);
    match xml::attribute(setting, "type")? {
        "map" => Some(Value::Map(parse_map(setting))),
        "list" => Some(Value::List(parse_list(setting))),
        "bool" => match text().to_ascii_lowercase().as_str() {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        "i64" => text().parse().ok().map(Value::I64),
        "f64" => text().parse().ok().map(Value::F64),
        "string" => Some(Value::String(Arc::from(
            xml::attribute(setting, "value").unwrap_or_default(),
        ))),
        other => {
            log::warn!("ignoring auto splitter setting of unknown type {other:?}");
            None
        }
    }
}

fn write_map(parent: &mut XmlElement, map: &Map) {
    for (key, value) in map.iter() {
        if let Some(element) = value_element(Some(key), value) {
            push(parent, element);
        }
    }
}

fn write_list(parent: &mut XmlElement, list: &List) {
    for value in list.iter() {
        if let Some(element) = value_element(None, value) {
            push(parent, element);
        }
    }
}

fn value_element(id: Option<&str>, value: &Value) -> Option<XmlElement> {
    let mut element = XmlElement::new("Setting");
    if let Some(id) = id {
        element.attributes.insert("id".to_string(), id.to_string());
    }
    let kind = match value {
        Value::Map(map) => {
            write_map(&mut element, map);
            "map"
        }
        Value::List(list) => {
            write_list(&mut element, list);
            "list"
        }
        Value::Bool(value) => {
            xml::set_text(&mut element, if *value { "True" } else { "False" });
            "bool"
        }
        Value::I64(value) => {
            xml::set_text(&mut element, value.to_string());
            "i64"
        }
        Value::F64(value) => {
            xml::set_text(&mut element, value.to_string());
            "f64"
        }
        Value::String(value) => {
            element
                .attributes
                .insert("value".to_string(), value.to_string());
            "string"
        }
        // `Value` is non-exhaustive. Dropping an unknown kind is better than
        // writing it in a shape that would be misread later.
        _ => return None,
    };
    element
        .attributes
        .insert("type".to_string(), kind.to_string());
    Some(element)
}
