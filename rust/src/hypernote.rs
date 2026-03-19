use hypernote_protocol as hn;

use crate::state::{
    HypernoteAttribute, HypernoteAttributeValueType, HypernoteData, HypernoteDocument,
    HypernoteFormField, HypernoteNode, HypernoteNodeType,
};

pub(crate) fn build_hypernote_data(
    content: &str,
    title: Option<String>,
    default_state: Option<String>,
) -> HypernoteData {
    let parsed = hn::parse_hypernote(content);
    let default_form_state = parse_default_form_state(default_state.as_deref());

    HypernoteData {
        ast_json: parsed.ast_json,
        document: convert_document(parsed.document),
        declared_actions: parsed.declared_actions,
        title,
        default_state,
        default_form_state,
        my_response: None,
        response_tallies: vec![],
        responders: vec![],
    }
}

pub(crate) fn parse_default_form_state(default_state: Option<&str>) -> Vec<HypernoteFormField> {
    let Some(default_state) = default_state.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return vec![];
    };

    let Ok(value) = serde_json::from_str::<serde_json::Value>(default_state) else {
        return vec![];
    };
    let Some(obj) = value.as_object() else {
        return vec![];
    };

    obj.iter()
        .filter_map(|(name, value)| {
            stringify_form_value(value).map(|value| HypernoteFormField {
                name: name.clone(),
                value,
            })
        })
        .collect()
}

fn stringify_form_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(if *value { "true" } else { "false" }.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        other => Some(other.to_string()),
    }
}

fn convert_document(document: hn::HypernoteDocument) -> HypernoteDocument {
    HypernoteDocument {
        root_node_ids: document.root_node_ids,
        nodes: document.nodes.into_iter().map(convert_node).collect(),
    }
}

fn convert_node(node: hn::HypernoteNode) -> HypernoteNode {
    HypernoteNode {
        id: node.id,
        node_type: map_node_type(node.node_type),
        child_ids: node.child_ids,
        value: node.value,
        level: node.level,
        url: node.url,
        lang: node.lang,
        name: node.name,
        raw_type_name: node.raw_type_name,
        checked: node.checked,
        attributes: node.attributes.into_iter().map(convert_attribute).collect(),
    }
}

fn map_node_type(node_type: hn::HypernoteNodeType) -> HypernoteNodeType {
    match node_type {
        hn::HypernoteNodeType::Heading => HypernoteNodeType::Heading,
        hn::HypernoteNodeType::Paragraph => HypernoteNodeType::Paragraph,
        hn::HypernoteNodeType::Strong => HypernoteNodeType::Strong,
        hn::HypernoteNodeType::Emphasis => HypernoteNodeType::Emphasis,
        hn::HypernoteNodeType::CodeInline => HypernoteNodeType::CodeInline,
        hn::HypernoteNodeType::CodeBlock => HypernoteNodeType::CodeBlock,
        hn::HypernoteNodeType::Link => HypernoteNodeType::Link,
        hn::HypernoteNodeType::Image => HypernoteNodeType::Image,
        hn::HypernoteNodeType::ListUnordered => HypernoteNodeType::ListUnordered,
        hn::HypernoteNodeType::ListOrdered => HypernoteNodeType::ListOrdered,
        hn::HypernoteNodeType::ListItem => HypernoteNodeType::ListItem,
        hn::HypernoteNodeType::Blockquote => HypernoteNodeType::Blockquote,
        hn::HypernoteNodeType::Hr => HypernoteNodeType::Hr,
        hn::HypernoteNodeType::HardBreak => HypernoteNodeType::HardBreak,
        hn::HypernoteNodeType::Text => HypernoteNodeType::Text,
        hn::HypernoteNodeType::MdxJsxElement => HypernoteNodeType::MdxJsxElement,
        hn::HypernoteNodeType::MdxJsxSelfClosing => HypernoteNodeType::MdxJsxSelfClosing,
        hn::HypernoteNodeType::Unsupported => HypernoteNodeType::Unsupported,
    }
}

fn convert_attribute(attribute: hn::HypernoteAttribute) -> HypernoteAttribute {
    let (value_type, value) = convert_attribute_value(attribute.value);
    HypernoteAttribute {
        name: attribute.name,
        value_type,
        value,
    }
}

