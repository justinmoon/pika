use std::collections::{BTreeSet, HashMap};

use hypernote_mdx::ast::{Ast, NodeData, NodeIndex, NodeTag};
use hypernote_mdx::semantic::JsxAttributeValue as MdxJsxAttributeValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedHypernote {
    pub ast_json: String,
    pub document: HypernoteDocument,
    pub declared_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HypernoteDocument {
    pub root_node_ids: Vec<u32>,
    pub nodes: Vec<HypernoteNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HypernoteNode {
    pub id: u32,
    pub node_type: HypernoteNodeType,
    pub child_ids: Vec<u32>,
    pub value: Option<String>,
    pub level: Option<u8>,
    pub url: Option<String>,
    pub lang: Option<String>,
    pub name: Option<String>,
    pub raw_type_name: Option<String>,
    pub checked: Option<bool>,
    pub attributes: Vec<HypernoteAttribute>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HypernoteNodeType {
    Heading,
    Paragraph,
    Strong,
    Emphasis,
    CodeInline,
    CodeBlock,
    Link,
    Image,
    ListUnordered,
    ListOrdered,
    ListItem,
    Blockquote,
    Hr,
    HardBreak,
    Text,
    MdxJsxElement,
    MdxJsxSelfClosing,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HypernoteAttribute {
    pub name: String,
    pub value: HypernoteAttributeValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HypernoteAttributeValue {
    String(String),
    Number(f64),
    InvalidNumber(String),
    Boolean(bool),
    Expression(String),
}

pub fn parse_hypernote(content: &str) -> ParsedHypernote {
    let ast = hypernote_mdx::parse(content);
    let ast_json = hypernote_mdx::serialize_tree(&ast);
    let document = HypernoteDocumentBuilder::new(&ast).build();
    let declared_actions = extract_submit_actions(&document);

    ParsedHypernote {
        ast_json,
        document,
        declared_actions,
    }
}

pub fn extract_submit_actions(document: &HypernoteDocument) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    for node in &document.nodes {
        if !matches!(
            node.node_type,
            HypernoteNodeType::MdxJsxElement | HypernoteNodeType::MdxJsxSelfClosing
        ) {
            continue;
        }
        if node.name.as_deref() != Some("SubmitButton") {
            continue;
        }

        let Some(action) = node
            .attributes
            .iter()
            .find(|attr| attr.name == "action")
            .and_then(|attr| match &attr.value {
                HypernoteAttributeValue::String(value) => Some(value.as_str()),
                _ => None,
            })
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        if seen.insert(action.to_string()) {
            out.push(action.to_string());
        }
    }

    out
}

struct HypernoteDocumentBuilder<'a> {
    ast: &'a Ast,
    nodes: Vec<HypernoteNode>,
    ids: HashMap<NodeIndex, u32>,
}

impl<'a> HypernoteDocumentBuilder<'a> {
    fn new(ast: &'a Ast) -> Self {
        Self {
            ast,
            nodes: Vec::new(),
            ids: HashMap::new(),
        }
    }

    fn build(mut self) -> HypernoteDocument {
        let root_node_ids = self
            .document_index()
            .map(|idx| {
                self.ast
                    .children(idx)
                    .iter()
                    .copied()
                    .map(|child| self.convert_node(child))
                    .collect()
            })
            .unwrap_or_default();

        HypernoteDocument {
            root_node_ids,
            nodes: self.nodes,
        }
    }

    fn document_index(&self) -> Option<NodeIndex> {
        self.ast
            .nodes
            .iter()
            .enumerate()
            .find_map(|(idx, node)| (node.tag == NodeTag::Document).then_some(idx as NodeIndex))
    }

    fn convert_node(&mut self, node_idx: NodeIndex) -> u32 {
        if let Some(id) = self.ids.get(&node_idx) {
            return *id;
        }

        let id = self.nodes.len() as u32;
        self.ids.insert(node_idx, id);
        self.nodes.push(HypernoteNode {
            id,
            node_type: HypernoteNodeType::Unsupported,
            child_ids: vec![],
            value: None,
            level: None,
            url: None,
            lang: None,
            name: None,
            raw_type_name: None,
            checked: None,
            attributes: vec![],
        });
        let node = self.build_node(node_idx, id);
        self.nodes[id as usize] = node;
        id
    }

    fn build_node(&mut self, node_idx: NodeIndex, id: u32) -> HypernoteNode {
        let node = self.ast.nodes[node_idx as usize];
        let node_type = map_node_type(node.tag);
        let is_unsupported = matches!(node_type, HypernoteNodeType::Unsupported);
        let mut child_ids = None;
        let mut value = None;
        let mut level = None;
        let mut url = None;
        let mut lang = None;
        let mut name = None;
        let mut checked = None;
        let mut attributes = Vec::new();

        match node.tag {
            NodeTag::Heading => {
                level = Some(self.ast.heading_info(node_idx).level);
            }
            NodeTag::Text => {
                value = Some(self.ast.token_slice(node.main_token).to_string());
            }
            NodeTag::CodeInline => {
                if let NodeData::Token(content_token) = node.data {
                    value = Some(self.ast.token_slice(content_token).to_string());
                }
            }
            NodeTag::CodeBlock => {
                if let Some(info) = self.ast.code_block_info(node_idx) {
                    lang = info.lang.map(str::to_string);
                    value = Some(info.code.to_string());
                }
            }
            NodeTag::Link => {
                if let Some(info) = self.ast.link_view(node_idx) {
                    child_ids = Some(
                        info.label_children
                            .iter()
                            .copied()
                            .map(|child| self.convert_node(child))
                            .collect(),
                    );
                    url = Some(info.url.to_string());
                }
            }
            NodeTag::Image => {
                if let Some(info) = self.ast.image_view(node_idx) {
                    child_ids = Some(
                        info.alt_children
                            .iter()
                            .copied()
                            .map(|child| self.convert_node(child))
                            .collect(),
                    );
                    url = Some(info.url.to_string());
                }
            }
            NodeTag::MdxJsxElement | NodeTag::MdxJsxSelfClosing => {
                if let Some(info) = self.ast.jsx_element_view(node_idx) {
                    child_ids = Some(
                        info.children
                            .iter()
                            .copied()
                            .map(|child| self.convert_node(child))
                            .collect(),
                    );
                    name = Some(info.name.to_string());
                    attributes = info.attrs.into_iter().map(convert_attribute).collect();
                }
            }
            NodeTag::ListItem => {
                checked = self.ast.list_item_info(node_idx).checked;
            }
            NodeTag::Frontmatter => {
                value = self
                    .ast
                    .frontmatter_view(node_idx)
                    .map(|info| info.value.to_string());
            }
            NodeTag::MdxTextExpression | NodeTag::MdxFlowExpression => {
                value = self
                    .ast
                    .expression_info(node_idx)
                    .map(|info| info.value.to_string());
            }
            NodeTag::Hr
            | NodeTag::HardBreak
            | NodeTag::Paragraph
            | NodeTag::Blockquote
            | NodeTag::ListUnordered
            | NodeTag::ListOrdered
            | NodeTag::Strong
            | NodeTag::Emphasis
            | NodeTag::Strikethrough
            | NodeTag::Table
            | NodeTag::TableRow
            | NodeTag::TableCell
            | NodeTag::MdxJsxFragment
            | NodeTag::MdxJsxAttribute
            | NodeTag::MdxEsmImport
            | NodeTag::MdxEsmExport
            | NodeTag::Document => {}
        }

        let child_ids = child_ids.unwrap_or_else(|| self.child_ids(node_idx));

        HypernoteNode {
            id,
            node_type,
            child_ids,
            value,
            level,
            url,
            lang,
            name,
            raw_type_name: is_unsupported.then(|| node.tag.name().to_string()),
            checked,
            attributes,
        }
    }

    fn child_ids(&mut self, node_idx: NodeIndex) -> Vec<u32> {
        self.ast
            .children(node_idx)
            .iter()
            .copied()
            .map(|child| self.convert_node(child))
            .collect()
    }
}

fn map_node_type(tag: NodeTag) -> HypernoteNodeType {
    match tag {
        NodeTag::Heading => HypernoteNodeType::Heading,
        NodeTag::Paragraph => HypernoteNodeType::Paragraph,
        NodeTag::Strong => HypernoteNodeType::Strong,
        NodeTag::Emphasis => HypernoteNodeType::Emphasis,
        NodeTag::CodeInline => HypernoteNodeType::CodeInline,
        NodeTag::CodeBlock => HypernoteNodeType::CodeBlock,
        NodeTag::Link => HypernoteNodeType::Link,
        NodeTag::Image => HypernoteNodeType::Image,
        NodeTag::ListUnordered => HypernoteNodeType::ListUnordered,
        NodeTag::ListOrdered => HypernoteNodeType::ListOrdered,
        NodeTag::ListItem => HypernoteNodeType::ListItem,
        NodeTag::Blockquote => HypernoteNodeType::Blockquote,
        NodeTag::Hr => HypernoteNodeType::Hr,
        NodeTag::HardBreak => HypernoteNodeType::HardBreak,
        NodeTag::Text => HypernoteNodeType::Text,
        NodeTag::MdxJsxElement => HypernoteNodeType::MdxJsxElement,
        NodeTag::MdxJsxSelfClosing => HypernoteNodeType::MdxJsxSelfClosing,
        NodeTag::Document
        | NodeTag::Frontmatter
        | NodeTag::MdxTextExpression
        | NodeTag::MdxFlowExpression
        | NodeTag::MdxJsxFragment
        | NodeTag::MdxJsxAttribute
        | NodeTag::MdxEsmImport
        | NodeTag::MdxEsmExport
        | NodeTag::Strikethrough
        | NodeTag::Table
        | NodeTag::TableRow
        | NodeTag::TableCell => HypernoteNodeType::Unsupported,
    }
}

fn convert_attribute(attr: hypernote_mdx::semantic::JsxAttributeView<'_>) -> HypernoteAttribute {
    let value = match attr.value {
        MdxJsxAttributeValue::String(value) => HypernoteAttributeValue::String(value),
        MdxJsxAttributeValue::Number(value) => HypernoteAttributeValue::Number(value),
        MdxJsxAttributeValue::InvalidNumber(value) => {
            HypernoteAttributeValue::InvalidNumber(value.to_string())
        }
        MdxJsxAttributeValue::Boolean(value) => HypernoteAttributeValue::Boolean(value),
        MdxJsxAttributeValue::Expression(value) => {
            HypernoteAttributeValue::Expression(value.to_string())
        }
    };

    HypernoteAttribute {
        name: attr.name.to_string(),
        value,
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
    fn parse_hypernote_builds_document_and_actions() {
        let parsed = parse_hypernote(
            "# Poll\n\n<SubmitButton action=\"yes\">Yes</SubmitButton>\n<SubmitButton action=\"no\" />",
        );

        assert!(!parsed.ast_json.is_empty());
        assert_eq!(parsed.declared_actions, vec!["yes", "no"]);

        let submit = find_node(
            &parsed.document,
            HypernoteNodeType::MdxJsxElement,
            Some("SubmitButton"),
        );
        assert_eq!(
            submit.attributes,
            vec![HypernoteAttribute {
                name: "action".to_string(),
                value: HypernoteAttributeValue::String("yes".to_string()),
            }]
        );
    }

    #[test]
    fn parse_hypernote_preserves_markdown_fields_and_rich_link_labels() {
        let parsed = parse_hypernote(
            "# Title\n\nParagraph with [**bold** label](https://example.com) and `code`.\n\n```rust\nfn main() {}\n```",
        );

        let heading = find_node(&parsed.document, HypernoteNodeType::Heading, None);
        assert_eq!(heading.level, Some(1));

        let link = find_node(&parsed.document, HypernoteNodeType::Link, None);
        assert_eq!(link.url.as_deref(), Some("https://example.com"));
        assert_eq!(link.child_ids.len(), 2);

        let code_block = find_node(&parsed.document, HypernoteNodeType::CodeBlock, None);
        assert_eq!(code_block.lang.as_deref(), Some("rust"));
        assert_eq!(code_block.value.as_deref(), Some("fn main() {}\n"));
    }

    #[test]
    fn parse_hypernote_uses_typed_attribute_values() {
        let parsed = parse_hypernote(
            "<Widget label=\"Fish &amp; Chips\" count=4 enabled visible=false expr={state.count} />",
        );

        let widget = find_node(
            &parsed.document,
            HypernoteNodeType::MdxJsxSelfClosing,
            Some("Widget"),
        );
        assert_eq!(
            widget.attributes,
            vec![
                HypernoteAttribute {
                    name: "label".to_string(),
                    value: HypernoteAttributeValue::String("Fish & Chips".to_string()),
                },
                HypernoteAttribute {
                    name: "count".to_string(),
                    value: HypernoteAttributeValue::Number(4.0),
                },
                HypernoteAttribute {
                    name: "enabled".to_string(),
                    value: HypernoteAttributeValue::Boolean(true),
                },
                HypernoteAttribute {
                    name: "visible".to_string(),
                    value: HypernoteAttributeValue::Boolean(false),
                },
                HypernoteAttribute {
                    name: "expr".to_string(),
                    value: HypernoteAttributeValue::Expression("state.count".to_string()),
                },
            ]
        );
    }

    #[test]
    fn extract_submit_actions_ignores_duplicates_and_blanks() {
        let document = HypernoteDocument {
            root_node_ids: vec![0, 1, 2],
            nodes: vec![
                HypernoteNode {
                    id: 0,
                    node_type: HypernoteNodeType::MdxJsxElement,
                    child_ids: vec![],
                    value: None,
                    level: None,
                    url: None,
                    lang: None,
                    name: Some("SubmitButton".to_string()),
                    raw_type_name: None,
                    checked: None,
                    attributes: vec![HypernoteAttribute {
                        name: "action".to_string(),
                        value: HypernoteAttributeValue::String("yes".to_string()),
                    }],
                },
                HypernoteNode {
                    id: 1,
                    node_type: HypernoteNodeType::MdxJsxSelfClosing,
                    child_ids: vec![],
                    value: None,
                    level: None,
                    url: None,
                    lang: None,
                    name: Some("SubmitButton".to_string()),
                    raw_type_name: None,
                    checked: None,
                    attributes: vec![HypernoteAttribute {
                        name: "action".to_string(),
                        value: HypernoteAttributeValue::String(" yes ".to_string()),
                    }],
                },
                HypernoteNode {
                    id: 2,
                    node_type: HypernoteNodeType::MdxJsxElement,
                    child_ids: vec![],
                    value: None,
                    level: None,
                    url: None,
                    lang: None,
                    name: Some("SubmitButton".to_string()),
                    raw_type_name: None,
                    checked: None,
                    attributes: vec![HypernoteAttribute {
                        name: "action".to_string(),
                        value: HypernoteAttributeValue::String(String::new()),
                    }],
                },
            ],
        };

        assert_eq!(extract_submit_actions(&document), vec!["yes"]);
    }

    #[test]
    fn parse_hypernote_preserves_unsupported_node_metadata() {
        let parsed = parse_hypernote("~~gone~~");
        let unsupported = parsed
            .document
            .nodes
            .iter()
            .find(|node| node.node_type == HypernoteNodeType::Unsupported)
            .expect("unsupported node exists");

        assert_eq!(unsupported.raw_type_name.as_deref(), Some("strikethrough"));
        assert!(!unsupported.child_ids.is_empty());
    }
}
