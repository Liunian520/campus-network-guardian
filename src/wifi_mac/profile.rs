//! A bounded, namespace-aware edit of one WLAN profile setting.

use quick_xml::{
    escape::{resolve_xml_entity, unescape},
    events::{BytesStart, Event},
    name::ResolveResult,
    NsReader,
};
use std::{collections::HashSet, ops::Range};

const V1: &str = "http://www.microsoft.com/networking/WLAN/profile/v1";
const V3: &str = "http://www.microsoft.com/networking/WLAN/profile/v3";
const MAX_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_ATTRIBUTES: usize = 256;
const MAX_NAMESPACE_BYTES: usize = 4096;
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Element {
    Root,
    Mac,
    Enable,
    Other,
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

struct Profile {
    disabled: bool,
    edit: Option<Edit>,
}

pub(super) fn disable_randomization(xml: &str) -> Result<String, ()> {
    let parsed = inspect(xml)?;
    let Some(edit) = parsed.edit else {
        return Ok(xml.to_owned());
    };
    let mut result = String::with_capacity(
        xml.len() - (edit.range.end - edit.range.start) + edit.replacement.len(),
    );
    result.push_str(xml.get(..edit.range.start).ok_or(())?);
    result.push_str(&edit.replacement);
    result.push_str(xml.get(edit.range.end..).ok_or(())?);
    // Never return an edit unless its resulting namespace and value verify.
    if !inspect(&result)?.disabled {
        return Err(());
    }
    Ok(result)
}

pub(super) fn randomization_disabled(xml: &str) -> Result<bool, ()> {
    Ok(inspect(xml)?.disabled)
}

fn xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn xml_character(c: char) -> bool {
    matches!(c, '\t' | '\r' | '\n' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

fn name_start(c: char) -> bool {
    matches!(c, 'A'..='Z' | '_' | 'a'..='z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}'
        | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}'
        | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}'
        | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}'
        | '\u{10000}'..='\u{effff}')
}

fn valid_name(name: &str) -> bool {
    let mut parts = name.split(':');
    let valid_part = |part: &str| {
        let mut characters = part.chars();
        characters.next().is_some_and(name_start)
            && characters.all(|c| {
                name_start(c)
                    || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
            })
    };
    valid_part(parts.next().unwrap_or_default())
        && parts.next().is_none_or(valid_part)
        && parts.next().is_none()
}

fn attributes(reader: &NsReader<&[u8]>, start: &BytesStart<'_>) -> Result<(), ()> {
    let mut expanded_names = HashSet::new();
    for (index, attribute) in start.attributes().enumerate() {
        if index >= MAX_ATTRIBUTES {
            return Err(());
        }
        let attribute = attribute.map_err(|_| ())?;
        if !valid_name(attribute.key.0) || attribute.value.contains('<') {
            return Err(());
        }
        let value = unescape(&attribute.value).map_err(|_| ())?;
        if !value.chars().all(xml_character) {
            return Err(());
        }
        if attribute.key.0 == "xmlns" || attribute.key.0.starts_with("xmlns:") {
            if attribute.value.len() > MAX_NAMESPACE_BYTES {
                return Err(());
            }
            match attribute.key.0.strip_prefix("xmlns:") {
                None if matches!(value.as_ref(), XML_NAMESPACE | XMLNS_NAMESPACE) => return Err(()),
                Some("xmlns") => return Err(()),
                Some("xml") if value != XML_NAMESPACE => return Err(()),
                Some(prefix)
                    if prefix != "xml"
                        && (value.is_empty()
                            || matches!(value.as_ref(), XML_NAMESPACE | XMLNS_NAMESPACE)) =>
                {
                    return Err(())
                }
                _ => {}
            }
            continue;
        }
        let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
        let namespace = match namespace {
            ResolveResult::Bound(namespace) => Some(unescape(namespace.0).map_err(|_| ())?),
            ResolveResult::Unbound => None,
            ResolveResult::Unknown(_) => return Err(()),
        };
        if !expanded_names.insert((namespace, local.into_inner())) {
            return Err(());
        }
    }
    Ok(())
}