fn convert_attribute_value(
    value: hn::HypernoteAttributeValue,
) -> (HypernoteAttributeValueType, Option<String>) {
    match value {
        hn::HypernoteAttributeValue::String(value) => {
            (HypernoteAttributeValueType::String, Some(value))
        }
        hn::HypernoteAttributeValue::Number(value) => {
            (HypernoteAttributeValueType::Number, Some(value.to_string()))
        }
        hn::HypernoteAttributeValue::InvalidNumber(value) => {
            (HypernoteAttributeValueType::InvalidNumber, Some(value))
        }
        hn::HypernoteAttributeValue::Boolean(value) => (
            HypernoteAttributeValueType::Boolean,
            Some(if value { "true" } else { "false" }.to_string()),
        ),
        hn::HypernoteAttributeValue::Expression(value) => {
            (HypernoteAttributeValueType::Expression, Some(value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_node<'a>(
        document: &'a HypernoteDocument,
        node_type: HypernoteNodeType,
        name: Option<&str>,
    ) -> &'a HypernoteNode {
        document
            .nodes
            .iter()
            .find(|node| {
                node.node_type == node_type
                    && match name {
                        Some(name) => node.name.as_deref() == Some(name),
                        None => true,
                    }
            })
            .expect("node exists")
    }

    #[test]
    fn build_hypernote_data_dual_writes_document_and_actions() {
        let hypernote = build_hypernote_data(
            "# Poll\n\n<SubmitButton action=\"yes\">Yes</SubmitButton>\n<SubmitButton action=\"no\" />",
            Some("Lunch".to_string()),
            Some(r#"{"name":"Ada","subscribed":true}"#.to_string()),
        );

        assert!(!hypernote.ast_json.is_empty());
        assert_eq!(hypernote.declared_actions, vec!["yes", "no"]);
        assert_eq!(
            hypernote.default_form_state,
            vec![
                HypernoteFormField {
                    name: "name".to_string(),
                    value: "Ada".to_string(),
                },
                HypernoteFormField {
                    name: "subscribed".to_string(),
                    value: "true".to_string(),
                },
            ]
        );

        let submit = find_node(
            &hypernote.document,
            HypernoteNodeType::MdxJsxElement,
            Some("SubmitButton"),
        );
        assert_eq!(
            submit.attributes,
            vec![HypernoteAttribute {
                name: "action".to_string(),
                value_type: HypernoteAttributeValueType::String,
                value: Some("yes".to_string()),
            }]
        );
    }

    #[test]
    fn build_hypernote_data_preserves_markdown_fields() {
        let hypernote = build_hypernote_data(
            "# Title\n\nParagraph with [**bold** label](https://example.com) and `code`.\n\n```rust\nfn main() {}\n```",
            None,
            None,
        );

        let heading = find_node(&hypernote.document, HypernoteNodeType::Heading, None);
        assert_eq!(heading.level, Some(1));

        let link = find_node(&hypernote.document, HypernoteNodeType::Link, None);
        assert_eq!(link.url.as_deref(), Some("https://example.com"));
        assert_eq!(link.child_ids.len(), 2);

        let code_block = find_node(&hypernote.document, HypernoteNodeType::CodeBlock, None);
        assert_eq!(code_block.lang.as_deref(), Some("rust"));
        assert_eq!(code_block.value.as_deref(), Some("fn main() {}\n"));
    }

    #[test]
    fn build_hypernote_data_projects_typed_attribute_values() {
        let hypernote = build_hypernote_data(
            "<Widget label=\"Fish &amp; Chips\" count=4 enabled visible=false expr={state.count} />",
            None,
            None,
        );

        let widget = find_node(
            &hypernote.document,
            HypernoteNodeType::MdxJsxSelfClosing,
            Some("Widget"),
        );
        assert_eq!(
            widget.attributes,
            vec![
                HypernoteAttribute {
                    name: "label".to_string(),
                    value_type: HypernoteAttributeValueType::String,
                    value: Some("Fish & Chips".to_string()),
                },
                HypernoteAttribute {
                    name: "count".to_string(),
                    value_type: HypernoteAttributeValueType::Number,
                    value: Some("4".to_string()),
                },
                HypernoteAttribute {
                    name: "enabled".to_string(),
                    value_type: HypernoteAttributeValueType::Boolean,
                    value: Some("true".to_string()),
                },
                HypernoteAttribute {
                    name: "visible".to_string(),
                    value_type: HypernoteAttributeValueType::Boolean,
                    value: Some("false".to_string()),
                },
                HypernoteAttribute {
                    name: "expr".to_string(),
                    value_type: HypernoteAttributeValueType::Expression,
                    value: Some("state.count".to_string()),
                },
            ]
        );
    }

    #[test]
    fn parse_default_form_state_handles_scalars_and_objects() {
        assert_eq!(
            parse_default_form_state(Some(
                r#"{"name":"Ada","subscribed":true,"count":3,"meta":{"x":1}}"#
            )),
            vec![
                HypernoteFormField {
                    name: "count".to_string(),
                    value: "3".to_string(),
                },
                HypernoteFormField {
                    name: "meta".to_string(),
                    value: r#"{"x":1}"#.to_string(),
                },
                HypernoteFormField {
                    name: "name".to_string(),
                    value: "Ada".to_string(),
                },
                HypernoteFormField {
                    name: "subscribed".to_string(),
                    value: "true".to_string(),
                },
            ]
        );
    }
}