fn inspect(xml: &str) -> Result<Profile, ()> {
    if xml.len() > MAX_BYTES || !xml.chars().all(xml_character) {
        return Err(());
    }
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().enable_all_checks(true);
    reader.resolver_mut().set_max_namespace_bindings(256);
    // Reader positions exclude a consumed UTF-8 BOM; slices address the
    // original string, including that prefix.
    let bom_bytes = if xml.starts_with('\u{feff}') { 3 } else { 0 };
    let mut stack = Vec::new();
    let mut root_seen = false;
    let mut root_finished = false;
    let mut declaration_seen = false;
    let mut root_insertion = None;
    let mut mac_insertion = None;
    let mut mac_seen = false;
    let mut enable_seen = false;
    let mut enable_start = 0;
    let mut disabled = false;
    let mut value_edit = None;
    loop {
        let before = usize::try_from(reader.buffer_position()).map_err(|_| ())? + bom_bytes;
        let event = reader.read_event().map_err(|_| ())?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                if root_finished || stack.len() >= MAX_DEPTH || !valid_name(start.name().0) {
                    return Err(());
                }
                attributes(&reader, start)?;
                let empty = matches!(event, Event::Empty(_));
                let after = usize::try_from(reader.buffer_position()).map_err(|_| ())? + bom_bytes;
                let (namespace, _) = reader.resolver().resolve_element(start.name());
                let namespace = match namespace {
                    ResolveResult::Bound(namespace) => Some(namespace.0),
                    ResolveResult::Unbound => None,
                    ResolveResult::Unknown(_) => return Err(()),
                };
                let kind = if stack.is_empty() {
                    if root_seen
                        || start.local_name().into_inner() != "WLANProfile"
                        || namespace != Some(V1)
                    {
                        return Err(());
                    }
                    root_seen = true;
                    Element::Root
                } else {
                    if stack.last() == Some(&Element::Enable) {
                        return Err(());
                    }
                    match start.local_name().into_inner() {
                        "MacRandomization" => {
                            if stack.len() != 1 || namespace != Some(V3) || mac_seen {
                                return Err(());
                            }
                            mac_seen = true;
                            mac_insertion = Some(if empty {
                                let range = after.checked_sub(2).ok_or(())?..after;
                                if xml.get(range.clone()) != Some("/>") {
                                    return Err(());
                                }
                                Edit {
                                    range,
                                    replacement: format!(
                                        ">{}</{}>",
                                        enable_element(),
                                        start.name().0
                                    ),
                                }
                            } else {
                                Edit {
                                    range: after..after,
                                    replacement: enable_element(),
                                }
                            });
                            Element::Mac
                        }
                        "enableRandomization" => {
                            if stack.len() != 2
                                || stack.last() != Some(&Element::Mac)
                                || namespace != Some(V3)
                                || enable_seen
                                || empty
                            {
                                return Err(());
                            }
                            enable_seen = true;
                            enable_start = after;
                            Element::Enable
                        }
                        _ => Element::Other,
                    }
                };
                if empty {
                    if kind == Element::Root {
                        root_finished = true;
                        let range = after.checked_sub(2).ok_or(())?..after;
                        if xml.get(range.clone()) != Some("/>") {
                            return Err(());
                        }
                        root_insertion = Some(Edit {
                            range,
                            replacement: format!(">{}</{}>", mac_element(), start.name().0),
                        });
                    }
                } else {
                    stack.push(kind);
                }
            }
            Event::End(_) => match stack.pop().ok_or(())? {
                Element::Root => {
                    root_finished = true;
                    root_insertion = Some(Edit {
                        range: before..before,
                        replacement: mac_element(),
                    });
                }
                Element::Enable => {
                    let content = xml.get(enable_start..before).ok_or(())?;
                    let value = content.trim_matches(xml_space);
                    match value {
                        "false" | "0" => disabled = true,
                        "true" | "1" => {
                            let value_start = enable_start + content.len()
                                - content.trim_start_matches(xml_space).len();
                            value_edit = Some(Edit {
                                range: value_start..value_start + value.len(),
                                replacement: "false".to_owned(),
                            });
                        }
                        _ => return Err(()),
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if (stack.is_empty() || matches!(stack.last(), Some(Element::Root | Element::Mac)))
                    && !text.chars().all(xml_space)
                {
                    return Err(());
                }
                if text.contains("]]>") {
                    return Err(());
                }
            }
            Event::GeneralRef(reference) => {
                if stack.is_empty()
                    || matches!(
                        stack.last(),
                        Some(Element::Enable | Element::Root | Element::Mac)
                    )
                {
                    return Err(());
                }
                match reference.resolve_char_ref().map_err(|_| ())? {
                    Some(character) if xml_character(character) => {}
                    Some(_) => return Err(()),
                    None if resolve_xml_entity(&reference).is_some() => {}
                    None => return Err(()),
                }
            }
            Event::Comment(_) | Event::PI(_) | Event::CData(_) => {
                if stack.last() == Some(&Element::Enable)
                    || (matches!(event, Event::CData(_)) && stack.is_empty())
                {
                    return Err(());
                }
                if let Event::PI(instruction) = event {
                    let target = instruction.target();
                    if !valid_name(target) || target.eq_ignore_ascii_case("xml") {
                        return Err(());
                    }
                }
            }
            Event::Decl(declaration) => {
                if declaration_seen
                    || root_seen
                    || !xml
                        .get(..before)
                        .ok_or(())?
                        .trim_start_matches('\u{feff}')
                        .is_empty()
                    || declaration.version().map_err(|_| ())?.as_ref() != "1.0"
                {
                    return Err(());
                }
                declaration_seen = true;
                let declaration = BytesStart::from_content(declaration.as_ref(), 3);
                attributes(&reader, &declaration)?;
                for attribute in declaration.attributes() {
                    let attribute = attribute.map_err(|_| ())?;
                    match attribute.key.0 {
                        "version" => {}
                        "encoding"
                            if !attribute.value.is_empty()
                                && attribute.value.chars().enumerate().all(|(index, c)| {
                                    c.is_ascii_alphabetic()
                                        || (index > 0 && matches!(c, '0'..='9' | '.' | '_' | '-'))
                                }) => {}
                        "standalone" if matches!(attribute.value.as_ref(), "yes" | "no") => {}
                        _ => return Err(()),
                    }
                }
            }
            Event::DocType(_) => return Err(()),
            Event::Eof => {
                if !root_seen || !root_finished || !stack.is_empty() {
                    return Err(());
                }
                return Ok(Profile {
                    disabled,
                    edit: if enable_seen {
                        value_edit
                    } else if mac_seen {
                        mac_insertion
                    } else {
                        root_insertion
                    },
                });
            }
        }
    }
}

fn enable_element() -> String {
    format!("<enableRandomization xmlns=\"{V3}\">false</enableRandomization>")
}

fn mac_element() -> String {
    format!("<MacRandomization xmlns=\"{V3}\"><enableRandomization>false</enableRandomization></MacRandomization>")
}

#[cfg(test)]
mod tests {
    use super::*;
    const V1: &str = "http://www.microsoft.com/networking/WLAN/profile/v1";
    const V3: &str = "http://www.microsoft.com/networking/WLAN/profile/v3";

    fn profile(body: &str) -> String {
        format!("<WLANProfile xmlns=\"{V1}\">{body}</WLANProfile>")
    }
    fn mac(body: &str) -> String {
        format!("<MacRandomization xmlns=\"{V3}\">{body}</MacRandomization>")
    }

    #[test]
    fn changes_only_the_true_value_and_preserves_opaque_profile_bytes() {
        let original = format!("<?xml version='1.0'?>\r\n<WLANProfile xmlns='{V1}' xmlns:x='urn:custom'>\r\n<x:custom a='&amp;'>中文&#32;</x:custom><keyMaterial>FAKE-OPAQUE-KEY</keyMaterial>{}</WLANProfile>", mac("<enableRandomization> \r\ntrue\t </enableRandomization><randomizeEveryday>true</randomizeEveryday><randomizationSeed>42</randomizationSeed>"));
        let expected = original.replacen("true\t", "false\t", 1);
        assert!(!randomization_disabled(&original).unwrap());
        let changed = disable_randomization(&original).unwrap();
        assert_eq!(changed, expected);
        assert!(randomization_disabled(&changed).unwrap());
    }

    #[test]
    fn false_and_zero_are_byte_identical_noops() {
        for value in ["false", "0", " \n0\t "] {
            let original = profile(&mac(&format!(
                "<enableRandomization>{value}</enableRandomization>"
            )));
            assert!(randomization_disabled(&original).unwrap());
            assert_eq!(disable_randomization(&original).unwrap(), original);
        }
    }

    #[test]
    fn numeric_true_and_prefixed_namespace_are_supported() {
        let original = format!("<w:WLANProfile xmlns:w='{V1}' xmlns:m='{V3}'><m:MacRandomization><m:enableRandomization>1</m:enableRandomization></m:MacRandomization></w:WLANProfile>");
        let changed = disable_randomization(&original).unwrap();
        assert_eq!(changed, original.replace(">1<", ">false<"));
        assert!(randomization_disabled(&changed).unwrap());
    }

    #[test]
    fn missing_container_is_inserted_in_the_correct_namespace() {
        let original =
            format!("<w:WLANProfile xmlns:w='{V1}'><w:name>Fake</w:name></w:WLANProfile>");
        assert!(!randomization_disabled(&original).unwrap());
        let changed = disable_randomization(&original).unwrap();
        assert!(changed.starts_with(original.split("</w:WLANProfile>").next().unwrap()));
        assert!(changed.ends_with("</w:WLANProfile>"));
        assert!(randomization_disabled(&changed).unwrap());
        assert_eq!(disable_randomization(&changed).unwrap(), changed);
    }

    #[test]
    fn missing_enable_is_inserted_before_other_container_children() {
        let original = format!("<WLANProfile xmlns='{V1}' xmlns:m='{V3}'><m:MacRandomization custom='keep' xmlns='urn:other'><m:randomizeEveryday>true</m:randomizeEveryday><m:randomizationSeed>7</m:randomizationSeed></m:MacRandomization></WLANProfile>");
        let changed = disable_randomization(&original).unwrap();
        assert!(randomization_disabled(&changed).unwrap());
        assert!(changed.contains("custom='keep' xmlns='urn:other'>"));
        assert!(
            changed.find("enableRandomization").unwrap()
                < changed.find("randomizeEveryday").unwrap()
        );
        assert!(changed.contains("<m:randomizationSeed>7</m:randomizationSeed>"));
    }

    #[test]
    fn empty_containers_and_empty_prefixed_root_can_be_expanded() {
        for original in [
            profile(&format!("<m:MacRandomization xmlns:m='{V3}' keep='yes' />")),
            profile(&format!(
                "<MacRandomization xmlns='{V3}'></MacRandomization>"
            )),
            format!("<w:WLANProfile xmlns:w='{V1}' />"),
        ] {
            let changed = disable_randomization(&original).unwrap();
            assert!(randomization_disabled(&changed).unwrap());
            if original.contains("keep='yes'") {
                assert!(changed.contains("keep='yes' "));
                assert!(changed.contains("</m:MacRandomization>"));
            }
        }
    }

    #[test]
    fn invalid_or_ambiguous_profiles_never_authorize_or_get_rewritten() {
        let valid = profile(&mac("<enableRandomization>false</enableRandomization>"));
        let wrong_ns = profile(
            "<MacRandomization><enableRandomization>false</enableRandomization></MacRandomization>",
        );
        let cases = vec![
            String::new(), "<WLANProfile/>".into(), format!("<Other xmlns='{V1}'/>"),
            format!("{valid}{valid}"), format!("{valid}garbage"), valid.replace("</WLANProfile>", ""),
            format!("<!DOCTYPE WLANProfile [<!ENTITY a 'false'>]>{valid}"),
            profile(&format!("{}{}", mac(""), mac(""))),
            profile(&mac("<enableRandomization>false</enableRandomization><enableRandomization>0</enableRandomization>")),
            wrong_ns,
            profile(&mac("<enableRandomization xmlns='urn:wrong'>false</enableRandomization>")),
            profile(&mac("<enableRandomization><nested>false</nested></enableRandomization>")),
            profile(&mac("<enableRandomization>FALSE</enableRandomization>")),
            profile(&mac("<enableRandomization/>")),
            profile(&mac("<enableRandomization><![CDATA[false]]></enableRandomization>")),
            profile(&mac("<enableRandomization><!--split-->false</enableRandomization>")),
            profile(&mac("<enableRandomization>&#48;</enableRandomization>")),
            profile("<other><enableRandomization xmlns='http://www.microsoft.com/networking/WLAN/profile/v3'>false</enableRandomization></other>"),
            profile("<unknown:field/>"),
            profile("<field a='1' a='2'/>"),
            profile("<field>&undefined;</field>"),
            profile("<field>\u{1}</field>"),
            profile("<!--bad--comment-->"),
        ];
        for candidate in cases {
            assert!(randomization_disabled(&candidate).is_err());
            assert!(disable_randomization(&candidate).is_err());
        }
    }

    #[test]
    fn namespace_rebinding_and_duplicate_expanded_attributes_are_rejected() {
        for body in [
            format!("<m:MacRandomization xmlns:m='{V3}'><m:enableRandomization xmlns:m='urn:wrong'>false</m:enableRandomization></m:MacRandomization>"),
            format!("<MacRandomization xmlns='{V3}' xmlns='{V3}'/>"),
            "<x xmlns:a='urn:same' xmlns:b='urn:same' a:id='1' b:id='2'/>".into(),
            "<x xmlns:a='urn:any' a:bad:name='1'/>".into(),
        ] {
            assert!(disable_randomization(&profile(&body)).is_err());
        }
    }

    #[test]
    fn input_size_and_depth_are_bounded() {
        let large = profile(&" ".repeat(1024 * 1024));
        assert!(disable_randomization(&large).is_err());
        let deep = profile(&format!("{}{}", "<x>".repeat(65), "</x>".repeat(65)));
        assert!(disable_randomization(&deep).is_err());
    }

    #[test]
    fn windows_unicode_declarations_bom_and_unrelated_cdata_are_preserved() {
        let original = format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n<!--keep-->\r\n<WLANProfile xmlns='{V1}'><extension><![CDATA[<fake>true</fake>]]></extension>{}</WLANProfile>\r\n",
            mac("<enableRandomization>true</enableRandomization>")
        );
        let changed = disable_randomization(&original).unwrap();
        assert_eq!(
            changed,
            original.replace(
                "<enableRandomization>true</enableRandomization>",
                "<enableRandomization>false</enableRandomization>"
            )
        );
        assert!(randomization_disabled(&changed).unwrap());
    }

    #[test]
    fn wrong_namespace_lookalikes_never_override_a_real_disabled_value() {
        let correct = mac("<enableRandomization>false</enableRandomization>");
        for wrong in [
            "<x:MacRandomization xmlns:x='urn:custom'/>",
            "<x:enableRandomization xmlns:x='urn:custom'>false</x:enableRandomization>",
            "<enableRandomization xmlns=''>false</enableRandomization>",
        ] {
            let original = profile(&format!("{correct}{wrong}"));
            assert!(randomization_disabled(&original).is_err());
            assert!(disable_randomization(&original).is_err());
        }
    }

    #[test]
    fn xml_document_and_attribute_errors_are_not_silently_repaired() {
        for original in [
            format!(" <?xml version='1.0'?>{}", profile("")),
            format!("<?xml version='1.0' version='1.0'?>{}", profile("")),
            format!("<?xml version='1.0'?><?xml version='1.0'?>{}", profile("")),
            profile("<x a='&#1;'/>"),
            profile("<x a='unescaped < text'/>"),
            profile("<:bad/>"),
            profile("<x xmlns:xml='urn:bad'/>"),
            profile("<x xmlns:xmlns='urn:bad'/>"),
            profile("<x xmlns='http://www.w3.org/2000/xmlns/'/>"),
            profile("<x xmlns='http://www.w3.org/XML/1998/namespace'/>"),
            profile("<x xmlns:a='urn:same' xmlns:b='urn:s&#97;me' a:id='1' b:id='2'/>"),
        ] {
            assert!(randomization_disabled(&original).is_err());
            assert!(disable_randomization(&original).is_err());
        }
    }

    #[test]
    fn namespace_attributes_are_bounded_without_rewriting_large_opaque_data() {
        let attributes = (0..257).map(|n| format!(" a{n}='x'")).collect::<String>();
        assert!(disable_randomization(&profile(&format!("<x{attributes}/>"))).is_err());
        let secret = "FAKE-KEY-ONLY-".repeat(20_000);
        let original = profile(&format!(
            "<keyMaterial>{secret}</keyMaterial>{}",
            mac("<enableRandomization>0</enableRandomization>")
        ));
        assert_eq!(disable_randomization(&original).unwrap(), original);
    }
}
